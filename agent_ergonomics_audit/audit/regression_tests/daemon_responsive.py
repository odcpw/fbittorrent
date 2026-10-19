"""P4-04 responsive native terminal proof, using a private real engine."""
import json
from pathlib import Path
import daemon as d
import fixtures


def tui_details_narrow(binary, systemd):
    import native
    with d.api_owned('tui_details_narrow',binary,systemd) as (case,sandbox,h,process),d.ApiConn(h) as client:
        d.mutation_hello(client,'details-observer')
        directory=sandbox.root/'details-payload';directory.mkdir()
        fixture=fixtures.two_file_fixture(directory)
        for path in fixture.files:fixtures.corrupt_byte(path,0)
        d.mutation_request(client,'torrents.add',dict(source=dict(path=str(fixture.torrent)),stopped=True,save_path=str(directory)))
        native.prepare_imdb_fixture(sandbox.env,['two-files'])
        ui=native.AttachedTui(case,str(binary),sandbox,h,session='fbt-fbt-p4-04-9lu')
        try:
            target,done=ui.start()
            resized=ui.tmux('resize-window','-t',target,'-x','80','-y','24')
            case.check(resized.returncode==0,'terminal is exactly 80 by 24',resized.stderr)
            ui.assert_pane(target,'‖ Pause')
            ui.keys(target,'Enter',literal=False)
            ui.assert_pane(target,'[Overview]  Files')
            ui.assert_pane(target,'[Details]')
            ui.assert_pane(target,'paused by you')
            pane=ui.capture(target)
            case.check(pane.splitlines()[0].startswith('╭ two-files'),
                       'narrow Details occupies the whole terminal with the stable torrent name',pane)
            ui.keys(target,'[')
            ui.assert_pane(target,'[Overview]  Files')
            ui.keys(target,']')
            ui.assert_pane(target,'Overview  [Files]')
            ui.assert_pane(target,'first.bin');ui.assert_pane(target,'second.bin')
            ui.keys(target,']')
            ui.assert_pane(target,'[Log]')
            ui.keys(target,'[')
            ui.assert_pane(target,'Overview  [Files]')
            ui.assert_pane(target,'first.bin')
            ui.assert_pane(target,'second.bin')
            ui.keys(target,' ')
            def skipped():
                files=client.call('torrents.files',dict(hash=fixture.info_hash))['result']['files']
                return not files[0]['included'] and files[1]['included'],json.dumps(files)
            h.wait_for(skipped,'Files tab changes real selection on the same native API connection')
            ui.keys(target,'[')
            ui.assert_pane(target,'[Overview]  Files')
            ui.keys(target,'h')
            ui.assert_pane(target,'[Torrents]')
            ui.keys(target,'Enter',literal=False)
            ui.assert_pane(target,'[Overview]  Files')
            ui.keys(target,'Escape',literal=False)
            ui.assert_pane(target,'[Torrents]')
            ui.keys(target,'q')
            h.wait_for(lambda:(done.exists(),ui.capture(target)),'q detaches the narrow terminal')
            row=client.call('torrents.get',dict(hash=fixture.info_hash))['result']['torrent']
            case.check(process.child.poll() is None and row['intent']=='pause',
                       'navigating and detaching leaves the owner and paused torrent intact',json.dumps(row))
        finally:ui.close()


CASES=(tui_details_narrow,)


def tui_search_narrow(binary, systemd):
    import native
    import daemon_search as search
    with search.owned('tui_search_narrow',binary,systemd) as (case,sandbox,h,process,runtime),d.ApiConn(h) as client:
        client.hello();native.prepare_imdb_fixture(sandbox.env,['辛特尔'])
        ui=native.AttachedTui(case,str(binary),sandbox,h,session='fbt-fbt-p4-04-9lu-search')
        try:
            target,done=ui.start()
            resize=ui.tmux('resize-window','-t',target,'-x','80','-y','24')
            case.check(resize.returncode==0,'search terminal is 80 by 24',resize.stderr)
            ui.keys(target,'2');ui.keys(target,'Sintel');ui.keys(target,'Enter',literal=False)
            ui.assert_pane(target,'辛特尔');ui.assert_pane(target,'SEEDS');ui.assert_pane(target,'★')
            pane=ui.capture(target)
            case.check('╭ Details' not in pane,'narrow result list keeps its name/size/seeds/rating columns without a side card',pane)
            ui.keys(target,'Escape',literal=False)
            ui.keys(target,'v');ui.assert_pane(target,'╭ IMDb');ui.assert_pane(target,'alpha, beta')
            ui.keys(target,'Escape',literal=False);ui.assert_pane(target,'Results (1)')
            ui.keys(target,'q');h.wait_for(lambda:(done.exists(),ui.capture(target)),'q detaches after card navigation')
            case.check(process.child.poll() is None,'card navigation never stops the owner',json.dumps(client.call('app.info')['result']))
        finally:ui.close()


CASES=CASES+(tui_search_narrow,)
