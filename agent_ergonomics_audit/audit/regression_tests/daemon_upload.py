"""Owner upload policy: actual piece requests, default refusal and explicit permission."""
import json
import socket
import struct
import time
import tomllib
from pathlib import Path
import daemon as d
import fixtures


def payload_request(box, info_hash):
    port=tomllib.loads((Path(box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())['peer_port']
    received=0
    try:
        with socket.create_connection(('127.0.0.1',port),timeout=1) as peer:
            peer.settimeout(.5)
            peer.sendall(b'\x13BitTorrent protocol'+bytes(8)+bytes.fromhex(info_hash)+b'-FB0001-upload-test1')
            def exact(size):
                value=b''
                while len(value)<size:
                    part=peer.recv(size-len(value))
                    if not part:raise EOFError('peer closed')
                    value+=part
                return value
            exact(68)
            peer.sendall(struct.pack('!IB',1,2)+struct.pack('!IBIII',13,6,0,0,16384))
            deadline=time.monotonic()+2
            while time.monotonic()<deadline:
                size=struct.unpack('!I',exact(4))[0]
                if size>65536:raise ValueError('oversize wire frame')
                if not size:continue
                frame=exact(size)
                if frame[0]==7:
                    if size<9:raise ValueError('invalid piece frame')
                    received+=size-9
                    break
    except (EOFError,OSError):pass
    return received


def zero_upload(binary,systemd):
    with d.Case('daemon','zero_upload') as case,fixtures.daemon_pair(case,binary,systemd,65536) as pair:
        c=pair.client
        def row():return c.call('torrents.get',dict(hash=pair.hash))['result']['torrent']
        initial=row()
        off=payload_request(pair.box,pair.hash)
        case.check(initial['upload_enabled'] is False and 'upload_limit_bps' not in initial and off==0,'default downloading torrent serves zero piece payload without an implicit rate cap',json.dumps(dict(row=initial,payload_bytes=off)))
        reply=pair.h.cli('settings','--upload','on')
        case.check(reply.get('upload')=='on' and reply.get('restart') is False,'CLI enables global upload live without reopening the engine',json.dumps(reply))
        measurements=[]
        def positive():
            value=payload_request(pair.box,pair.hash);measurements.append(value)
            return value==16384,json.dumps(dict(payload_bytes=value,row=row()))
        assert pair.h.wait_for(positive,'global upload on serves a verified piece while downloading',timeout=8)
        c.call('torrents.set_seed',dict(hashes=[pair.hash],on=False))
        case.check(payload_request(pair.box,pair.hash)==0 and row()['upload_enabled'] is False,'explicit torrent off overrides global on',json.dumps(row()))
        pair.h.cli('settings','--upload','off')
        # A rate-only change must not grant permission; the source opts in separately.
        c.call('torrents.set_upload_limit',dict(hashes=[pair.hash],bps=None))
        case.check(payload_request(pair.box,pair.hash)==0,'unlimited rate still serves zero payload when permission is off',json.dumps(row()))
        d.DaemonContract.throttle(pair.seed_client,pair.hash,None)
        assert pair.h.wait_for(lambda:((value:=row())['state']=='done',json.dumps(value)),'default-off client finishes downloading normally',timeout=20)
        complete_off=payload_request(pair.box,pair.hash)
        case.check(complete_off==0 and row()['upload_enabled'] is False,'completed default-off torrent serves zero piece payload',json.dumps(dict(row=row(),payload_bytes=complete_off)))
        c.call('torrents.set_seed',dict(hashes=[pair.hash],on=True))
        assert pair.h.wait_for(lambda:((value:=row())['state']=='seeding',json.dumps(value)),'explicit torrent upload on admits the completed payload')
        complete_on=payload_request(pair.box,pair.hash)
        case.check(complete_on==16384 and row()['upload_enabled'] is True,'completed explicit upload on serves requested payload with global upload off',json.dumps(dict(row=row(),payload_bytes=complete_on,downloading_on_bytes=measurements[-1])))
        c.call('torrents.set_seed',dict(hashes=[pair.hash],on=False))
        case.check(payload_request(pair.box,pair.hash)==0,'turning completed torrent upload off stops payload again',json.dumps(row()))
