"""Fail-closed local network and DNS for real-daemon UI tests."""
import atexit
import errno
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import threading

MARKER='FBT_PRIVATE_NET_ACTIVE'
_dns=None


class LocalDns:
    """Answer A/AAAA with loopback, never forwarding a fixture query."""
    def __init__(self):
        self.socket=socket.socket(socket.AF_INET,socket.SOCK_DGRAM)
        self.socket.bind(('192.0.2.53',53));self.socket.settimeout(.2)
        self.stop=threading.Event()
        self.thread=threading.Thread(target=self.run,name='fbt-local-dns',daemon=True);self.thread.start()

    def run(self):
        while not self.stop.is_set():
            try:query,peer=self.socket.recvfrom(513)
            except socket.timeout:continue
            except OSError:break
            if not 12<=len(query)<=512 or query[4:6]!=b'\x00\x01':continue
            cursor=12
            while cursor<len(query) and query[cursor]:
                count=query[cursor]
                if count>63:break
                cursor+=count+1
            if cursor>=len(query) or query[cursor]!=0 or cursor+5>len(query):continue
            end=cursor+5;qtype=struct.unpack('!H',query[cursor+1:cursor+3])[0]
            value=socket.inet_pton(socket.AF_INET,'127.0.0.1') if qtype==1 else socket.inet_pton(socket.AF_INET6,'::1') if qtype==28 else None
            reply=query[:2]+struct.pack('!HHHHH',0x8180,1,1 if value else 0,0,0)+query[12:end]
            if value:reply+=b'\xc0\x0c'+struct.pack('!HHIH',qtype,1,60,len(value))+value
            try:self.socket.sendto(reply,peer)
            except OSError:break

    def close(self):
        self.stop.set();self.socket.close();self.thread.join(timeout=1)


def require():
    global _dns
    if os.environ.get(MARKER)!='1':raise RuntimeError('real UI fixture requires private_net.enter() before creating cases')
    for kind in ('net','mnt'):
        previous=os.environ.get('FBT_PRIVATE_PARENT_'+kind.upper())
        if not previous or os.readlink('/proc/self/ns/'+kind)==previous:raise RuntimeError('private UI '+kind+' namespace was not created')
    links=json.loads(subprocess.check_output(['ip','-json','link','show'],text=True))
    if {item['ifname'] for item in links}!={'lo'}:raise RuntimeError('private UI network must contain only loopback; refusing owner namespace')
    subprocess.run(['ip','link','set','lo','up'],check=True)
    for family,address in [(socket.AF_INET,('1.1.1.1',53)),(socket.AF_INET6,('2606:4700:4700::1111',53))]:
        with socket.socket(family,socket.SOCK_DGRAM) as probe:
            try:probe.connect(address)
            except OSError as error:
                if error.errno not in (errno.ENETUNREACH,errno.EHOSTUNREACH):raise
            else:raise RuntimeError('private namespace unexpectedly has an external route')
    if 'FBT_PRIVATE_DNS_PID' not in os.environ:
        subprocess.run(['ip','address','add','192.0.2.53/32','dev','lo'],check=True)
        # Both mounts are confined to the verified private mount namespace.
        with tempfile.TemporaryDirectory(prefix='fbt-net-',dir='/tmp') as root:
            resolver=Path(root)/'resolv.conf';resolver.write_text('nameserver 192.0.2.53\noptions attempts:1 timeout:1\n')
            hosts=Path(root)/'hosts';hosts.write_text('127.0.0.1 localhost\n::1 localhost\n')
            for source,target in [(resolver,'/etc/resolv.conf'),(hosts,'/etc/hosts')]:subprocess.run(['mount','--bind',str(source),target],check=True)
        _dns=LocalDns();atexit.register(_dns.close)
        os.environ['FBT_PRIVATE_DNS_PID']=str(os.getpid())
    else:
        pid=int(os.environ['FBT_PRIVATE_DNS_PID']);os.kill(pid,0)
        if os.readlink(f'/proc/{pid}/ns/net')!=os.readlink('/proc/self/ns/net'):raise RuntimeError('fixture DNS is in a different network namespace')
    return describe()


def describe():
    return json.dumps(dict(network=os.readlink('/proc/self/ns/net'),mount=os.readlink('/proc/self/ns/mnt'),interfaces=['lo'],dns='192.0.2.53 on private loopback; A/AAAA answers are loopback',external_routes=False))


def enter():
    if os.environ.get(MARKER)!='1':
        environment=dict(os.environ,**{MARKER:'1','FBT_PRIVATE_PARENT_NET':os.readlink('/proc/self/ns/net'),'FBT_PRIVATE_PARENT_MNT':os.readlink('/proc/self/ns/mnt')})
        environment.pop('FBT_PRIVATE_DNS_PID',None)
        command=['unshare','--user','--map-current-user','--net','--mount','--propagation','private','--keep-caps',sys.executable,*sys.argv]
        os.execvpe(command[0],command,environment)
    return require()
