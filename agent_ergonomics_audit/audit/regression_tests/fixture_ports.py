"""Use the kernel's real :0 binding, then publish its owned listener to fixtures."""
import os
from pathlib import Path
import re
import time
import tomllib


def listener_port(pid):
    # Inspect only this explicitly owned daemon PID, including its network
    # namespace. A host-wide port inventory could identify another app's socket.
    root=Path('/proc')/str(pid)
    inodes=set()
    for fd in (root/'fd').iterdir():
        try:target=os.readlink(fd)
        except FileNotFoundError:continue
        if target.startswith('socket:['):inodes.add(target[8:-1])
    ports=set()
    for table in ('tcp','tcp6'):
        try:lines=(root/'net'/table).read_text().splitlines()[1:]
        except FileNotFoundError:continue
        for line in lines:
            fields=line.split()
            if fields[3]=='0A' and fields[9] in inodes:
                ports.add(int(fields[1].split(':')[1],16))
    if len(ports)>1:raise RuntimeError('owned daemon has ambiguous peer listeners: '+str(ports))
    return next(iter(ports),None)


def wait_listener(pid,timeout=10):
    deadline=time.monotonic()+timeout
    while time.monotonic()<deadline:
        if value:=listener_port(pid):return value
        time.sleep(.02)
    raise TimeoutError('owned daemon did not bind its peer listener')


def publish(box,pid):
    path=Path(box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml'
    contents=path.read_text()
    if tomllib.loads(contents)['peer_port']!=0:return
    port=listener_port(pid)
    if port is None:return # A parked engine owns no peer listener.
    updated=re.sub(r'^peer_port\s*=\s*0\s*$',f'peer_port = {port}',contents,flags=re.M)
    temporary=path.with_suffix('.fixture-port.tmp')
    with temporary.open('w') as output:output.write(updated)
    temporary.replace(path)
