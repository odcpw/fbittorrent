"""Private headless Sway for native layer-shell tests; never uses owner displays."""
import os
from pathlib import Path
import shutil
import subprocess
import time

ROOT=Path(__file__).resolve().parents[2]


def executable():
    configured=os.environ.get('FBT_TEST_SWAY')
    found=configured or shutil.which('sway')
    development=ROOT/'plugin/tests/target/headless-tools/usr/bin/sway'
    if not found and development.exists():found=str(development)
    if not found:raise RuntimeError('native popup tests need sway (or FBT_TEST_SWAY); no owner display fallback')
    return str(Path(found).resolve())


class Headless:
    def __init__(self,sandbox,case):
        self.case=case;self.env=dict(sandbox.env)
        for key in ('DISPLAY','WAYLAND_DISPLAY','SWAYSOCK','HYPRLAND_INSTANCE_SIGNATURE'):
            self.env.pop(key,None)
        self.env.update(WLR_BACKENDS='headless',WLR_RENDERER='pixman',WLR_HEADLESS_OUTPUTS='1',
                        WLR_LIBINPUT_NO_DEVICES='1',XDG_CURRENT_DESKTOP='sway',QT_QPA_PLATFORM='wayland',
                        DBUS_SESSION_BUS_ADDRESS='unix:path='+str(sandbox.root/'no-session-bus'),
                        DBUS_SYSTEM_BUS_ADDRESS='unix:path='+str(sandbox.root/'no-system-bus'))
        binary=executable();local_lib=Path(binary).parent.parent/'lib'
        if (local_lib/'libwlroots-0.20.so').exists():self.env['LD_LIBRARY_PATH']=str(local_lib)
        config=sandbox.root/'headless-sway.conf'
        config.write_text('xwayland disable\noutput * mode 1280x720\nseat seat0 fallback true\nfocus_follows_mouse no\n')
        self.log=case.path.with_suffix('.compositor.log');self.stream=self.log.open('w')
        self.child=subprocess.Popen([binary,'--config',str(config),'--unsupported-gpu'],env=self.env,
                                    stdout=self.stream,stderr=subprocess.STDOUT,start_new_session=True)
        try:
            deadline=time.monotonic()+8
            runtime=Path(sandbox.env['XDG_RUNTIME_DIR'])
            while time.monotonic()<deadline:
                sockets=[p for p in runtime.glob('wayland-*') if p.is_socket()]
                if sockets:
                    self.env['WAYLAND_DISPLAY']=str(sockets[0])
                    case.check(True,'private compositor uses headless software backend',f'pid={self.child.pid}; display={sockets[0]}; log={self.log}')
                    return
                if self.child.poll() is not None:raise RuntimeError(f'private compositor exited {self.child.returncode}; {self.log}')
                time.sleep(.03)
            raise TimeoutError('private compositor did not create a Wayland socket')
        except BaseException:self.close();raise

    def close(self):
        if self.child.poll() is None:
            self.child.terminate()
            try:self.child.wait(timeout=3)
            except subprocess.TimeoutExpired:self.child.kill();self.child.wait(timeout=3)
        self.stream.close()
        self.case.check(self.child.poll() is not None,'owned compositor is reaped',f'pid={self.child.pid}; exit={self.child.returncode}; log={self.log}')

    def ipc(self,command='',kind=0):
        """Talk only to this compositor's private, sandbox-owned IPC socket."""
        import json,socket,struct
        paths=list(Path(self.env['XDG_RUNTIME_DIR']).glob('sway-ipc.*.sock'))
        if len(paths)!=1:raise RuntimeError('expected exactly one private Sway IPC socket')
        payload=command.encode()
        with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as channel:
            channel.settimeout(3);channel.connect(str(paths[0]));channel.sendall(b'i3-ipc'+struct.pack('<II',len(payload),kind)+payload)
            def read(count):
                result=b''
                while len(result)<count:
                    chunk=channel.recv(count-len(result))
                    if not chunk:raise EOFError('private compositor IPC closed')
                    result+=chunk
                return result
            header=read(14)
            if header[:6]!=b'i3-ipc':raise RuntimeError('invalid private compositor IPC magic')
            length,_=struct.unpack('<II',header[6:])
            if length>1048576:raise RuntimeError('private compositor reply exceeds 1 MiB')
            reply=json.loads(read(length))
        if kind==0 and any(not item.get('success') for item in reply):raise RuntimeError(str(reply))
        return reply
