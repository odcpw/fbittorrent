#!/usr/bin/env python3
"""One owner RSS journey through the real dashboard, isolated from live data."""
import argparse
import json
from pathlib import Path
import sys
from dashboard import ROOT
from journey_support import JourneySession
import daemon
import fixtures
from fbtlog import Case,suite_status


class RssSession(JourneySession):
    def keys(self,args,predicate):
        # Native input is sent only to our isolated compositor/window.
        try:return super().keys(args,predicate)
        except TimeoutError as error:raise TimeoutError('RSS keys '+repr(args)+': '+str(error)+'; actions='+self.ipc('settingsTrace')+'; active='+self.ipc('activeWindow')) from error


def view(q):return q.state().get('view') or {}
def rss(q):return view(q).get('rss') or {}
def add(q):return view(q).get('add') or {}


def run(binary):
    with Case('qs_rss','feed_to_deliberate_download') as case,daemon.Sandbox() as box,fixtures.HttpFixture(box.root) as http,daemon.Harness(case,binary,box) as h:
        payload=box.root/'rss-journey.bin';payload.write_bytes(b'private owned fixture'*2048)
        torrent=fixtures.torrent_fixture(box.root,[payload]);before=payload.read_bytes()
        (box.root/'feed.xml').write_text('<rss><channel><title>Owner fixture feed</title><item><guid>item</guid><title>Deliberate download</title><description>Only Download starts this transfer.</description><enclosure url="'+http.base+'/'+torrent.torrent.name+'" type="application/x-bittorrent"/></item></channel></rss>')
        saved=box.downloads/'unrelated.bin';saved.write_bytes(b'keep existing files')
        process=h.start_serve();h.receive_ready();h.wait_engine();q=RssSession(case,box)
        try:
            q.wait(lambda:q.ipc('ready')=='true');q.shell('summon','{"view":"rss"}');q.wait(lambda:'rss' in view(q))
            q.keys(['n'],lambda:rss(q).get('modal')=='add')
            q.keys([http.base+'/feed.xml','-k','Tab','Owner feed','-k','Return'],lambda:len(rss(q).get('feeds',[]))==1 and not rss(q).get('modal'))
            feed=rss(q)['feed']
            q.keys(['-k','Return'],lambda:rss(q).get('pane')=='items' and len(rss(q).get('items',[]))==1)
            row=rss(q)['items'][0];case.check(not row['read'] and 'size' not in row,'feed articles show unread and unknown size honestly',json.dumps(row))
            q.compositor.ipc('[title="^fbittorrent$"] floating enable, resize set width 900 px height 720 px')
            q.wait(lambda:view(q).get('rssNarrow'));q.keys(['-k','Left'],lambda:rss(q).get('pane')=='feeds')
            q.keys(['-k','Right'],lambda:rss(q).get('pane')=='items')
            case.check(view(q)['rssNarrow'],'narrow layout keeps feeds and articles reachable with native keys','Left to feeds, Right to articles at 900 px')
            q.keys(['m'],lambda:rss(q)['items'][0]['read']);q.keys(['m'],lambda:not rss(q)['items'][0]['read'])
            q.keys(['M'],lambda:rss(q)['items'][0]['read']);q.keys(['m'],lambda:not rss(q)['items'][0]['read'])
            hits=sum(x=='/feed.xml' for x in http.requests)
            q.keys(['r'],lambda:sum(x=='/feed.xml' for x in http.requests)>hits)
            with daemon.ApiConn(h) as c:
                c.hello()
                assert c.call('torrents.list',{})['result']['torrents']==[]
                q.keys(['-k','Return'],lambda:add(q).get('phase')=='ready')
                staged=c.call('torrents.get',dict(hash=torrent.info_hash))['result']['torrent']
                case.check(staged['staged'] and not rss(q)['items'][0]['read'],'review stages metadata without marking read',json.dumps(staged))
                q.keys(['-k','Escape'],lambda:not add(q).get('visible'))
                c.call('torrents.get',dict(hash=torrent.info_hash),error='not_found')
                q.wait(lambda:not rss(q)['items'][0]['read'])
                case.check(payload.read_bytes()==before and saved.read_bytes()==b'keep existing files' and not (box.downloads/payload.name).exists(),'discard preserves unread, source bytes and unrelated files','no payload destination created')
                q.keys(['-k','Return'],lambda:add(q).get('phase')=='ready')
                q.keys(['-k','Return'],lambda:not add(q).get('visible'))
                q.shell('summon','{"view":"rss"}');q.wait(lambda:rss(q).get('items') and rss(q)['items'][0]['grabbed'] and rss(q)['items'][0]['read'])
                row=c.call('torrents.get',dict(hash=torrent.info_hash))['result']['torrent']
                case.check(not row['staged'] and row['origin']=='rss','Download alone commits a real RSS-origin record and read/added state',json.dumps(row))
                q.keys(['x'],lambda:rss(q).get('modal')=='remove' and view(q).get('cancelFocused'))
                q.keys(['-k','Return'],lambda:not rss(q).get('modal'))
                case.check(len(c.call('rss.feeds')['result']['feeds'])==1,'remove defaults to Cancel','feed retained')
                q.keys(['x'],lambda:rss(q).get('modal')=='remove');q.keys(['-k','Right','-k','Return'],lambda:not rss(q).get('feeds'))
                case.check(saved.read_bytes()==b'keep existing files' and payload.read_bytes()==before,'removing a feed preserves all download/source files','source and unrelated bytes unchanged')
                q.keys(['q'],lambda:q.state().get('view') is None)
                case.check(process.child.poll() is None,'closing the dashboard leaves the owner alive','daemon still serves API')
        finally:
            try:q.close()
            finally:
                if process.child.poll() is None:h.stop_serve(process)
    return suite_status('qs_rss')

if __name__=='__main__':
    import private_net
    private_net.enter()
    parser=argparse.ArgumentParser();parser.add_argument('--binary',required=True,type=Path)
    args=parser.parse_args();sys.exit(run(args.binary.resolve()))
