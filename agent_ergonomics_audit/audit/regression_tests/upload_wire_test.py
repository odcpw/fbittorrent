"""Negative controls: a dead/slow/wrong peer cannot prove zero upload."""
from contextlib import contextmanager
from pathlib import Path
import socket
import struct
import tempfile
import threading
import time
import unittest
from upload_wire import incoming

HASH='ab'*20


def exact(peer,n):
    value=b''
    while len(value)<n:
        part=peer.recv(n-len(value))
        if not part:raise EOFError('control closed')
        value+=part
    return value


@contextmanager
def endpoint(mode):
    listener=socket.socket();listener.bind(('127.0.0.1',0));listener.listen();listener.settimeout(1)
    errors=[];requests=[]
    def serve():
        try:
            with listener.accept()[0] as peer:
                peer.settimeout(1);hello=exact(peer,68)
                if mode=='closed':return
                if mode=='handshake_slow':time.sleep(.15);return
                if mode=='wrong_hash':hello=hello[:28]+b'x'*20+hello[48:]
                peer.sendall(hello)
                if mode=='wrong_hash':return
                exact(peer,5) # interested
                peer.sendall(b'\0\0\0\2\5\x80') # real advertised piece 0
                request=exact(peer,17);requests.append(request)
                if request!=struct.pack('!IBIII',13,6,0,0,16384):raise ValueError('wrong request')
                if mode=='request_slow':time.sleep(.15)
                elif mode=='payload':peer.sendall(struct.pack('!IBII',16393,7,0,0)+b'z'*16384)
        except BaseException as error:errors.append(error)
    thread=threading.Thread(target=serve);thread.start()
    try:yield listener.getsockname(),requests
    finally:
        listener.close();thread.join(timeout=2)
        if thread.is_alive():raise TimeoutError('control thread leaked')
        if errors:raise errors[0]


class ProbeTests(unittest.TestCase):
    def setUp(self):
        self.temporary=tempfile.TemporaryDirectory(prefix='fbt-wire-',dir='/tmp');self.addCleanup(self.temporary.cleanup)
        self.payload=Path(self.temporary.name)/'payload';self.payload.write_bytes(b'z'*16384)

    def test_refused_connection_is_not_zero(self):
        with socket.socket() as reserved:
            reserved.bind(('127.0.0.1',0))
            with self.assertRaises(ConnectionRefusedError):incoming(reserved.getsockname(),HASH,self.payload,timeout=.05)

    def test_closed_wrong_or_slow_handshake_is_not_zero(self):
        for mode,error in [('closed',EOFError),('wrong_hash',ValueError),('handshake_slow',TimeoutError)]:
            with self.subTest(mode=mode),endpoint(mode) as (addr,_):
                with self.assertRaises(error):incoming(addr,HASH,self.payload,timeout=.05)

    def test_timeout_after_the_request_is_not_zero(self):
        with endpoint('request_slow') as (addr,requests):
            with self.assertRaises(TimeoutError):incoming(addr,HASH,self.payload,timeout=.05)
            self.assertEqual(len(requests),1)

    def test_zero_requires_a_complete_handshake_and_written_piece_request(self):
        with endpoint('denied') as (addr,requests):
            proof=incoming(addr,HASH,self.payload,timeout=.5)
            self.assertTrue(proof.handshake and proof.request_sent and proof.closed)
            self.assertEqual(proof.payload_bytes,0);self.assertEqual(len(requests),1)

    def test_positive_control_verifies_actual_payload_bytes(self):
        with endpoint('payload') as (addr,requests):
            proof=incoming(addr,HASH,self.payload,timeout=.5)
            self.assertEqual(proof.payload_bytes,16384);self.assertEqual(len(requests),1)


if __name__=='__main__':unittest.main()
