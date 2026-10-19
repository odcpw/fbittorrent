"""Bounded BitTorrent probes. Failed transport is never zero-upload evidence."""
from dataclasses import asdict, dataclass
import select
import socket
import struct
import threading
import time

PEER_ID=b'-FB0001-proof-peer01'
assert len(PEER_ID)==20


@dataclass
class Observation:
    direction: str
    handshake: bool
    verified_piece: int
    request_sent: bool
    payload_bytes: int
    closed: bool

    def describe(self):return asdict(self)


class Wire:
    def __init__(self, peer, info_hash, timeout=3):
        self.peer,self.hash,self.timeout=peer,bytes.fromhex(info_hash),timeout
        self.deadline=time.monotonic()+timeout;self.phase='handshake'

    def exact(self, size, eof=False):
        value=b''
        while len(value)<size:
            left=self.deadline-time.monotonic()
            if left<=0:raise TimeoutError('peer exchange deadline expired')
            self.peer.settimeout(left)
            try:part=self.peer.recv(size-len(value))
            except socket.timeout as error:raise TimeoutError(f'peer timed out during {self.phase} after {len(value)}/{size} bytes') from error
            if not part:
                if eof and not value:return None
                raise EOFError('peer closed before completing a protocol field')
            value+=part
        return value

    def handshake(self, outgoing):
        message=b'\x13BitTorrent protocol'+bytes(8)+self.hash+PEER_ID
        if not outgoing:self.peer.sendall(message)
        response=self.exact(68)
        if response[:20]!=message[:20] or response[28:48]!=self.hash:
            raise ValueError('peer handshake protocol/info hash does not match the torrent')
        if outgoing:self.peer.sendall(message)
        self.peer.sendall(struct.pack('!IB',1,2)) # interested; no pieces owned by this leecher

    def frame(self):
        prefix=self.exact(4,eof=True)
        if prefix is None:return None
        size=struct.unpack('!I',prefix)[0]
        if size>65536:raise ValueError('oversize wire frame')
        return self.exact(size) if size else b''

    def available(self):
        self.phase='verified-piece advertisement'
        while True:
            frame=self.frame()
            if frame is None:raise EOFError('peer closed before advertising a verified piece')
            if frame and frame[0]==5:
                for byte,mask in enumerate(frame[1:]):
                    for bit in range(8):
                        if mask&(128>>bit):return byte*8+bit
            if frame and frame[0]==4:
                if len(frame)!=5:raise ValueError('invalid have frame')
                return struct.unpack('!I',frame[1:])[0]

    def request(self, piece, direction, payload):
        self.phase='response to the written piece request'
        self.deadline=time.monotonic()+self.timeout
        self.peer.sendall(struct.pack('!IBIII',13,6,piece,0,16384))
        while True:
            frame=self.frame()
            if frame is None:return Observation(direction,True,piece,True,0,True)
            if frame and frame[0]==7:
                if len(frame)!=16393 or struct.unpack('!II',frame[1:9])!=(piece,0):
                    raise ValueError('response is not the requested 16 KiB piece block')
                with payload.open('rb') as source:
                    source.seek(piece*16384);expected=source.read(16384)
                if frame[9:]!=expected:raise ValueError('piece bytes do not match the real seed payload')
                return Observation(direction,True,piece,True,len(frame)-9,False)


def incoming(addr, info_hash, payload, timeout=3, piece=None):
    with socket.create_connection(addr,timeout=timeout) as peer:
        wire=Wire(peer,info_hash,timeout);wire.handshake(False)
        return wire.request(wire.available() if piece is None else piece,'incoming',payload)


class Outbound:
    """The daemon must connect to this advertised leecher; no reverse dial."""
    def __init__(self,info_hash=None,payload=None,piece=None,timeout=15):
        self.info_hash,self.payload,self.piece,self.timeout=info_hash,payload,piece,timeout
        self.listener=socket.socket();self.listener.bind(('127.0.0.1',0));self.listener.listen(1);self.listener.settimeout(.1)
        self.addr=self.listener.getsockname();self.ready=threading.Event();self.go=threading.Event();self.done=threading.Event()
        self.stop=threading.Event();self.result=None;self.error=None;self.peer=None
        self.configured=threading.Event()
        if info_hash is not None:self.configured.set()
        self.thread=threading.Thread(target=self.run,daemon=True);self.thread.start()

    def run(self):
        try:
            deadline=time.monotonic()+self.timeout
            while not self.stop.is_set():
                try:self.peer,_=self.listener.accept();break
                except socket.timeout:
                    if time.monotonic()>=deadline:raise TimeoutError('daemon did not dial the advertised leecher')
            if self.stop.is_set():return
            if not self.configured.wait(self.timeout):raise TimeoutError('outbound fixture configuration deadline')
            wire=Wire(self.peer,self.info_hash,self.timeout);wire.handshake(True)
            piece=self.piece;self.ready.set()
            if not self.go.wait(self.timeout):raise TimeoutError('request authorization deadline')
            wire.timeout=3;self.result=wire.request(piece,'outgoing',self.payload)
        except BaseException as error:self.error=error
        finally:self.ready.set();self.done.set()

    def configure(self,info_hash,payload,piece):
        self.info_hash,self.payload,self.piece=info_hash,payload,piece;self.configured.set()

    def connected(self):
        if not self.ready.wait(self.timeout+1):raise TimeoutError('outbound handshake did not complete')
        if self.error:raise self.error

    def request(self):
        self.go.set()
        if not self.done.wait(5):raise TimeoutError('outbound payload probe did not finish')
        if self.error:raise self.error
        if self.result is None:raise RuntimeError('outbound probe stopped without an observation')
        return self.result

    def __enter__(self):return self
    def __exit__(self,*_):
        self.stop.set();self.go.set();self.configured.set();self.listener.close()
        if self.peer:
            try:self.peer.shutdown(socket.SHUT_RDWR)
            except OSError:pass
            self.peer.close()
        self.thread.join(timeout=2)
        if self.thread.is_alive():raise TimeoutError('outbound probe thread did not stop')
