#!/usr/bin/env python3
"""Contract 1.2 CLI qualification against owned foreground/systemd daemons."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

from fbtlog import Case, suite_status
from daemon import ApiConn, Harness, Sandbox, select_replay_interface
import fixtures


def cli(case, sandbox, binary, *args, code=0, valid=True, extra=None):
    argv=[str(binary),*map(str,args)]
    environment=dict(sandbox.env,**(extra or {}))
    result=subprocess.run(argv,env=environment,capture_output=True,text=True,timeout=20)
    try:
        value=json.loads(result.stdout) if valid else None
    except ValueError:
        value=None
    case.check(result.returncode==code and (not valid or isinstance(value,dict)),
               f"argv={argv!r} exits {code}; stdout is one JSON object" if valid else f"argv={argv!r} exits {code}",
               json.dumps(dict(argv=argv,code=result.returncode,stdout=result.stdout,stderr=result.stderr),sort_keys=True))
    return value or {},result


def reads_never_start(binary,systemd):
    with Case('agent_contract','reads_never_start') as case,Sandbox() as sandbox:
        for flag in ([],['--no-start']):
            for verb in ('list','triage','status'):
                value,_=cli(case,sandbox,binary,verb,'--json',*flag)
                case.check(value.get('running') is False,'closed reads truthfully report running:false',json.dumps(value))
            value,_=cli(case,sandbox,binary,'show','zzzz','--json',*flag,code=6)
        value,result=cli(case,sandbox,binary,'status','--quiet',code=4,valid=False)
        case.check(not result.stdout,'status --quiet emits nothing while stopped',repr(result.stdout))
        cli(case,sandbox,binary,'stop','all','--json')
        cli(case,sandbox,binary,'--no-start','add','magnet:?xt=urn:btih:'+'ab'*20,'--json',code=4)
        case.check(not (Path(sandbox.env['XDG_RUNTIME_DIR'])/'fbittorrent/api.sock').exists(),'read commands and empty offline stop did not create an owner','api.sock absent')
        log=sandbox.root/'shims/systemctl.log'
        case.check(not log.exists() or 'start' not in log.read_text(),'read commands never request a manager start',log.read_text() if log.exists() else 'no manager calls')


def contract(binary,systemd):
    with Case('agent_contract','contract') as case,Sandbox() as sandbox,Harness(case,binary,sandbox,systemd) as h:
        process=h.start_serve()
        try:
            h.receive_ready();h.wait_engine()
            fixture=fixtures.two_file_fixture(sandbox.root)
            bare,_=cli(case,sandbox,binary)
            case.check('backend' in bare and 'next_commands' in bare,'R-002 bare non-TTY invocation is triage JSON',json.dumps(bare))
            for name in ('robot-docs','docs'):
                _,r=cli(case,sandbox,binary,name,valid=False)
                case.check('DOWNLOAD SOMETHING' in r.stdout and 'EXIT CODES' in r.stdout,'R-003 handbook and alias keep their sections',r.stdout)
            missing,_=cli(case,sandbox,binary,'stop','zzzz9999','--json',code=6)
            case.check(missing.get('code')==6 and missing.get('hint'),'R-004 unknown selector keeps the JSON error and hint',json.dumps(missing))
            _,typo=cli(case,sandbox,binary,'list','--jsno',code=2,valid=False)
            case.check('--json' in typo.stderr,'R-006 flag typo suggests --json',typo.stderr)
            _,helped=cli(case,sandbox,binary,'--help',valid=False)
            case.check('--json' in helped.stdout and 'Exit codes' in helped.stdout and 'robot-docs' in helped.stdout,'R-009 help keeps agent pointers',helped.stdout)
            for alias in ('ls','rm','pause','resume','find','info'):
                cli(case,sandbox,binary,alias,'--help',valid=False)
            empty,_=cli(case,sandbox,binary,'stop','--json',code=2)
            case.check('list --json' in empty.get('hint',''),'R-011 empty selector has a copy-pasteable hint',json.dumps(empty))
            value,r1=cli(case,sandbox,binary,'capabilities','--json')
            _,r2=cli(case,sandbox,binary,'capabilities','--json')
            case.check(r1.stdout==r2.stdout,'R-008 capabilities is byte-identical',hashlib.sha256(r1.stdout.encode()).hexdigest())
            case.check(value.get('contract_version')=='1.2.6' and any(v.get('field')=='status.ok' and v.get('breaking') for v in value['changes']),'R-001 capabilities documents the 1.2.6 patch and retains the declared 1.2.0 break',json.dumps(value.get('changes')))
            cli(case,sandbox,binary,'triage','--json') # R-002
            added,_=cli(case,sandbox,binary,'add',fixture.torrent,'--stopped','--origin','agent','--json') # R-003
            digest=added['hash']
            case.check(digest==fixture.info_hash and added['duplicate'] is False,'add uses the API hash with origin attribution',json.dumps(added))
            duplicate,_=cli(case,sandbox,binary,'add',fixture.torrent,'--stopped','--json')
            case.check(duplicate['hash']==digest and duplicate['duplicate'],'duplicate add preserves identity',json.dumps(duplicate))
            cli(case,sandbox,binary,'show','zzzz','--json',code=6) # R-004
            cli(case,sandbox,binary,'show','file','--json')
            cli(case,sandbox,binary,'delete',digest,'--files','--json',code=3) # R-005
            cli(case,sandbox,binary,'delete',digest,'--files','--dry-run','--json') # R-006
            waited,_=cli(case,sandbox,binary,'wait',digest,'--timeout','1','--json',code=8)
            case.check(waited.get('code_name')=='timeout','R-012 wait retains the timeout envelope',json.dumps(waited)) # R-007
            cli(case,sandbox,binary,'start',digest,'--json')
            cli(case,sandbox,binary,'stop',digest,'--json')
            cli(case,sandbox,binary,'noseed',digest,'--json') # R-008
            cli(case,sandbox,binary,'seq',digest,'--json',code=2)
            cli(case,sandbox,binary,'list','--json') # R-009
            cli(case,sandbox,binary,'no-such-verb','--json',code=2) # R-010
            status,_=cli(case,sandbox,binary,'status','--json') # R-011
            case.check(status.get('running') and status.get('connection')=='api' and status.get('backend')=='native' and not status.get('app_open'),'running status has the additive daemon shape and no invented UI presence',json.dumps(status))
            cli(case,sandbox,binary,'status','--quiet',valid=False)
            cli(case,sandbox,binary,'robot-docs','--json') # R-012
            cli(case,sandbox,binary,'show',digest,'--json')
            with ApiConn(h) as client:
                client.hello()
                row=client.call('torrents.get',dict(hash=digest))['result']['torrent']
                case.check(row['origin']=='agent','origin is retained through a duplicate add',json.dumps(row))
            removed,_=cli(case,sandbox,binary,'delete',digest,'--files','--yes','--json')
            case.check(removed.get('count')==1,'R-012 add/show/wait/delete round trip removes the selected torrent',json.dumps(removed))
        finally:
            if process.child.poll() is None:h.stop_serve(process)
            h.lock_free()


def offline_records(binary,systemd):
    with Case('agent_contract','offline_records') as case,Sandbox() as sandbox,Harness(case,binary,sandbox,systemd) as h:
        select_replay_interface(Path(sandbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml','fbmissing0')
        process=h.start_serve()
        try:
            h.receive_ready();h.wait_engine()
            fixture=fixtures.two_file_fixture(sandbox.root)
            added,_=cli(case,sandbox,binary,'add',fixture.torrent,'--origin','agent','--json')
            case.check(added.get('deferred') is True,'parked CLI reports deferred:true from the API',json.dumps(added))
            h.stop_serve(process)
            value,_=cli(case,sandbox,binary,'list','--json')
            case.check(value['running'] is False and value['torrents'][0]['hash']==fixture.info_hash,'offline list retains the saved record',json.dumps(value))
            cli(case,sandbox,binary,'stop',fixture.info_hash,'--json')
            ledger=Path(sandbox.env['XDG_DATA_HOME'])/'fbittorrent/engine/torrents.json'
            case.check(json.loads(ledger.read_text())['records'][0]['paused'],'offline stop durably saves pause',ledger.read_text())
            cli(case,sandbox,binary,'delete',fixture.info_hash,'--json')
            case.check(not json.loads(ledger.read_text())['records'] and not (ledger.parent/(fixture.info_hash+'.torrent')).exists(),'offline keep-files deletion removes ledger/cache and keeps payload',ledger.read_text())
            case.check(all(path.exists() for path in fixture.files),'download payload is retained',str(fixture.files))
        finally:
            if process.child.poll() is None:h.stop_serve(process)


def json_determinism(binary,systemd):
    with Case('agent_contract','json_determinism') as case,Sandbox() as sandbox,Harness(case,binary,sandbox,systemd) as h:
        process=h.start_serve()
        try:
            h.receive_ready();h.wait_engine()
            fixture=fixtures.two_file_fixture(sandbox.root)
            added,_=cli(case,sandbox,binary,'add',fixture.torrent,'--stopped','--json')
            with ApiConn(h) as client:
                client.hello()
                h.wait_for(lambda: ((row:=client.call('torrents.get',dict(hash=added['hash']))['result']['torrent'])['checked'],json.dumps(row)),'fixture hash check finishes before idle determinism sampling')
            for args in [('status',),('list',),('show',added['hash']),('triage',),('config',),('capabilities',),('robot-docs',)]:
                outputs=[]
                for _ in range(2):
                    _,r=cli(case,sandbox,binary,*args,'--json',extra={'SOURCE_DATE_EPOCH':'1792382400','NO_COLOR':'1','CI':'1'})
                    outputs.append(r.stdout)
                case.check(outputs[0]==outputs[1],f'{args} is byte-identical with SOURCE_DATE_EPOCH',json.dumps(dict(hashes=[hashlib.sha256(s.encode()).hexdigest() for s in outputs],diff_bytes=sum(a!=b for a,b in zip(outputs[0],outputs[1]))+abs(len(outputs[0])-len(outputs[1])))))
        finally:
            if process.child.poll() is None:h.stop_serve(process)


def version_skew_warning(binary,systemd):
    with Case('agent_contract','version_skew_warning') as case,Sandbox() as sandbox,Harness(case,binary,sandbox,systemd) as h:
        sandbox.env['FBT_BUILD_ID_OVERRIDE']='fixture-daemon';sandbox.keys.append('FBT_BUILD_ID_OVERRIDE')
        process=h.start_serve()
        try:
            h.receive_ready();h.wait_engine()
            value,r=cli(case,sandbox,binary,'list','--json',extra={'FBT_BUILD_ID_OVERRIDE':'fixture-client'})
            case.check(any(w['code']=='version_skew' and 'fixture-daemon' in w['message'] and 'fixture-client' in w['message'] for w in value.get('warnings',[])) and not r.stderr,'different builds produce a typed JSON warning on data stdout only',json.dumps(value))
            _,r=cli(case,sandbox,binary,'list',valid=False,extra={'FBT_BUILD_ID_OVERRIDE':'fixture-client'})
            case.check(len(r.stderr.splitlines())==1 and 'fbittorrent restart' in r.stderr,'human mode prints exactly one actionable skew line',r.stderr)
        finally:
            if process.child.poll() is None:h.stop_serve(process)


def deprecation_warning_legacy_mode(binary,systemd):
    with Case('agent_contract','deprecation_warning_legacy_mode') as case,Sandbox() as sandbox:
        config=Path(sandbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml'
        config.write_text(config.read_text().replace('engine = "native"','engine = "qbittorrent"')+'\nurl="http://127.0.0.1:1"\n')
        value,r=cli(case,sandbox,binary,'status','--json')
        case.check(len(r.stderr.splitlines())==1 and 'deprecated' in r.stderr and any(w['code']=='deprecated_engine' for w in value.get('warnings',[])),'legacy mode has one stderr warning and a JSON warnings entry',json.dumps(dict(value=value,stderr=r.stderr)))

def exit_code_dictionary_total(binary,systemd):
    with Case('agent_contract','exit_code_dictionary_total') as case,Sandbox() as sandbox,Harness(case,binary,sandbox,systemd) as h:
        cli(case,sandbox,binary,'--bad-flag','--json',code=2)
        cli(case,sandbox,binary,'--no-start','add','magnet:?xt=urn:btih:'+'cd'*20,'--json',code=4)
        ratings=Path(sandbox.env['XDG_CACHE_HOME'])/'fbittorrent/title.ratings.tsv'
        ratings.unlink(missing_ok=True)
        cli(case,sandbox,binary,'imdb','Fixture','--refresh','--json',code=5,extra={'FBT_IMDB_BASE_URL':'http://127.0.0.1:1'})
        process=h.start_serve()
        try:
            h.receive_ready();h.wait_engine()
            payload=sandbox.downloads/'Completed.bin';payload.write_bytes(os.urandom(4096))
            fixture=fixtures.torrent_fixture(sandbox.root,[payload])
            added,_=cli(case,sandbox,binary,'add',fixture.torrent,'--json')
            with ApiConn(h) as client:
                client.hello()
                h.wait_for(lambda:((row:=client.call('torrents.get',dict(hash=added['hash']))['result']['torrent'])['state']=='done',json.dumps(row)),'completed fixture reaches done before explicit Resume refusal')
            refusal,_=cli(case,sandbox,binary,'start',added['hash'],'--json',code=1)
            case.check(refusal.get('hint')=='fbittorrent seed '+fixture.info_hash,'completion refusal carries the exact seed command',json.dumps(refusal))
            cli(case,sandbox,binary,'delete',fixture.info_hash,'--files','--json',code=3)
            cli(case,sandbox,binary,'show','zzzz','--json',code=6)
            for name in ('ambiguous-one','ambiguous-two'):
                directory=sandbox.root/name;directory.mkdir();item=fixtures.two_file_fixture(directory)
                cli(case,sandbox,binary,'add',item.torrent,'--stopped','--json')
            cli(case,sandbox,binary,'show','two-files','--json',code=7)
            cli(case,sandbox,binary,'wait','two-files','--timeout','1','--json',code=7)
            # A stopped incomplete record reaches the public wait timeout path.
            row,_=cli(case,sandbox,binary,'list','--json')
            incomplete=next(r['hash'] for r in row['torrents'] if r['hash']!=fixture.info_hash)
            cli(case,sandbox,binary,'wait',incomplete,'--timeout','1','--json',code=8)
            import socket
            sockets=[]
            try:
                time.sleep(.1) # allow prior short-lived CLI connections to retire
                for _ in range(32):
                    connection=socket.socket(socket.AF_UNIX);connection.connect(str(Path(sandbox.env['XDG_RUNTIME_DIR'])/'fbittorrent/api.sock'));sockets.append(connection)
                time.sleep(.1)
                busy,_=cli(case,sandbox,binary,'list','--json',code=4)
                case.check(busy.get('code_name')=='backend' and busy.get('error')=='busy','33rd connection maps typed busy to exit 4',json.dumps(busy))
            finally:
                for connection in sockets:connection.close()
        finally:
            if process.child.poll() is None:h.stop_serve(process)


def reannounce_verb(binary,systemd):
    from daemon import tracker_live_fixture,tracker_rows
    with Case('agent_contract','reannounce_verb') as case,Sandbox() as sandbox,Harness(case,binary,sandbox,systemd) as h,fixtures.tracker_server(fail_first=1,interval_s=600,peers=[('127.0.0.1',9)],directory=sandbox.root) as tracker:
        process=h.start_serve()
        try:
            h.receive_ready();h.wait_engine()
            fixture=tracker_live_fixture(sandbox,tracker.url)
            with ApiConn(h) as client:
                client.hello();client.call('torrents.add',dict(source=dict(path=str(fixture.torrent))))
                h.wait_for(lambda:((rows:=tracker_rows(client,fixture.info_hash))[0]['status']=='error',json.dumps(rows)),'first announce fails')
                result,_=cli(case,sandbox,binary,'reannounce',fixture.info_hash,'--json')
                case.check(result==dict(ok=True,action='reannounce',count=1,hashes=[fixture.info_hash]),'reannounce uses the stable act envelope',json.dumps(result))
                h.wait_for(lambda:((rows:=tracker_rows(client,fixture.info_hash))[0]['status']=='ok',json.dumps(rows)),'CLI force wakes backoff')
                detail,_=cli(case,sandbox,binary,'show',fixture.info_hash,'--json')
                case.check(detail.get('tracker_count')==1 and len(detail.get('trackers',[]))==1 and detail['trackers'][0].get('peers')==1,'show adds truthful tracker_count and tracker rows',json.dumps(detail))
                cli(case,sandbox,binary,'stop',fixture.info_hash,'--json')
                failure,_=cli(case,sandbox,binary,'reannounce',fixture.info_hash,'--json',code=1)
                case.check(failure.get('code')==1 and failure.get('ok') is False,'paused invalid_state keeps CLI exit 1',json.dumps(failure))
                cli(case,sandbox,binary,'reannounce','unknown-zz','--json',code=6)
                cli(case,sandbox,binary,'reannounce','--json',code=2)
        finally:
            if process.child.poll() is None:h.stop_serve(process)


from agent_stream import (events_follow_completion, events_since_seq_replay, events_resync_when_older, events_down_exit_4, api_list_matches, api_list_matches_ipc_md, api_unknown_op_hint, api_mutating_starts_daemon_read_does_not)

from daemon_limits import limits_verb

CASES={f.__name__:f for f in [limits_verb,reannounce_verb,events_follow_completion,events_since_seq_replay,events_resync_when_older,events_down_exit_4,api_list_matches,api_list_matches_ipc_md,api_unknown_op_hint,api_mutating_starts_daemon_read_does_not,reads_never_start,contract,offline_records,json_determinism,version_skew_warning,deprecation_warning_legacy_mode,exit_code_dictionary_total]}
if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary',type=Path)
    parser.add_argument('--systemd',action='store_true')
    parser.add_argument('--case',action='append',choices=CASES)
    parser.add_argument('--log-root',type=Path,default=Path(__file__).resolve().parents[3]/'target/test-logs/fbt-p1-11-zqb/cli')
    args=parser.parse_args()
    import fbtlog
    fbtlog.LOG_ROOT=args.log_root
    for name in args.case or CASES:CASES[name](args.binary.resolve(),args.systemd)
    sys.exit(suite_status('agent_contract'))
