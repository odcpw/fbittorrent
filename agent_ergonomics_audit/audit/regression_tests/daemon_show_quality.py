"""P3-18: persisted quality selection and independent inbox read/quality filters."""
import json
import time
import daemon as d
import fixtures

IMDB='tt1234001'


def seed(box):
    path=d.Path(box.env['XDG_DATA_HOME'])/'fbittorrent/shows.json';path.parent.mkdir(exist_ok=True)
    path.write_text(json.dumps(dict(shows=[dict(imdb=IMDB,title='Local Fixture A',added=int(time.time())),dict(imdb='tt1234002',title='Local Fixture B',added=int(time.time()))])))
    return path


def shows_quality_prefers(binary,systemd):
    with d.Case('daemon','shows_quality_prefers') as case,d.Sandbox() as box,fixtures.HttpFixture(box.root,{'/api/get-torrents':dict(file='mixed.json')}) as http:
        box.env['FBT_EZTV_BASE_URL']=http.base;box.keys+=['FBT_EZTV_BASE_URL'];path=seed(box)
        releases=[dict(id=i,imdb_id=IMDB[2:],title=f'Local.Fixture.S01E{episode:02}.{quality}',season='1',episode=str(episode),seeds=seeds,size_bytes='262144',date_released_unix=1792274400+episode,magnet_url='magnet:?xt=urn:btih:'+f'{i:040x}')
                  for i,(episode,quality,seeds) in enumerate([(2,'1080p',900),(2,'2160p',2),(1,'1080p',900),(1,'2160p',2)],1)]
        (box.root/'mixed.json').write_text(json.dumps(dict(imdb_id=IMDB[2:],torrents_count=4,limit=100,page=1,torrents=releases)))
        case.step('prepare mixed-quality EZTV catalogue','2160p preference must outweigh much larger 1080p seed counts').ok(json.dumps(releases))
        with d.Harness(case,binary,box,systemd) as h:
            p=h.start_serve(idle_exit_secs=0);h.receive_ready();h.wait_engine()
            with d.ApiConn(h) as c:
                c.hello()
                first=c.call('shows.set_quality',dict(imdb=IMDB,quality='2160p'))['result']
                second=c.call('shows.set_quality',dict(imdb=IMDB,quality='2160p'))['result']
                case.check(first==second and first['show']['quality']=='2160p' and json.loads(path.read_text())['shows'][0]['quality']=='2160p','setting 2160p is durable and idempotent',json.dumps(dict(first=first,second=second,store=json.loads(path.read_text()))))
            h.stop_serve(p);h.notify.close();p=h.start_serve(idle_exit_secs=0);h.receive_ready();h.wait_engine()
            with d.ApiConn(h) as c:
                c.hello()
                case.check(c.call('shows.list')['result']['shows'][0]['quality']=='2160p','quality survives an actual daemon restart',path.read_text())
                def check():
                    pending=c.call('shows.check',dict(imdb=IMDB))['result']
                    d.mutation_outcome(c,pending['ref'])
                check()
                chosen=c.call('shows.releases',dict(inbox=True,quality='2160p'))['result']['releases']
                case.check(len(chosen)==2 and all(r['quality']=='2160p' and r['seeds']==2 for r in chosen),'next actual fixture check prefers 2160p over 1080p despite seed counts',json.dumps(dict(fixture=releases,chosen=chosen)))
                c.call('shows.set_read',dict(imdb=IMDB,key='S01E01',read=True))
                unread=c.call('shows.releases',dict(inbox=True,unread_only=True,quality='2160p'))['result']['releases']
                all_rows=c.call('shows.releases',dict(inbox=True,unread_only=False,quality='2160p'))['result']['releases']
                other=c.call('shows.releases',dict(inbox=True,quality='720p'))['result']['releases']
                case.check([r['key'] for r in unread]==['S01E02'] and len(all_rows)==2 and any(r['read'] for r in all_rows) and other==[],
                           'inbox filters hide read releases and other qualities independently',json.dumps(dict(unread=unread,all=all_rows,other=other)))
                c.call('shows.set_quality',dict(imdb=IMDB,quality='4k'),error='invalid_args')
                c.call('shows.set_quality',dict(imdb='tt99999',quality='2160p'),error='not_found')
                reset=c.call('shows.set_quality',dict(imdb=IMDB,quality=None)).get('result',{}).get('show',{});check()
                defaults=c.call('shows.releases',dict(imdb=IMDB,unread_only=False))['result']['releases']
                case.check('quality' not in reset and len(defaults)==2 and all(r['quality']=='1080p' for r in defaults) and any(r['read'] for r in defaults),
                           'null restores the configured 1080p default on the next check and preserves read history',json.dumps(dict(show=reset,releases=defaults)))
                case.check(c.call('torrents.list',{})['result']['torrents']==[],'quality edits and checks never add torrents','empty real ledger')
            h.stop_serve(p)


def shows_quality_verb(binary,systemd):
    from agent_contract import cli
    with d.Case('agent_contract','shows_quality_verb') as case,d.Sandbox() as box:
        path=seed(box)
        case.step("verify private environment", "all process paths stay in the fixture").ok(d.private_environment(box))
        result,_=cli(case,box,binary,'shows','quality',IMDB,'2160p','--json')
        case.check(result.get('action')=='shows-quality' and result.get('quality')=='2160p' and json.loads(path.read_text())['shows'][0]['quality']=='2160p','offline CLI stores the selected quality',json.dumps(result))
        cli(case,box,binary,'shows','quality',IMDB,'4k','--json',code=2,valid=False)
        cli(case,box,binary,'shows','quality','missing','2160p','--json',code=6)
        cli(case,box,binary,'shows','quality','Local Fixture','2160p','--json',code=7)
        control=box.root/'shims/systemctl.log'
        commands=[json.loads(line) for line in control.read_text().splitlines()] if control.exists() else []
        case.check(not d.DaemonContract.socket_path(box).exists() and not any('start' in row for row in commands),
                   'offline quality edits and selector errors never start a daemon',json.dumps(commands))
        with d.Harness(case,binary,box,systemd) as h:
            p=h.start_serve(idle_exit_secs=0);h.receive_ready();h.wait_engine()
            result,_=cli(case,box,binary,'shows','quality',IMDB,'720p','--json')
            with d.ApiConn(h) as c:
                c.hello();value=c.call('shows.list')['result']['shows'][0]
                case.check(result.get('quality')=='720p' and value['quality']=='720p','live CLI uses the daemon API and returns the applied quality',json.dumps(dict(cli=result,show=value)))
                result,_=cli(case,box,binary,'shows','quality',IMDB,'default','--json')
                value=c.call('shows.list')['result']['shows'][0]
                case.check(result.get('quality','missing') is None and 'quality' not in value,'CLI default clears the per-show override',json.dumps(dict(cli=result,show=value)))
            h.stop_serve(p)
