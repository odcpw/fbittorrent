"""Real kernel-assigned peer listeners, no probe-close-rebind reservation."""
from contextlib import ExitStack
import json
from pathlib import Path
import socket
import tomllib
import daemon as d
from fixture_ports import listener_port


def fixture_kernel_peer_ports(binary,systemd):
    with d.Case('daemon','fixture_kernel_peer_ports') as case,ExitStack() as stack:
        occupied=stack.enter_context(socket.socket());occupied.bind(('127.0.0.1',0));occupied.listen()
        busy=occupied.getsockname()[1];ports=[];owners=[]
        for index in range(3):
            box=stack.enter_context(d.Sandbox());h=stack.enter_context(d.Harness(case,binary,box,systemd))
            cfg=Path(box.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml'
            case.check(tomllib.loads(cfg.read_text())['peer_port']==0,'fixture leaves initial port allocation to the daemon kernel bind')
            p=h.start_serve(idle_exit_secs=0);assert h.receive_ready() and h.wait_engine()
            port=tomllib.loads(cfg.read_text())['peer_port'];actual=listener_port(h.daemon_pid())
            with socket.create_connection(('127.0.0.1',port),timeout=2) as peer:
                case.check(0<port==actual and port!=busy and peer.getpeername()[1]==port,
                           'published fixture port is this real daemon listening socket',json.dumps(dict(config_port=port,actual_port=actual,occupied=busy)))
            ports.append(port);owners.append((h,p))
        case.check(len(set(ports))==3,'concurrent real fixture daemons own distinct kernel-assigned peer ports',json.dumps(ports))
        for h,p in owners:h.stop_serve(p);h.lock_free()

CASES=(fixture_kernel_peer_ports,)
