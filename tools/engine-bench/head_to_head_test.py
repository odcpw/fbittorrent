"""Controls for cross-file verification and private process cleanup."""
import hashlib
import os
from pathlib import Path
import subprocess
import socket
import sys
import tempfile
from types import SimpleNamespace
import unittest

from head_to_head import Match, decode, verify_payload
from fixtures import Sandbox, bencode
from clients import Qbittorrent, owns_listener
from process_cleanup import ProcessRun
from run import wait


class ComparisonControls(unittest.TestCase):
    @unittest.skipUnless(os.environ.get('FBT_BENCH_SEED') and os.environ.get('FBT_BENCH_BINARY'),
                         'set explicit retained binaries for the real transfer control')
    def test_real_multifile_transfer_with_both_client_adapters(self):
        with tempfile.TemporaryDirectory() as directory:
            for client in ('fbt','qbt'):
                with self.subTest(client=client):
                    args=SimpleNamespace(output=Path(directory),disk_root=Path('/tmp'),strace=None,
                        public_torrent=None,profile=False,seed=Path(os.environ['FBT_BENCH_SEED']),
                        binary=Path(os.environ['FBT_BENCH_BINARY']),qbt=Path('/usr/bin/qbittorrent-nox'))
                    run=Match(args)
                    with run.setup_match(client,client,262144,3,4) as torrent:
                        run.client.add(torrent,run.info_hash)
                        wait(lambda:run.client.observation()['complete'],15)
                        verify_payload(run.sandbox.downloads,run.info)
                        self.assertEqual(run.stats()['torrents'][0]['uploaded_bytes'],0)
                    self.assertFalse(run.owners.survivors())
                    self.assertFalse((run.output/'result.json').exists(),'control must not become a timing trial')

    def test_listener_ownership_refuses_another_process(self):
        with socket.socket() as listener:
            listener.bind(('127.0.0.1',0));listener.listen()
            port=listener.getsockname()[1]
            child=subprocess.Popen([sys.executable,'-c','import time;time.sleep(60)'])
            try:
                self.assertTrue(owns_listener(os.getpid(),port))
                self.assertFalse(owns_listener(child.pid,port))
            finally:
                child.terminate();child.wait(3)

    @unittest.skipUnless(Path('/usr/bin/qbittorrent-nox').exists(),'qBittorrent not installed')
    def test_real_qbt_private_profile_preferences_and_shutdown(self):
        with tempfile.TemporaryDirectory() as output, Sandbox() as sandbox:
            run=Match(SimpleNamespace(strace=None))
            run.output=Path(output);run.sandbox=sandbox
            owner=ProcessRun();owner.register(sandbox.root)
            client=None
            try:
                client=Qbittorrent(run,Path('/usr/bin/qbittorrent-nox'))
                self.assertEqual(client.call('torrents/info'),[])
                self.assertEqual(Path(f'/proc/{client.pid}/cwd').resolve(),sandbox.root)
                self.assertTrue((sandbox.root/'qb-profile/qBittorrent/config/qBittorrent.conf').exists())
                client.stop()
                self.assertFalse(owner.survivors())
            finally:
                owner.cleanup()
                for child in run.children:child.wait(3)
                for stream in run.handles:stream.close()

    def test_cross_file_pieces_and_truncated_or_corrupt_payload(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'bundle').mkdir()
            payload=b'0123456789abcdefXYZ'
            sizes=[3,0,8,8];offset=0;files=[]
            for index,size in enumerate(sizes):
                (root/'bundle'/str(index)).write_bytes(payload[offset:offset+size]);offset+=size
                files.append({b'length':size,b'path':[str(index).encode()]})
            info={b'name':b'bundle',b'files':files,b'piece length':8,
                  b'pieces':b''.join(hashlib.sha1(payload[i:i+8]).digest() for i in range(0,len(payload),8))}
            self.assertEqual(decode(bencode(info)),info)
            self.assertEqual(verify_payload(root,info)[0],hashlib.sha256(payload).hexdigest())
            path=root/'bundle/2';original=path.read_bytes();path.write_bytes(b'x'+original[1:])
            with self.assertRaises(AssertionError):verify_payload(root,info)
            path.write_bytes(original[:-1])
            with self.assertRaises(AssertionError):verify_payload(root,info)

    def test_setup_failure_reaps_child_and_does_not_qualify(self):
        with tempfile.TemporaryDirectory() as output:
            class Fails(Match):
                def tool(self,*_args,**_kwargs):
                    self.spawn([sys.executable,'-c','import time;time.sleep(60)'])
                    raise RuntimeError('injected after private child launch')
            run=Fails(SimpleNamespace(output=Path(output),disk_root=Path('/tmp'),strace=None,public_torrent=None))
            with self.assertRaises(BaseExceptionGroup):
                with run.setup_match('fail','fbt',1024,1,1):pass
            self.assertFalse(run.owners.survivors())
            self.assertFalse(run.sandbox.root.exists())
            import json
            teardown=json.loads((run.output/'teardown.json').read_text())
            self.assertFalse(teardown['qualified'])
            self.assertTrue(teardown['emergency_cleanup'])
            self.assertEqual(teardown['survivors'],0)
            self.assertTrue(all(p.poll() is not None for p in run.children))


if __name__=='__main__':unittest.main()
