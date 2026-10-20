#!/usr/bin/env python3
"""The real shell retries a full daemon listener without starting another owner."""
import argparse
import json
import select
import os
import signal
from pathlib import Path
import sys
from contextlib import ExitStack

from dashboard import Session
import daemon as d
import fixtures
import private_net
import fbtlog


def run(binary):
    with fbtlog.Case('qs_lifecycle', 'admission_recovery') as case, fixtures.Sandbox() as box, d.Harness(case, binary, box, False) as h:
        process = h.start_serve(idle_exit_secs=0)
        assert h.receive_ready() and h.wait_engine()
        holders = ExitStack()
        shell = None
        try:
            # Actual authenticated connections occupy every server
            # admission slot. No fake Socket or invented error is involved.
            clients = []
            rejection = None
            for _ in range(64):
                client = holders.enter_context(d.ApiConn(h, timeout=1))
                identifier = None
                try:
                    # A full listener speaks before reading hello. Consume
                    # that response first; writing into its closing socket
                    # would race a reset and obscure the actual busy frame.
                    if select.select([client.socket], [], [], .05)[0]:
                        frame = client.receive()
                    else:
                        identifier = client.send('hello', dict(proto=1, client=dict(
                            name='admission-holder', version='0.6.0', kind='test')))
                        frame = client.receive()
                    if frame and frame.get('ev') == 'error':
                        rejection = frame['error']
                        client.close()
                        break
                    assert frame and frame.get('id') == identifier and frame.get('ok'), frame
                except (OSError, EOFError):
                    client.close()
                    continue
                clients.append(client)
            assert case.check(clients and rejection and rejection.get('code') == 'busy'
                              and rejection.get('details', {}).get('limit') == 'connections',
                              'real authenticated clients fill the listener and the next connection is rejected as busy',
                              json.dumps(dict(held=len(clients), rejection=rejection)))
            original = clients[0].call('app.info')['result']
            shell = Session(case, box)
            shell.wait(lambda: json.loads(shell.ipc('connectionState')))
            shell.shell('summon', '{"view":"torrents"}')
            try:
                shell.wait(lambda: (s := json.loads(shell.ipc('connectionState')))['recovering'] and s['reconnecting'] and 'Reconnecting' in json.loads(shell.ipc('stoppedState'))['message'])
            except TimeoutError:
                case.check(False, 'real window explains transient recovery', shell.ipc('connectionState') + shell.ipc('stoppedState'))
                raise
            assert case.check(not shell.state()['online'],
                              'connection rejection leaves the shell recovering, not stopped', shell.ipc('connectionState'))
            holders.close()
            shell.wait(lambda: shell.state()['online'], timeout=20)
            assert case.check(shell.state()['instance'] == original['instance'],
                              'freeing capacity reconnects the existing shell to the same daemon', json.dumps(shell.state()))
            h.stop_serve(process)
            shell.wait(lambda: not shell.state()['online'])
            assert case.check(not json.loads(shell.ipc('connectionState'))['recovering'],
                              'explicit shutdown cancels recovery', shell.ipc('connectionState'))
            h.lock_free()
        finally:
            holders.close()
            if shell:
                shell.close()
    queued_hello(binary)
    return fbtlog.suite_status('qs_lifecycle')


def queued_hello(binary):
    with fbtlog.Case('qs_lifecycle', 'queued_hello') as case, fixtures.daemon_pair(case, binary, False, 65536) as pair:
        shell = Session(case, pair.box)
        stopped = False
        pid = pair.h.daemon_pid()
        try:
            shell.wait(lambda: shell.state()['online'])
            shell.shell('summon', '{"view":"torrents"}')
            shell.wait(lambda: pair.hash in shell.ipc('renderedRows'))
            before = pair.client.call('torrents.get', dict(hash=pair.hash))['result']['torrent']['progress']
            instance = shell.state()['instance']
            generation = json.loads(shell.ipc('connectionState'))['generation']
            os.kill(pid, signal.SIGSTOP)
            stopped = True
            assert shell.ipc('disconnectForRecovery') == 'true'
            # The actual daemon cannot answer. Require retirement of the new
            # unanswered hello, not only the deliberately closed old socket.
            shell.wait(lambda: json.loads(shell.ipc('connectionState'))['generation'] >= generation + 2, timeout=10)
            assert case.check(not shell.state()['online'], 'an unanswered native hello is retired within its five-second deadline', shell.ipc('connectionState'))
            os.kill(pid, signal.SIGCONT)
            stopped = False
            shell.wait(lambda: shell.state()['online'] and any(row['hash']==pair.hash and row['progress']>before for row in json.loads(shell.ipc('renderedRows'))), timeout=20)
            assert case.check(shell.state()['instance']==instance, 'the same real shell reconnects to the resumed owner and renders newer payload progress', shell.ipc('connectionState'))
        finally:
            if stopped:
                os.kill(pid, signal.SIGCONT)
            shell.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--log-root', type=Path, required=True)
    args = parser.parse_args()
    fbtlog.LOG_ROOT = args.log_root.resolve()
    private_net.enter()
    sys.exit(run(args.binary.resolve()))
