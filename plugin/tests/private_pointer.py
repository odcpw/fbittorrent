"""Build a tiny native virtual-pointer sender against the installed Wayland ABI."""
from pathlib import Path
import hashlib
import os
import shlex
import subprocess

ROOT=Path(__file__).resolve().parents[2]


def binary():
    source=Path(__file__).with_name('private-pointer.c')
    xml=source.parent/'protocols/wlr-virtual-pointer-unstable-v1.xml'
    digest=hashlib.sha256(source.read_bytes()+xml.read_bytes()).hexdigest()[:16]
    build=ROOT/'plugin/tests/target/private-pointer'/digest;build.mkdir(parents=True,exist_ok=True)
    output=build/'private-pointer'
    if not output.exists():
        subprocess.run(['wayland-scanner','client-header',str(xml),str(build/'virtual-pointer.h')],check=True)
        subprocess.run(['wayland-scanner','private-code',str(xml),str(build/'virtual-pointer.c')],check=True)
        flags=shlex.split(subprocess.check_output(['pkg-config','--cflags','--libs','wayland-client'],text=True))
        subprocess.run(['cc','-O2','-Wall','-Wextra','-Werror','-I'+str(build),str(source),str(build/'virtual-pointer.c'),'-o',str(output),*flags],check=True)
    return output


class Pointer:
    def __init__(self,session,x,y,button=272,clicks=0):
        display=session.env['WAYLAND_DISPLAY']
        self.child=subprocess.Popen([str(binary()),display,str(x),str(y),'1920','1080',str(button),str(clicks)],env=session.env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
        try:self.expect('ready')
        except BaseException:self.child.kill();self.child.wait();raise

    def expect(self,line):
        import select
        if not select.select([self.child.stdout],[],[],3)[0] or self.child.stdout.readline().strip()!=line:raise RuntimeError('native pointer handshake failed')

    def motion(self):self.child.stdin.write('m\n');self.child.stdin.flush()
    def click(self):self.child.stdin.write('b\n');self.child.stdin.flush();self.expect('clicked')
    def close(self):
        if self.child.poll() is None:
            try:self.child.stdin.write('q\n');self.child.stdin.flush();self.child.wait(timeout=3)
            except (BrokenPipeError,subprocess.TimeoutExpired):self.child.kill();self.child.wait()
        if self.child.returncode!=0:raise RuntimeError('native pointer exited '+str(self.child.returncode))
