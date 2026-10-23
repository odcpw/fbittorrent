#!/usr/bin/env python3
"""Real native TUI smoke at 80/90/120/160 columns; private XDG, peers and tmux.

Each UI step retains its complete captured pane, on success and failure. Setup
has its own suite so forced failures in the UI suite always carry a real pane.
"""
import argparse
import base64
from contextlib import ExitStack, contextmanager
import json
import os
import re
from pathlib import Path
import socket
import struct
import sys
import time

import daemon as d
import fbtlog
from fbtlog import Case, REPO, suite_status
import fixtures
import native
import tomllib

sys.path.insert(0,str(REPO/'plugin/tests'))
import private_net

ACTIVE='The Diplomat S04E01 1080p WEB-DL H264-GROUP'
DONE='Sintel.2010.1080p'
META='Other.Movie.2026.1080p'
SIZES={'layout_80x24':(80,24),'layout_90x24':(90,24),'layout_120x30':(120,30),'layout_160x45':(160,45)}
CASES=(*SIZES,'poster_overlay_cleanup','low_chroma_theme_markers','log_history','turtle_limits','peer_port')


class PaneUi(native.AttachedTui):
    def __init__(self,*args,**kwargs):
        super().__init__(*args,**kwargs)
        self.setup_case=self.case
        self.panes=0
        self.target=None
        self.context=""

    def start_owned(self):
        self.case=self.setup_case
        self.target,self.done=super().start()
        return self.target,self.done

    def observe(self, action, expect, predicate=lambda pane:True, timeout=5, *, artifacts=()):
        step=self.case.step(action,expect)
        deadline=time.monotonic()+timeout
        pane='';ok=False;error=''
        while True:
            pane=self.capture(self.target)
            try:ok=bool(predicate(pane))
            except (OSError,ValueError,EOFError) as failure:error=f'{type(failure).__name__}: {failure}'
            if ok or time.monotonic()>=deadline:break
            time.sleep(.025)
        self.panes+=1
        artifact=self.case.path.with_suffix(f'.{self.panes:03}.pane.txt')
        artifact.write_text(pane)
        return step.close(ok,(self.context+'\n'if self.context else '')+pane+(f'\nCheck error: {error}' if error else ''),[artifact,*artifacts])

    def press(self,key,expect,predicate=lambda pane:True,*,literal=True):
        result=self.tmux('send-keys','-t',self.target,*(['-l',key]if literal else[key]))
        return self.observe('tmux send-keys '+('-l 'if literal else '')+repr(key),expect,
                            lambda pane:result.returncode==0 and predicate(pane))

    def contains(self,key,text,*,literal=True):
        return self.press(key,'pane contains '+text,lambda pane:text in pane,literal=literal)

    def details_tab(self,name):
        def selected(pane):
            match=re.search(r'\[(Overview|Files|Peers|Trackers|Pieces|Log)\]',pane)
            return match.group(1) if match else None
        for _ in range(8):
            pane=self.capture(self.target);current=selected(pane)
            if current==name:return True
            header=next((line for line in pane.splitlines() if current and '['+current+']' in line),'')
            if not current or name not in header:break
            key=']' if header.index(name)>header.index(current) else '['
            if not self.press(key,'details changes tab toward '+name,lambda p:selected(p) not in (None,current)):return False
        return self.observe('open '+name+' details','the named detail tab is visible',lambda p:selected(p)==name,timeout=0)

    def settings_row(self,label):
        def selected(pane):return next((line for line in pane.splitlines() if '▌' in line and any(name in line for name in ('Simultaneous downloads','Peer encryption','VPN connection','Reduce motion','limit','Peer port','Download destination','Upload:','Idle exit','Clipboard offers'))),'')
        for _ in range(24):
            before=selected(self.capture(self.target))
            if label in before:return True
            if not self.press('Down','choose Settings field '+label,lambda p:selected(p)!=before,literal=False):return False
        return self.observe('select '+label,'the named Settings row is selected',lambda p:label in selected(p),timeout=0)

    def select_active(self,client):
        rows=client.call('torrents.list',{})['result']['torrents']
        rows.sort(key=lambda row:(-row['added_at'],row['hash']))
        self.press('Home','first confirmed torrent is selected',
                   lambda pane:bool(pane.splitlines())and pane.splitlines()[-1].strip()==rows[0]['name'],literal=False)
        for row in rows[1:]:
            if self.capture(self.target).splitlines()[-1].strip()==ACTIVE:break
            self.press('Down','selected full filename is '+row['name'],
                       lambda pane,name=row['name']:pane.splitlines()[-1].strip()==name,literal=False)
        self.observe('read selection','the real downloading fixture is the keyboard target',
                     lambda pane:pane.splitlines()[-1].strip()==ACTIVE)


@contextmanager
def owned_fixture(binary,setup):
    private_net.require()
    with ExitStack() as stack:
        sandbox=stack.enter_context(fixtures.Sandbox())
        h=stack.enter_context(d.Harness(setup,binary,sandbox))
        process=h.start_serve()
        if not(h.receive_ready()and h.wait_engine()):raise RuntimeError('owned daemon did not become ready')
        seedbox=stack.enter_context(fixtures.Sandbox())
        seed=stack.enter_context(d.Harness(setup,binary,seedbox))
        seed.notify=seed.notify_socket()
        seed_process=seed.spawn(d.DaemonContract.command(binary,300,False),name="seed")
        if not(seed.receive_ready()and seed.wait_engine()):raise RuntimeError('owned seed did not become ready')
        source=sandbox.root/'pristine';source.mkdir()
        video=source/'The.Diplomat.S04E01.mkv';video.write_bytes(os.urandom(16*fixtures.MIB))
        subtitles=source/'English.srt';subtitles.write_bytes(b'1\n00:00:01 --> 00:00:02\nLocal fixture.\n')
        sample=source/'sample.mkv';sample.write_bytes(os.urandom(16384))
        port=tomllib.loads((Path(seedbox.env['XDG_CONFIG_HOME'])/'fbittorrent/config.toml').read_text())['peer_port']
        peers=socket.inet_aton('127.0.0.1')+struct.pack('!H',port)
        tracker=stack.enter_context(fixtures.HttpFixture(source,{'/announce':dict(base64=base64.b64encode(fixtures.bencode(dict(interval=1,peers=peers))).decode())}))
        torrent=fixtures.torrent_fixture(source,[video,subtitles,sample],name=ACTIVE,tracker=tracker.base+'/announce')
        with d.ApiConn(seed)as client:
            d.mutation_hello(client,'smoke-seed')
            d.mutation_request(client,'torrents.add',dict(source=dict(path=str(torrent.torrent)),save_path=str(source),seed=True))
            seed.wait_for(lambda:((row:=client.call('torrents.get',dict(hash=torrent.info_hash))['result']['torrent'])['state']=='seeding',json.dumps(row)),
                          'real seed verifies every fixture piece')
            client.call('torrents.set_upload_limit',dict(hashes=[torrent.info_hash],bps=32768))
        # A real verified prefix, followed by genuine capped loopback transfer.
        (sandbox.downloads/video.name).write_bytes(video.read_bytes()[:7*fixtures.MIB])
        completed=sandbox.downloads/DONE;completed.write_bytes(os.urandom(32768))
        done=fixtures.torrent_fixture(sandbox.root,[completed],name='completed')
        native.prepare_imdb_fixture(sandbox.env,[ACTIVE,DONE,META])
        with d.ApiConn(h)as client:
            d.mutation_hello(client,'smoke-preconditions')
            d.mutation_request(client,'torrents.add',dict(source=dict(path=str(torrent.torrent)),save_path=str(sandbox.downloads)))
            d.mutation_request(client,'torrents.add',dict(source=dict(path=str(done.torrent)),save_path=str(sandbox.downloads)))
            client.call('torrents.add',dict(source=dict(magnet='magnet:?xt=urn:btih:'+'e'*40+'&dn='+META)))
            h.wait_for(lambda:((rows:=client.call('torrents.list',{})['result']['torrents'])and
                              {r['state']for r in rows}=={'downloading','done','metadata'}and
                              any(r['name']==ACTIVE and 0<r.get('completed_bytes',0)<r.get('size',0)and r.get('dl_bps',0)>0 for r in rows),json.dumps(rows)),
                       'P0-16 states come from real payload checks, capped transfer and unresolved metadata',timeout=15)
        ui=PaneUi(setup,str(binary),sandbox,h,session='fbt-fbt-p4-10-bce')
        stack.callback(ui.close)
        yield sandbox,h,ui,process,torrent
        # Harness resources own termination even after an assertion/exception.


def drive_size(name,size,binary,setup,sandbox,h,ui,process,torrent):
    width,height=size
    target,done=ui.start_owned()
    with Case('tui_smoke',name)as case:
        ui.case=case
        try:
            resize=ui.tmux('resize-window','-t',target,'-x',str(width),'-y',str(height))
            ui.observe('resize terminal and read list',f'real fixture list at {width}x{height}',
                       lambda pane:resize.returncode==0 and len(pane.splitlines())==height and all(title in pane for title in(ACTIVE,DONE,META)))
            ui.observe('read state column','worded states remain visible without relying on colour',
                       lambda pane:all(any(prefix in line and label in line for line in pane.splitlines())
                                       for prefix,label in [('The Diplomat','↓ Down'),('Sintel','✓ Done'),('Other.Movie','◌ Meta')]))
            with d.ApiConn(h)as client:
                client.hello();ui.select_active(client)
                ui.contains('Enter','[Overview]  Files',literal=False)
                ui.contains('[','[Overview]  Files')
                ui.contains(']','Overview  [Files]')
                ui.observe('read files','real file names and include controls are visible',
                           lambda pane:all(text in pane for text in('The.Diplomat.S04E01.mkv','English.srt','sample.mkv','include/skip')))
                ui.details_tab('Log')
                header=next(line for line in ui.capture(ui.target).splitlines() if '[Log]' in line)
                last=re.findall(r'\b(?:Overview|Files|Peers|Trackers|Pieces|Log)\b',header)[-1]
                ui.details_tab(last)
                ui.contains(']','['+last+']')
                ui.details_tab('Files')
                ui.details_tab('Overview')
                ui.contains('Escape','[Torrents]',literal=False)
                before=client.call('torrents.get',dict(hash=torrent.info_hash))['result']['torrent']['intent']
                ui.contains('d','keep files')
                ui.contains('Escape','[Torrents]',literal=False)
                ui.observe('check cancelled remove','cancel preserves the confirmed downloading record',
                           lambda pane:client.call('torrents.get',dict(hash=torrent.info_hash))['result']['torrent']['intent']==before)
                ui.contains('2','Search')
                ui.press('Escape','leave query editing before switching screens',lambda pane:'[Query]'not in pane,literal=False)
                ui.contains('3','Shows ›')
                ui.contains('4','RSS › Feeds')
                ui.contains('1','[Torrents]')
                ui.contains('?','GLOBAL')
                ui.press('Escape','help closes without changing the list',lambda pane:'GLOBAL'not in pane and'[Torrents]'in pane,literal=False)
                downloaded=client.call('torrents.get',dict(hash=torrent.info_hash))['result']['torrent']['completed_bytes']
                def still_transferring(pane):
                    if not(done.exists()and done.read_text()=='0'and process.child.poll()is None):return False
                    row=client.call('torrents.get',dict(hash=torrent.info_hash))['result']['torrent']
                    return row['intent']=='run'and row['state']=='downloading'and row.get('completed_bytes',0)>downloaded
                ui.press('q','q detaches while real loopback payload bytes continue increasing',still_transferring)
            ui.start_owned();ui.case=case
            resize=ui.tmux('resize-window','-t',ui.target,'-x',str(width),'-y',str(height))
            ui.observe('reattach at the same size','same owner and fixture list are visible',lambda pane:resize.returncode==0 and ACTIVE in pane)
            ui.contains('Q','Stop fbittorrent?')
            ui.contains('Escape','[Torrents]',literal=False)
            ui.observe('check cancelled shutdown','cancel keeps the owner running',lambda pane:process.child.poll()is None)
            ui.contains('Q','Stop fbittorrent?')
            ui.press('y','confirmed Q stops the daemon and exits the client cleanly',
                     lambda pane:ui.done.exists()and ui.done.read_text()=='0'and process.child.poll()==0 and not d.DaemonContract.socket_path(sandbox).exists())
            diagnostics=case.path.with_suffix('.shutdown.log');diagnostics.write_text(h.stderr())
            ui.context=h.stderr()
            ui.observe('read shutdown diagnostics','confirmed Q emits healthy diagnostics and drains before unlinking sockets',
                       lambda pane:d.expected_daemon_stderr(h.stderr()),timeout=0,artifacts=[diagnostics])
            ui.context=''
        except Exception as error:
            ui.observe('finish UI actions','all actions complete without an exception',lambda pane:False,timeout=0)
            setup.check(False,'UI actions complete without exception',f'{type(error).__name__}: {error}')
    ui.case=setup


def poster_overlay_cleanup(binary,setup,sandbox,h,ui,process,torrent):
    """Real decoded poster through the portable path; log actual tmux capability."""
    import re,shutil
    cache=Path(sandbox.env['XDG_CACHE_HOME'])/'fbittorrent'
    identifier='tt17491088'
    title=dict(id=identifier,name='The Diplomat',year=2023,kind='TV series',stars='Keri Russell, Rufus Sewell',
               poster_url='http://127.0.0.1/poster-fixture.jpg',rating=7.9,votes=102000,
               blurb='A veteran US diplomat takes a new assignment.',wiki_url='')
    def document(kind,key,value):
        directory=cache/'imdb'/kind;directory.mkdir(parents=True,exist_ok=True)
        path=directory/(re.sub('[^a-z0-9]','_',key.lower())[:120]+'.json');path.write_text(json.dumps(value));path.chmod(0o600)
    document('by-name-v2',ACTIVE,dict(id=identifier));document('by-id',identifier,title)
    posters=cache/'posters';posters.mkdir(exist_ok=True)
    shutil.copyfile(fixtures.HERE/'fixtures/imdb/poster.jpg',posters/(identifier+'.jpg'))
    (posters/(identifier+'.jpg')).chmod(0o600)
    (cache/'title.ratings.tsv').write_text('tconst\taverageRating\tnumVotes\n'+identifier+'\t7.9\t102000\n')
    target,done=ui.start_owned()
    with Case('tui_smoke','poster_overlay_cleanup')as case:
        ui.case=case
        ui.tmux('resize-window','-t',target,'-x','80','-y','24')
        # kitty_supported() intentionally disables the graphics layer under tmux.
        # Text capture can prove the half-block path; the real kitty encoder has
        # its own delete/restore control in tui::poster_tests.
        features=ui.tmux('display-message','-p','-t',target,'#{pane_tty} #{client_termfeatures}').stdout.strip()
        ui.observe('detect terminal graphics; kitty=false under tmux; '+features,
                   'portable half-block path is exercised without inventing kitty support',lambda pane:'Torrents (3)'in pane)
        with d.ApiConn(h)as client:
            client.hello();ui.select_active(client)
        ui.contains('v','IMDb · v/Esc Back')
        ui.observe('read decoded poster','local poster pixels, title, year, rating and cast are visible',
                   lambda pane:'The Diplomat (2023)'in pane and '7.9'in pane and 'Keri Russell'in pane and any(c in pane for c in('▀','▄')))
        ui.contains('?','GLOBAL')
        ui.observe('capture Help over poster; kitty=false','no poster cells remain in the Help capture',lambda pane:'GLOBAL'in pane and '▀'not in pane and '▄'not in pane)
        ui.press('Escape','closing Help restores the poster card',lambda pane:'IMDb · v/Esc Back'in pane and any(c in pane for c in('▀','▄')),literal=False)
        ui.press('Escape','Esc hides the centered card and returns to the list',lambda pane:'IMDb · v/Esc Back'not in pane and '[Torrents]'in pane,literal=False)
        ui.press('q','q detaches the card test while the owner survives',lambda pane:done.exists()and done.read_text()=='0'and process.child.poll()is None)
    ui.case=setup


def low_chroma_theme_markers(binary,setup,sandbox,h,ui,process,torrent):
    """Actual failed recheck/open, never fabricated API state or UI text."""
    import shutil
    with d.ApiConn(h)as client:
        client.hello()
        rows=client.call('torrents.list',{})['result']['torrents']
        selected=next(row for row in rows if row['name']==ACTIVE)
        cached=Path(sandbox.env['XDG_DATA_HOME'])/'fbittorrent/engine'/(selected['hash']+'.torrent')
        metainfo=cached.read_bytes();cached.unlink();cached.mkdir()
        hidden=sandbox.root/'held-downloads'
        try:
            client.call('torrents.recheck',dict(hashes=[selected['hash']]),ref='theme-error')
            h.wait_for(lambda:((rows:=client.call('torrents.list',{})['result']['torrents'])and any(row['hash']==selected['hash']and row['state']=='error'for row in rows),json.dumps(rows)),
                       'real failed metainfo-cache write becomes an engine error')
            sandbox.downloads.rename(hidden)
            with Case('tui_smoke','low_chroma_theme_markers')as case:
                for name,low in [('owner',True),('tokyo-night',False)]:
                    source=REPO/'contracts/ui/themes'/(name+'.toml')
                    destination=Path(sandbox.env['XDG_STATE_HOME'])/'omarchy/current/theme/colors.toml'
                    destination.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(source,destination)
                    target,done=ui.start_owned();ui.tmux('resize-window','-t',target,'-x','80','-y','24')
                    ui.case=case;ui.context=f'theme={name}; colors={destination}; fixture={source}'
                    ui.observe('load isolated theme','the real error row has its worded state cell',lambda pane:'! Error'in pane)
                    current=client.call('torrents.list',{})['result']['torrents'];current.sort(key=lambda row:(-row['added_at'],row['hash']))
                    ui.press('Home','first row selected',lambda pane:pane.splitlines()[-1].strip()==current[0]['name'],literal=False)
                    for row in current[1:]:
                        if ui.capture(target).splitlines()[-1].strip()==ACTIVE:break
                        ui.press('Down','select '+row['name'],lambda pane,title=row['name']:pane.splitlines()[-1].strip()==title,literal=False)
                    ui.observe('read keyboard target','the failed downloading fixture is selected',lambda pane:pane.splitlines()[-1].strip()==ACTIVE)
                    ui.press('o','failed open reports an error with the correct monochrome marker',
                             lambda pane:('! Error'in pane)and(pane.splitlines()[-1].lstrip().startswith('!')==low)and
                             any(text in pane.splitlines()[-1].lower()for text in('missing','not found','directory','cannot','no such','could not','reading','internal','invalid_state')))
                    ui.press('q','q detaches after the error',lambda pane:done.exists()and done.read_text()=='0')
                    ui.case=setup;ui.context=''
        finally:
            if hidden.exists():hidden.rename(sandbox.downloads)
            cached.rmdir();cached.write_bytes(metainfo);cached.chmod(0o600)


def turtle_limits(binary,setup,sandbox,h,ui,process,torrent):
    # Retain the existing runner selector; exercise ordinary limits and the
    # retired key while older API clients may still use alternate limits.
    with d.ApiConn(h)as client:
        client.hello();client.call('transfer.set_limits',dict(download_bps=0,alt_download_bps=65536,upload_bps=0));client.call('transfer.set_alt',dict(on=True))
        ui.start_owned();ui.tmux('resize-window','-t',ui.target,'-x','80','-y','24')
        with Case('tui_smoke','turtle_limits')as case:
            ui.case=case
            ui.observe('receive confirmed summary','the list is ready without a retired limits indicator',lambda pane:ACTIVE in pane and'Turtle'not in pane and'(z)'not in pane)
            ui.press('z','the retired key leaves the current list visible',lambda pane:'[Torrents]'in pane and'Turtle'not in pane)
            ui.contains('F6','Download settings',literal=False)
            settings=client.call('app.settings.get')['result']
            case.check(settings['alt_limits_on'],'z followed by F6 never changes the legacy backend setting',json.dumps(settings))
            ui.observe('review ordinary Settings','download and upload limits remain editable without alternate-mode fields',lambda pane:'Download limit'in pane and'Upload limit'in pane and'Turtle'not in pane)
            ui.settings_row('Download limit')
            ui.contains('Right','Download limit · B/s',literal=False)
            ui.press('C-a','select the existing draft rate',lambda pane:'Download limit · B/s'in pane,literal=False)
            ui.press('128K','type a numeric rate with K suffix',lambda pane:'128K'in pane)
            ui.contains('Enter','131072 B/s',literal=False)
            client.call('transfer.set_limits',dict(upload_bps=196608))
            ui.observe('receive shared settings update','the untouched upload cap follows the API while the download draft remains',lambda pane:'196608 B/s'in pane and'131072 B/s'in pane)
            ui.contains('Enter','[Torrents]',literal=False)
            settings=client.call('app.settings.get')['result']
            case.check(settings.get('download_limit_bps')==131072 and settings.get('upload_limit_bps')==196608 and settings.get('alt_download_limit_bps')==65536 and settings['alt_limits_on'],
                       'F6 saves only the edited rate and preserves concurrent upload and legacy limits',json.dumps(settings))
            rows=client.call('torrents.list',{})['result']['torrents']
            case.check(all(row['upload_enabled'] is False and row['seed'] is False and row['upload_policy']=='off'
                           and row.get('ul_bps',0)==0 and row.get('uploaded_bytes',0)==0 for row in rows),
                       'editing global caps never enables payload upload for default-off torrents',json.dumps(rows))
            ui.press('q','q detaches without changing the confirmed limits or stopping the owner',lambda pane:ui.done.exists()and ui.done.read_text()=='0'and process.child.poll()is None)
        ui.case=setup


def peer_port(binary,setup,sandbox,h,ui,process,torrent):
    with d.ApiConn(h)as client, socket.socket()as reservation:
        client.hello();client.call('subscribe',dict(topics=['events']))
        reservation.bind(('127.0.0.1',0));port=reservation.getsockname()[1]
        ui.start_owned();ui.tmux('resize-window','-t',ui.target,'-x','80','-y','24')
        before=client.call('app.info')['result'];client.pushed.clear()
        with Case('tui_smoke','peer_port')as case:
            ui.case=case
            ui.contains('F6','Download settings',literal=False)
            ui.settings_row('Peer port')
            ui.contains('Right','Peer port · 0 automatic',literal=False)
            ui.press('C-a','select the current port text',lambda pane:'Peer port · 0 automatic'in pane,literal=False)
            ui.press(str(port),'type a free private peer port',lambda pane:str(port)in pane)
            ui.contains('Enter',str(port),literal=False)
            reservation.close()
            ui.contains('Enter','[Torrents]',literal=False)
            def restarted():
                info=client.call('app.info')['result']
                changed=[frame for frame in client.pushed if frame.get('ev')=='engine.changed']
                return info['state']=='running'and info['engine_epoch']==before['engine_epoch']+1 and len(changed)>=2,json.dumps(dict(info=info,changed=changed))
            h.wait_for(restarted,'one port-setting engine restart completes without reconnecting clients')
            after=client.call('app.info')['result'];changed=[frame for frame in client.pushed if frame.get('ev')=='engine.changed']
            settings=client.call('app.settings.get')['result'];active=client.call('app.stats')['result']['active_clients']
            case.check(settings['peer_port']==port and after['instance']==before['instance']and after['clients']['tui']==before['clients']['tui']and active['tui']==1 and not ui.done.exists(),
                       'the edited port is applied; owner, observer socket and one TUI connection survive',json.dumps(dict(before=before,after=after,settings=settings,active=active)))
            case.check([frame['state']for frame in changed]==['starting','running'],
                       'the setting produces exactly the two engine.changed phases, without a second client-requested restart',json.dumps(changed))
            with socket.create_connection(('127.0.0.1',port),timeout=1):
                pass
            ui.press('q','q detaches after the engine restart while the owner survives',lambda pane:ui.done.exists()and ui.done.read_text()=='0'and process.child.poll()is None)
        ui.case=setup


def log_history(binary):
    """A real injected engine error and a failed open survive in the pushed log."""
    import shutil
    with Case('tui_smoke_setup','log_fixtures')as setup, fixtures.Sandbox()as sandbox:
        source=sandbox.downloads/'history.mkv';source.write_bytes(b'local log fixture'*1024)
        hidden=sandbox.root/'history.hidden'
        item=fixtures.torrent_fixture(sandbox.root,[source]);fixtures.corrupt_byte(source,0)
        sandbox.env['FBT_TEST_INJECT_ERROR']=item.info_hash;sandbox.keys.append('FBT_TEST_INJECT_ERROR')
        theme=Path(sandbox.env['XDG_STATE_HOME'])/'omarchy/current/theme/colors.toml'
        theme.parent.mkdir(parents=True);shutil.copyfile(REPO/'contracts/ui/themes/owner.toml',theme)
        with d.Harness(setup,binary,sandbox)as h:
            process=h.start_serve()
            if not(h.receive_ready()and h.wait_engine()):raise RuntimeError('private log owner failed startup')
            ui=PaneUi(setup,str(binary),sandbox,h,session='fbt-fbt-p4-08-p3e')
            try:
                with d.ApiConn(h)as client:
                    client.hello()
                    ui.start_owned();ui.tmux('resize-window','-t',ui.target,'-x','120','-y','30')
                    h.wait_for(lambda:((info:=client.call('app.info')['result'])['subscriptions']['by_topic'].get('log',0)==1,json.dumps(info['subscriptions'])),'the TUI subscribes before the injected diagnostic occurs')
                    client.call('torrents.add',dict(source=dict(path=str(item.torrent)),save_path=str(sandbox.downloads),stopped=True))
                    h.wait_for(lambda:((entries:=client.call('app.log',dict(hash=item.info_hash,level='error'))['result']['entries'])and any('injected engine error'in row['message']for row in entries),json.dumps(entries)),'the real error-injection hook writes the engine log')
                    source.rename(hidden)
                    with Case('tui_smoke','log_history')as case:
                        ui.case=case;ui.context=f'private theme={theme}; injected hash={item.info_hash}'
                        ui.observe('load torrent','the incomplete fixture appears in the attached terminal',lambda pane:'history.mkv'in pane)
                        ui.press('o','the real open action fails with a worded monochrome status',lambda pane:pane.splitlines()[-1].lstrip().startswith('!'))
                        failure=ui.capture(ui.target).splitlines()[-1].lstrip('! ').strip();needle=failure[:32]
                        expires=time.monotonic()+10.5
                        ui.observe('wait for the ten-second toast expiry','the action toast expires without changing the footer geometry',lambda pane:time.monotonic()>=expires and needle not in '\n'.join(pane.splitlines()[:-2]),timeout=12)
                        ui.press('L','Log retains the failed action and the actual injected engine diagnostic',lambda pane:'injected engine error'in pane and needle in pane and 'level: warn'in pane)
                        ui.contains('l','level: info')
                        ui.press('Escape','Esc returns to the unchanged torrent list',lambda pane:'[Torrents]'in pane,literal=False)
                        ui.press('q','q detaches the log test while the owner survives',lambda pane:ui.done.exists()and ui.done.read_text()=='0'and process.child.poll()is None)
            finally:
                ui.close()
                # Undo the deliberate missing-file fault before demanding a
                # healthy resume snapshot during fixture shutdown.
                if hidden.exists():hidden.rename(source)
                if process.child.poll()is None:h.stop_serve(process)
                h.lock_free()


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary',nargs='?',type=Path,default=REPO/'target/debug/fbittorrent')
    parser.add_argument('--case',choices=CASES)
    parser.add_argument('--list',action='store_true')
    parser.add_argument('--log-root',type=Path,help='override root for a retained independent harness run')
    args=parser.parse_args()
    if args.list:print('\n'.join(CASES));return 0
    private_net.enter()
    if args.log_root:fbtlog.LOG_ROOT=args.log_root.resolve()
    if args.case=='log_history':
        log_history(args.binary.resolve());return int(bool(suite_status('tui_smoke')or suite_status('tui_smoke_setup')))
    with Case('tui_smoke_setup','fixtures')as setup:
        with owned_fixture(args.binary.resolve(),setup)as(sandbox,h,ui,process,torrent):
            for index,(name,size)in enumerate(([]if args.case in('poster_overlay_cleanup','low_chroma_theme_markers','turtle_limits','peer_port')else[(args.case,SIZES[args.case])]if args.case else SIZES.items())):
                if index:
                    h.notify.close()
                    process=h.start_serve()
                    if not(h.receive_ready()and h.wait_engine()):raise RuntimeError('restarted fixture daemon did not become ready')
                    with d.ApiConn(h)as client:
                        client.hello()
                        h.wait_for(lambda:((row:=client.call('torrents.get',dict(hash=torrent.info_hash))['result']['torrent'])['state']=='downloading',json.dumps(row)),
                                   'saved incomplete fixture resumes its real loopback transfer')
                drive_size(name,size,args.binary.resolve(),setup,sandbox,h,ui,process,torrent)
            if args.case in(None,'poster_overlay_cleanup'):
                if process.child.poll()is not None:
                    h.notify.close();process=h.start_serve()
                    if not(h.receive_ready()and h.wait_engine()):raise RuntimeError('poster fixture owner did not restart')
                poster_overlay_cleanup(args.binary.resolve(),setup,sandbox,h,ui,process,torrent)
            if args.case in(None,'peer_port'):
                peer_port(args.binary.resolve(),setup,sandbox,h,ui,process,torrent)
            if args.case in(None,'turtle_limits'):
                turtle_limits(args.binary.resolve(),setup,sandbox,h,ui,process,torrent)
            if args.case in(None,'low_chroma_theme_markers'):
                low_chroma_theme_markers(args.binary.resolve(),setup,sandbox,h,ui,process,torrent)


    if args.case is None:log_history(args.binary.resolve())
    return int(bool(suite_status('tui_smoke')or suite_status('tui_smoke_setup')))


if __name__=='__main__':sys.exit(main())
