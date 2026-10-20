"""P3-02: local counted feeds, store edits, bounded fetch and deliberate grabs."""
import json
import os
import time
import daemon as d
import fixtures


def all_events(c):
    c.call('app.info');out=[]
    for frame in c.pushed:
        if frame.get('ev')=='mutation.result':out.append(frame)
        out.extend(frame.get('added',frame.get('items',[])) if frame.get('topic')=='events' else [])
    return out


def run(name,binary,systemd):
    with d.api_owned(name,binary,systemd) as (case,box,h,p),d.ApiConn(h) as c,fixtures.HttpFixture(box.root) as http:
        payload=box.root/'rss-owned.bin';payload.write_bytes(os.urandom(32768))
        item=fixtures.torrent_fixture(box.root,[payload])
        source=box.root/'feed.xml'
        source.write_text('<rss><channel><title>Local feed</title><item><guid>good</guid><title>Good fixture</title><enclosure url="'+http.base+'/'+item.torrent.name+'" type="application/x-bittorrent"/></item><item><guid>bad</guid><title>Malformed magnet</title><link>magnet:?xt=urn:btih:invalid</link></item></channel></rss>')
        c.hello('bar')
        def count():return sum(path=='/feed.xml' for path in http.requests)
        def stale():
            path=d.Path(box.env['XDG_DATA_HOME'])/'fbittorrent/rss.json'
            value=json.loads(path.read_text())
            for row in value['feeds']:row['last_checked']=int(time.time())-1801
            temporary=path.with_suffix('.fixture.tmp');temporary.write_text(json.dumps(value));temporary.replace(path)
            case.step('make the owned cached feed stale','next interactive edge may refresh once').ok(json.dumps(dict(last_checked=value['feeds'][0]['last_checked'],hits=count())))
        if name=='rss_cli_edit_visible':
            c.call('subscribe',dict(topics=['rss.feeds']))
            result=h.cli('rss','add',http.base+'/feed.xml','--name','CLI feed')
            began=time.monotonic()
            def pushed():
                c.call('app.info')
                frames=[f for f in c.pushed if f.get('topic')=='rss.feeds']
                rows=[r for f in frames for r in f.get('added',f.get('items',[]))]
                return any(r.get('name')=='CLI feed' for r in rows),json.dumps(dict(elapsed=time.monotonic()-began,frames=frames,cli=result,hits=count()))
            h.wait_for(pushed,'CLI store edit appears in pushed rss.feeds within two seconds',timeout=2)
            feed=c.call('rss.feeds')['result']['feeds'][0]
        else:
            feed=c.call('rss.add',dict(url=http.base+'/feed.xml',name='Local'))['result']['feed']
        identity=feed['id']
        c.call('subscribe',dict(topics=['rss.feeds','rss.items:'+identity,'events']))
        items=c.call('rss.items',dict(feed=identity))['result']['items']
        case.check(count()==1 and len(items)==2 and all('size' not in row and 'published_at' not in row for row in items),'one feed fetch caches two items and omits unknown sizes/dates',json.dumps(dict(hits=count(),feed=feed,items=items)))
        if name=='rss_refresh_on_interactive':
            stale();c.call('app.client',dict(interactive=True));time.sleep(.25)
            case.check(count()==1,'bar presence does not schedule network work',str(http.requests))
            for lap in range(2):
                with d.ApiConn(h) as opened:
                    opened.call('hello',dict(proto=1,client=dict(kind='tui',name='rss-open',version='0'),interactive=True))
                    def refreshed():
                        rows=c.call('rss.feeds')['result']['feeds']
                        return count()==2 and not rows[0]['loading'],json.dumps(dict(hits=count(),feeds=rows,frames=c.pushed))
                    h.wait_for(refreshed,'first stale interactive open refreshes once; fresh reconnect does not',timeout=3)
                    time.sleep(.25)
                    case.check(count()==2,'no periodic or duplicate refresh follows an interactive open',str(http.requests))
            stale()
            with d.ApiConn(h) as opened:
                opened.call('hello',dict(proto=1,client=dict(kind='tui',name='rss-stale-again',version='0'),interactive=True))
                h.wait_for(lambda:(count()==3,json.dumps(dict(hits=count(),requests=http.requests))),'another stale interactive connect gets exactly one refresh',timeout=3)
            case.check(c.call('torrents.list',{})['result']['torrents']==[],'refreshes never add a torrent',str(http.requests))
        elif name=='rss_oversized_refused':
            source.write_bytes(b'<rss><channel>'+b'x'*(4*1024*1024)+b'</channel></rss>')
            pending=c.call('rss.refresh',dict(feed=identity))['result']
            found=[]
            def failed():
                found[:]=[e for e in all_events(c) if e.get('ref')==pending['ref']]
                return bool(found),json.dumps(found)
            h.wait_for(failed,'pending refresh produces an observed failure envelope',timeout=4)
            error=found[-1].get('error',{})
            after=c.call('rss.items',dict(feed=identity))['result']['items']
            current=c.call('rss.feeds')['result']['feeds'][0]
            case.check(found[-1]['outcome']=='failed' and error.get('code')=='upstream' and error.get('details',{}).get('source')=='rss' and after==items and current.get('error') and count()==2,
                       'oversized body is upstream/rss, explains refusal, and preserves all cached articles',json.dumps(dict(envelope=found[-1],before=len(items),after=len(after),feed=current,hits=count())))
        elif name=='rss_grab_marks_read_only_on_success':
            c.call('rss.grab',dict(feed=identity,item='bad'),error='invalid_args')
            bad=next(r for r in c.call('rss.items',dict(feed=identity))['result']['items'] if r['id']=='bad')
            case.check(not bad['read'] and not bad['grabbed'],'failed grab changes neither read nor grabbed intent',json.dumps(bad))
            added=c.call('rss.grab',dict(feed=identity,item='good',stopped=True))['result']
            row=c.call('torrents.get',dict(hash=item.info_hash))['result']['torrent']
            good=next(r for r in c.call('rss.items',dict(feed=identity))['result']['items'] if r['id']=='good')
            case.check(added.get('hash')==item.info_hash and row.get('origin')=='rss' and good['read'] and good['grabbed'],'successful real metainfo add uses rss origin and commits read/grabbed',json.dumps(dict(add=added,torrent=row,item=good)))
            c.call('rss.set_read',dict(feed=identity,item='good',read=False))
            first=c.call('rss.mark_feed_read',dict(feed=identity))['result']
            again=c.call('rss.mark_feed_read',dict(feed=identity))['result']
            case.check(first==dict(count=2) and again==dict(count=0),'mark-feed-read is idempotent and counts changed unread rows',json.dumps([first,again]))
            cli=h.cli('rss','items','Local')
            case.check(all(r['isRead'] for r in cli['items']),'in-process CLI reads the API-committed read history',json.dumps(cli))
            (box.root/'child.xml').write_bytes(source.read_bytes())
            child=c.call('rss.add',dict(url=http.base+'/child.xml',name='Local/Child'))['result']['feed']
            c.call('rss.remove',dict(feed=identity))
            remaining=c.call('rss.feeds')['result']['feeds']
            case.check([r['id'] for r in remaining]==[child['id']],'removing a feed id preserves a different feed with a nested display name',json.dumps(remaining))
            c.call('rss.remove',dict(feed=child['id']))
            case.check(c.call('rss.feeds')['result']['feeds']==[],'remove commits the same shared store',str(http.requests))
        case.step('retain RSS topic and request-count observations','all snapshots/deltas and fixture hits are reviewable').ok(json.dumps(dict(frames=c.pushed,requests=http.requests,hits=count())))


def rss_refresh_on_interactive(binary,systemd):run('rss_refresh_on_interactive',binary,systemd)
def rss_cli_edit_visible(binary,systemd):run('rss_cli_edit_visible',binary,systemd)
def rss_oversized_refused(binary,systemd):run('rss_oversized_refused',binary,systemd)
def rss_grab_marks_read_only_on_success(binary,systemd):run('rss_grab_marks_read_only_on_success',binary,systemd)


def rss_passkeys_redacted(binary,systemd):
    """Private feed credentials stay in the 0600 store; every outward surface is redacted."""
    import subprocess
    with d.api_owned('rss_passkeys_redacted',binary,systemd) as (case,box,h,p),d.ApiConn(h) as c,fixtures.HttpFixture(box.root) as http:
        payload=box.root/'private-owned.bin';payload.write_bytes(os.urandom(16384))
        item=fixtures.torrent_fixture(box.root,[payload])
        secrets=['SECRETPASSKEY','SECRETAUTHKEY','SECRETUSERPW']
        (box.root/'private.xml').write_text('<rss><channel><title>Private</title><item><guid>p1</guid><title>Private&#x1b;]52;c;cHduZWQ=&#x7;item&#x1b;[2J</title>'
            '<link>'+http.base+'/details.php?id=7&amp;authkey=SECRETAUTHKEY</link>'
            '<enclosure url="'+http.base+'/'+item.torrent.name+'?passkey=SECRETPASSKEY" type="application/x-bittorrent"/></item></channel></rss>')
        feed_url='http://feeduser:SECRETUSERPW@'+http.base.split('://',1)[1]+'/private.xml?passkey=SECRETPASSKEY'
        c.hello('bar')
        c.call('subscribe',dict(topics=['rss.feeds']))
        added=c.call('rss.add',dict(url=feed_url))['result']['feed']
        identity=added['id']
        c.call('subscribe',dict(topics=['rss.items:'+identity]))
        feeds=c.call('rss.feeds')['result']['feeds'];items=c.call('rss.items',dict(feed=identity))['result']['items']
        duplicate=c.call('rss.add',dict(url=feed_url),error='invalid_state')
        listing=h.cli('rss','list');rows=h.cli('rss','items',added['name'])
        text=subprocess.run([binary,'--no-start','rss','list'],env=box.env,cwd=box.root,capture_output=True,text=True,timeout=10)
        human=subprocess.run([binary,'--no-start','rss','items',added['name']],env=box.env,cwd=box.root,capture_output=True,text=True,timeout=10)
        case.check(human.returncode==0 and 'Private' in human.stdout and not any(ch in human.stdout+text.stdout for ch in '\x1b\x07\x9b'),
                   'human CLI output strips remote terminal control sequences',repr(human.stdout+human.stderr))
        c.call('app.info')
        observed=json.dumps(dict(added=added,feeds=feeds,items=items,duplicate=duplicate,listing=listing,rows=rows,text=text.stdout+text.stderr,pushed=c.pushed))
        case.check(len(items)==1 and items[0].get('torrent_url') and not any(s in observed for s in secrets),
                   'feed URL, default name, item link/torrent URL, errors, pushes and CLI text/JSON carry no credentials',observed)
        grabbed=c.call('rss.grab',dict(feed=identity,item='p1',stopped=True))['result']
        case.check(grabbed.get('hash')==item.info_hash and any('SECRETPASSKEY' in r for r in http.requests),
                   'the owner still grabs from the stored source, passkey included',json.dumps(dict(grab=grabbed,requests=http.requests)))
        stored=d.Path(box.env['XDG_DATA_HOME'])/'fbittorrent/rss.json'
        case.check(stored.stat().st_mode&0o777==0o600 and 'SECRETPASSKEY' in stored.read_text(),
                   'raw values remain only in the private 0600 store',oct(stored.stat().st_mode))


CASES=(rss_passkeys_redacted,)
