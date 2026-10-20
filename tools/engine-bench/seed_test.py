#!/usr/bin/env python3
"""Exercise the measurement peer against independent wire/hash observations."""
import hashlib
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import time
import urllib.request

seed = Path(sys.argv[1]).resolve()
rate = next((int(arg) for arg in sys.argv[2:] if not arg.startswith('--')), 0)
sys.path.insert(0, str(Path(__file__).resolve().parents[2]/"agent_ergonomics_audit/audit/regression_tests"))
from fixtures import Sandbox, bencode
from head_to_head import decode
from process_cleanup import ProcessRun


def exact(connection, size):
    data = b""
    while len(data) < size:
        part = connection.recv(size-len(data))
        if not part:
            raise EOFError()
        data += part
    return data


with Sandbox() as sandbox:
    owner = ProcessRun()
    owner.register(sandbox.root)
    directory = sandbox.root / "metadata"
    directory.mkdir()
    info = json.loads(subprocess.check_output([seed,"make","--out",directory/"one.torrent","--name","fixture.bin",
                       "--size","262144","--piece-len","262144","--key","42","--tracker","http://127.0.0.1:1/announce"],
                      env=sandbox.env, cwd=sandbox.root))
    if '--multi' in sys.argv:
        path=directory/'one.torrent'
        metadata=decode(path.read_bytes())
        del metadata[b'info'][b'length']
        metadata[b'info'][b'files']=[{b'length':length,b'path':[str(i).encode()]}
                                  for i,length in enumerate((17,0,262127))]
        path.write_bytes(bencode(metadata))
        info['info_hash']=hashlib.sha1(bencode(metadata[b'info'])).hexdigest()
    payload = sandbox.root/"expected.bin"
    subprocess.run([seed,"write","--file",payload,"--size","262144","--key","42"],check=True,env=sandbox.env,cwd=sandbox.root)
    ready = sandbox.root/"ready.json"
    child = subprocess.Popen([seed,"serve","--torrents",directory,"--peers","4","--tracker-port","0","--ready",ready,"--rate",str(rate)],
                             env=sandbox.env,cwd=sandbox.root,start_new_session=True)
    connections = []
    try:
        deadline=time.monotonic()+10
        while not ready.exists() and time.monotonic()<deadline:
            time.sleep(.01)
        binding=json.loads(ready.read_text())
        identities=[]
        for port in binding["peer_ports"]:
            connection=socket.create_connection(("127.0.0.1",port),timeout=5)
            connections.append(connection)
            hello=b"\x13BitTorrent protocol" + bytes(8) + bytes.fromhex(info["info_hash"]) + b"-PYTEST-123456789012"
            assert len(hello)==68
            connection.sendall(hello)
            reply=exact(connection,68)
            assert reply[28:48]==bytes.fromhex(info["info_hash"])
            identities.append(reply[48:])
        assert len(set(identities))==4, "seed identities must be distinct"
        connection=connections[0]
        got=bytearray()
        # Queue a complete piece at once. A throttled peer must deliver blocks
        # steadily even when the request buffer is full, not sleep for a burst.
        connection.sendall(b"".join(struct.pack("!IBIII",13,6,0,offset,16384)
                                   for offset in range(0,262144,16384)))
        for offset in range(0,262144,16384):
            while True:
                length=struct.unpack("!I",exact(connection,4))[0]
                message=exact(connection,length)
                if message[0]==7:
                    index,begin=struct.unpack("!II",message[1:9])
                    assert index==0 and begin==offset
                    got.extend(message[9:])
                    break
        expected=payload.read_bytes()
        assert got==expected and hashlib.sha1(got).digest()==hashlib.sha1(expected).digest()
        # Negative control: a client upload and unchoke must show up in stats.
        connection.sendall(struct.pack("!IB",1,1)+struct.pack("!IBII",12,7,0,0)+b"bad")
        deadline=time.monotonic()+5
        while time.monotonic()<deadline:
            with urllib.request.urlopen(f"http://127.0.0.1:{binding['tracker_port']}/stats",timeout=5) as response:
                stats=json.load(response)["torrents"][0]
            if stats["uploaded_bytes"]==3 and stats["unchokes"]==1:
                break
            time.sleep(.01)
        assert stats["uploaded_bytes"]==3 and stats["unchokes"]==1, stats
        print("PASS: distinct peers, independent payload/hash check, upload/unchoke detector negative control")
    finally:
        for connection in connections:
            connection.close()
        child.terminate()
        child.wait(5)
        survivors=owner.survivors()
        owner.cleanup()
        assert not survivors, survivors
