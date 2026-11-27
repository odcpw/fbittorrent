"""Everyday TUI actions against a private daemon, payload and local provider."""
import hashlib
import json
from pathlib import Path
import shlex
import daemon as d
import fixtures
import native


def tui_everyday(binary, systemd):
    with d.api_owned('tui_everyday',binary,systemd) as (case,box,h,process), d.ApiConn(h) as client:
        client.hello()
        payload=box.downloads/'everyday.bin';payload.write_bytes(b'everyday payload kept by Undo\n'*4096)
        fixture=fixtures.torrent_fixture(box.root,[payload])
        checksum=hashlib.sha256(payload.read_bytes()).hexdigest()
        client.call('torrents.add',dict(source=dict(path=str(fixture.torrent)),save_path=str(box.downloads),stopped=True))
        copied=box.root/'copied-magnet'
        fixtures.path_shim('wl-copy','cat > '+shlex.quote(str(copied)),box.root)
        engines=Path(box.env['XDG_DATA_HOME'])/'fbittorrent/nova3/engines';engines.mkdir(parents=True,exist_ok=True)
        (engines/'everyday.py').write_text('''class everyday:
    name = 'Everyday local provider'
    url = 'https://everyday.invalid'
    def search(self, query):
        from novaprinter import prettyPrinter
        for n, name, seeds in [(1,'Zulu.S01E01',9),(2,'Alpha.S01E02',2),(3,'Unknown.S01E03',-1)]:
            prettyPrinter(dict(link='magnet:?xt=urn:btih:'+f'{n:040x}',name=name,size=16384,seeds=seeds,leech=-1,engine_url=self.url))
''')
        native.prepare_imdb_fixture(box.env,['everyday.bin','Zulu.S01E01','Alpha.S01E02','Unknown.S01E03'])
        ui=native.AttachedTui(case,str(binary),box,h,session='fbt-s05-everyday')
        try:
            target,done=ui.start();ui.assert_pane(target,'everyday.bin')
            ui.keys(target,'d');ui.assert_pane(target,'y/N')
            ui.keys(target,'Escape',literal=False)
            case.check(len(d.undo_rows(client))==1 and payload.exists(),'cancelled remove preserves row and payload')
            ui.keys(target,'d');ui.keys(target,'y')
            h.wait_for(lambda:(not d.undo_rows(client),ui.capture(target)),'confirmed remove hides its row')
            ui.assert_pane(target,'(U) Undo removal');ui.keys(target,'U')
            h.wait_for(lambda:(len(d.undo_rows(client))==1,ui.capture(target)),'U restores the daemon-owned record')
            row=client.call('torrents.get',dict(hash=fixture.info_hash))['result']['torrent']
            case.check(row['hash']==fixture.info_hash and hashlib.sha256(payload.read_bytes()).hexdigest()==checksum,'Undo preserves exact payload bytes and record identity',json.dumps(row))
            case.check(client.call('app.settings.get')['result']['network_interface']=='lo',
                       'search retains the VPN binding throughout the TUI journey','lo')
            ui.keys(target,'2');ui.keys(target,'local everyday');ui.keys(target,'Enter',literal=False)
            ui.assert_pane(target,'Zulu.S01E01');ui.assert_pane(target,'Alpha.S01E02');ui.assert_pane(target,'Unknown.S01E03')
            ui.keys(target,'Escape',literal=False)
            def names():
                pane=ui.capture(target)
                return [next((name for name in ('Alpha.S01E02','Zulu.S01E01','Unknown.S01E03')if name in line),None)for line in pane.splitlines()if any(name in line for name in ('Alpha.S01E02','Zulu.S01E01','Unknown.S01E03'))]
            ui.keys(target,'s')
            h.wait_for(lambda:(names()==['Alpha.S01E02','Unknown.S01E03','Zulu.S01E01'],ui.capture(target)),'s sorts search by name ascending')
            ui.keys(target,'S')
            h.wait_for(lambda:(names()==['Zulu.S01E01','Unknown.S01E03','Alpha.S01E02'],ui.capture(target)),'S reverses search order')
            ui.keys(target,'y')
            h.wait_for(lambda:(copied.exists(),ui.capture(target)),'y copies the same selected search row')
            case.check(copied.read_text()=='magnet:?xt=urn:btih:'+f'{1:040x}','sorting preserves selection identity through clipboard copy',copied.read_text())
            ui.keys(target,'F6',literal=False);ui.assert_pane(target,'Download settings')
            for _ in range(9):ui.keys(target,'Down',literal=False)
            ui.keys(target,'Right',literal=False);ui.assert_pane(target,'Download destination · absolute folder path')
            destination=box.root/'chosen-downloads';destination.mkdir()
            ui.keys(target,'C-a',literal=False);ui.keys(target,str(destination));ui.keys(target,'Enter',literal=False)
            ui.keys(target,'Down',literal=False);ui.keys(target,'Right',literal=False)
            ui.keys(target,'Down',literal=False);ui.keys(target,'Right',literal=False)
            ui.keys(target,'C-a',literal=False);ui.keys(target,'0');ui.keys(target,'Enter',literal=False)
            ui.assert_pane(target,'Disabled (0)')
            ui.keys(target,'Enter',literal=False)
            def settings_saved():
                settings=client.call('app.settings.get')['result']
                return settings['save_path']==str(destination) and settings['upload']=='on' and settings['idle_exit_secs']==0,json.dumps(settings)
            h.wait_for(settings_saved,'destination, upload permission and disabled idle exit persist through API Settings')
            ui.keys(target,'F6',literal=False);ui.assert_pane(target,'Disabled (0)');ui.assert_pane(target,str(destination))
            ui.keys(target,'Escape',literal=False)
            ui.keys(target,'1');ui.keys(target,'a');ui.keys(target,'magnet:?xt=urn:btih:'+'d'*40+'&dn=New.Defaults');ui.assert_pane(target,'streaming automatic');ui.keys(target,'Enter',literal=False)
            h.wait_for(lambda:('d'*40 in [r['hash']for r in d.undo_rows(client)],ui.capture(target)),'new add uses the saved defaults')
            added=client.call('torrents.get',dict(hash='d'*40))['result']['torrent']
            case.check(added['save_path']==str(destination) and not added['seed'] and added['upload_enabled'],'new torrent inherits destination and upload permission without completion seeding',json.dumps(added))
            client.call('app.settings.set',dict(network_interface='fbt_no_vpn'))
            ui.assert_pane(target,'Waiting for VPN')
            ui.tmux('resize-window','-t',target,'-x','80','-y','24')
            ui.assert_pane(target,'Waiting for VPN')
            case.check('Waiting for VPN' in '\n'.join(ui.capture(target).splitlines()[:3]),'VPN state remains useful at 80 columns',ui.capture(target))
            # Restore the owned binding before the common running-engine drain check.
            client.call('app.settings.set',dict(network_interface='lo'))
            assert h.wait_for(lambda:((info:=client.call('app.info')['result'])['state']=='running',json.dumps(info)),
                              'restored private binding unparks the same daemon before shutdown')
            ui.keys(target,'q');h.wait_for(lambda:(done.exists(),ui.capture(target)),'q detaches after everyday commands')
            case.check(process.child.poll() is None and payload.exists(),'detach preserves daemon and original payload')
        finally:ui.close()


CASES=(tui_everyday,)


def tui_release_sort_copy(binary,systemd):
    imdb='tt1234001'
    with d.Case('daemon','tui_release_sort_copy') as case,d.Sandbox() as box,fixtures.HttpFixture(box.root,{'/api/get-torrents':dict(file='episodes.json')}) as http:
        box.env.update(FBT_EZTV_BASE_URL=http.base,FBT_IMDB_BASE_URL=http.base)
        box.keys+=['FBT_EZTV_BASE_URL','FBT_IMDB_BASE_URL']
        cache=Path(box.env['XDG_CACHE_HOME'])/'fbittorrent/imdb/by-id';cache.mkdir(parents=True)
        (cache/(imdb+'.json')).write_text(json.dumps(dict(id=imdb,name='Local Fixture',year=2026,kind='tvSeries',stars='',poster_url='',rating=None,votes=None,blurb='',wiki_url='')))
        (box.root/'episodes.json').write_text(json.dumps(dict(imdb_id=imdb[2:],torrents_count=2,limit=100,page=1,torrents=[dict(id=i,imdb_id=imdb[2:],title=f'Local.Fixture.S01E0{i}.1080p',season='1',episode=str(i),seeds=i,size_bytes='45056',date_released_unix=1792274400+i,magnet_url='magnet:?xt=urn:btih:'+f'{i:040x}')for i in (2,1)])))
        copied=box.root/'copied-release';fixtures.path_shim('wl-copy','cat > '+shlex.quote(str(copied)),box.root)
        with d.Harness(case,binary,box,systemd) as h:
            process=h.start_serve(idle_exit_secs=0);h.receive_ready();h.wait_engine()
            ui=None
            try:
                with d.ApiConn(h) as client:
                    client.hello();client.call('shows.follow',dict(imdb=imdb));client.call('shows.check',dict(imdb=imdb))
                    h.wait_for(lambda:((rows:=client.call('shows.list')['result']['shows'])and rows[0]['counts']['episodes']==2 and not rows[0]['checking'],json.dumps(rows)),'explicit check loads local release catalogue')
                    ui=native.AttachedTui(case,str(binary),box,h,session='fbt-s05-releases');target,done=ui.start()
                    ui.keys(target,'3');ui.keys(target,'Enter',literal=False);ui.assert_pane(target,'S01E02')
                    ui.keys(target,'s');ui.assert_pane(target,'name ↑')
                    def ordered(keys):
                        pane=ui.capture(target);lines=[line for line in pane.splitlines()if 'Unread' in line and ('S01E01' in line or 'S01E02' in line)]
                        return len(lines)==2 and all(key in line for key,line in zip(keys,lines)),pane
                    h.wait_for(lambda:ordered(['S01E01','S01E02']),'release title sort is ascending')
                    ui.keys(target,'S');ui.assert_pane(target,'name ↓')
                    h.wait_for(lambda:ordered(['S01E02','S01E01']),'release title sort reverses')
                    ui.keys(target,'y');h.wait_for(lambda:(copied.exists(),ui.capture(target)),'release y reaches the clipboard helper')
                    case.check(copied.read_text()=='magnet:?xt=urn:btih:'+f'{2:040x}','sort and reverse preserve the selected release identity',copied.read_text())
                    case.check(not d.undo_rows(client),'sort/copy/check never create a transfer')
                    ui.keys(target,'q');h.wait_for(lambda:(done.exists(),ui.capture(target)),'q detaches from release catalogue')
            finally:
                if ui:ui.close()
                h.stop_serve(process);h.lock_free()


CASES=CASES+(tui_release_sort_copy,)
