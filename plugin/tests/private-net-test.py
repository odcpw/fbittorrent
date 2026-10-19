#!/usr/bin/env python3
"""Real local DNS and fail-closed namespace boundary controls."""
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'agent_ergonomics_audit/audit/regression_tests'))
from fbtlog import Case,suite_status
import private_net
private_net.enter()
with Case('qs_private_net','boundary') as case:
    case.check(True,'network/mount namespaces are new, only lo exists, no external route',private_net.require())
    env=dict(os.environ,FBT_PRIVATE_PARENT_NET=os.readlink('/proc/self/ns/net'))
    child=subprocess.run([sys.executable,'-c','import private_net; private_net.require()'],cwd=Path(__file__).parent,env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    case.check(child.returncode!=0 and 'namespace was not created' in child.stderr,'forged active marker is rejected before any mount/interface mutation',child.stderr.strip())
with Case('qs_private_net','dns') as case:
    for qtype,family,address in [(1,socket.AF_INET,'127.0.0.1'),(28,socket.AF_INET6,'::1')]:
        query=struct.pack('!HHHHHH',1234,0x0100,1,0,0,0)+b'\x07fixture\x07invalid\x00'+struct.pack('!HH',qtype,1)
        with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as connection:
            connection.settimeout(1);connection.sendto(query,('192.0.2.53',53));reply,_=connection.recvfrom(512)
        case.check(reply[:2]==query[:2] and reply[6:8]==b'\x00\x01' and reply.endswith(socket.inet_pton(family,address)),'local DNS answers fixture name with '+address,reply.hex())
    with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as connection:
        connection.settimeout(.25);connection.sendto(b'x'*513,('192.0.2.53',53))
        try:connection.recvfrom(512);rejected=False
        except socket.timeout:rejected=True
        case.check(rejected,'oversized DNS datagram is discarded','513-byte datagram received no answer')
raise SystemExit(suite_status('qs_private_net'))
