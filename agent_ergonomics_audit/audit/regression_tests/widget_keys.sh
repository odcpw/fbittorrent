#!/usr/bin/env bash
# Real Panel.qml + Service.qml in a disposable Quickshell instance. CLI is a
# recording fixture; no owner daemon, torrents, shell config or plugin is used.
# Opt in: FBT_TEST_WIDGET=1 ./widget_keys.sh (Wayland, qs, wtype, QtTest required).
set -euo pipefail
[[ "${FBT_TEST_WIDGET:-0}" == 1 ]] || { echo 'widget tests skipped (set FBT_TEST_WIDGET=1)'; exit 0; }
cd "$(dirname "$0")/../../.."
python3 - <<'PY'
import datetime, json, os, pathlib, shutil, subprocess, tempfile, time
repo=pathlib.Path.cwd()
suite='widget-p0-13-'+str(time.time_ns())
logs=repo/'target/test-logs'/suite
logs.mkdir(parents=True,exist_ok=True)
original=dict(os.environ)
steps={}; failed=set(); last_step=time.monotonic()
qslog=logs/'W-010.qs.log'
qslog.touch()
def run(*args,env=None):
    return subprocess.run(args,env=env or envtest,capture_output=True,text=True,check=True,timeout=10).stdout.strip()
def record(case,action,expect,observed,ok=True,final=False):
    global last_step
    now=time.monotonic(); elapsed=round((now-last_step)*1000); last_step=now
    steps[case]=steps.get(case,0)+1
    row=dict(ts=datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='milliseconds').replace('+00:00','Z'),suite=suite,case=case,step=steps[case],action=action,expect=expect,observed=observed,ok=ok,ms=elapsed,artifacts=[str(qslog.relative_to(repo))])
    if final: row['final']=True
    with (logs/(case+'.jsonl')).open('a') as output: output.write(json.dumps(row)+'\n')
    if not ok: failed.add(case)
    print(json.dumps(row),flush=True)
def wait(check):
    end=time.monotonic()+8
    while time.monotonic()<end:
        try:
            value=check()
            if value: return value
        except (subprocess.SubprocessError,json.JSONDecodeError): pass
        time.sleep(.05)
    raise AssertionError('fixture condition timed out; see '+str(qslog))
with tempfile.TemporaryDirectory(prefix='fbt-p0-13-widget-') as temporary:
    root=pathlib.Path(temporary)
    envtest=dict(original)
    for name in ['XDG_RUNTIME_DIR','XDG_DATA_HOME','XDG_CONFIG_HOME','XDG_STATE_HOME','XDG_CACHE_HOME']:
        path=root/name; path.mkdir(mode=0o700); envtest[name]=str(path)
    display=original.get('WAYLAND_DISPLAY','wayland-0')
    envtest['WAYLAND_DISPLAY']=display if display.startswith('/') else str(pathlib.Path(original['XDG_RUNTIME_DIR'])/display)
    for name in ['Commons','Ui']: (root/name).symlink_to(pathlib.Path('/usr/share/omarchy/shell')/name,target_is_directory=True)
    shutil.copytree(repo/'plugin/oliver.torrents',root/'widget')
    state=root/'status.json'; actions=root/'actions.jsonl'
    torrents=[dict(hash=f'{i:040x}',name=f'<b>P0-13 fixture {i}</b>',state='stoppedDL',progress=.5,size=1024,up_limit=32768) for i in [1,2]]
    def save(rows):
        tmp=state.with_suffix('.tmp')
        tmp.write_text(json.dumps(dict(ok=True,dl=0,up=0,active=0,seeding=0,stopped=len(rows),total=len(rows),torrents=rows))); tmp.replace(state)
    save(torrents)
    binary=root/'fbittorrent-fixture'
    binary.write_text('''#!/usr/bin/env python3
import json,os,pathlib,sys
state=pathlib.Path(os.environ['FBT_WIDGET_STATE'])
if sys.argv[1]=='status': print(state.read_text())
else:
    with open(os.environ['FBT_WIDGET_ACTIONS'],'a') as output: output.write(json.dumps(sys.argv[1:])+'\\n')
    if sys.argv[1]=='delete':
        data=json.loads(state.read_text()); data['torrents']=[t for t in data['torrents'] if t['hash']!=sys.argv[2]]
        tmp=state.with_suffix('.tmp'); tmp.write_text(json.dumps(data)); tmp.replace(state)
    print('{"ok":true}')
''')
    binary.chmod(0o700)
    envtest.update(FBT_WIDGET_BIN=str(binary),FBT_WIDGET_STATE=str(state),FBT_WIDGET_ACTIONS=str(actions))
    (root/'shell.qml').write_text('''import QtQuick
import QtTest
import Quickshell
import Quickshell.Io
import "widget" as Widgets
ShellRoot {
  PanelWindow {
    implicitWidth: 40; implicitHeight: 32
    anchors { top: true; left: true }
    exclusionMode: ExclusionMode.Ignore
    Widgets.Panel {
      id: widget
      width: 32; height: 32
      ipcTarget: "p0-13-widget"
      settings: ({binary: Quickshell.env("FBT_WIDGET_BIN"), refreshIntervalSec: 1, idleRefreshIntervalSec: 2})
    }
  }
  TestCase { id: events; name: "FixtureClicks"; when: false }
  IpcHandler {
    target: "fixture"
    function question(): string {
      var item = events.findChild(widget, "removal-question")
      return JSON.stringify({text: item.text, plain: item.textFormat === Text.PlainText})
    }
    function park(): string {
      var item = events.findChild(widget, "cancel-removal")
      events.mouseMove(item, -300, -300)
      return "parked"
    }
    function visualChild(item, name) {
      if (!item) return null
      if (item.objectName === name) return item
      for (var i = 0; i < item.children.length; ++i) {
        var found = visualChild(item.children[i], name)
        if (found) return found
      }
      return null
    }
    function click(name: string): string {
      var item = events.findChild(widget, name) || visualChild(events.findChild(widget, "torrent-rows"), name)
      if (!item || !item.visible) return "missing " + name
      events.mouseClick(item, item.width / 2, item.height / 2)
      return "clicked"
    }
  }
}
''')
    proc=None; current='W-001'
    def ipc(method,*args,target='p0-13-widget'): return run('qs','ipc','-p',str(root),'call',target,method,*args)
    def cur(): return json.loads(ipc('cursor'))
    def keys(*args): run('wtype',*args); time.sleep(.12)
    def click(name):
        result=ipc('click',name,target='fixture')
        assert result=='clicked', f'click {name}: {result!r}'
    def action_rows(): return [json.loads(line) for line in actions.read_text().splitlines()] if actions.exists() else []
    def pending():
        def ready():
            s=cur()
            return s if s['confirming'] and s['confirmFocused']=='cancel' else False
        return wait(ready)
    try:
        with qslog.open('w') as output:
            proc=subprocess.Popen(['qs','-p',str(root),'--no-color'],env=envtest,stdout=output,stderr=subprocess.STDOUT)
        wait(lambda: cur()['rows']==2)
        ipc('open'); wait(lambda: cur()['opened']); time.sleep(.2)
        record(current,'open disposable popup','opened with two fixture rows',json.dumps(cur()),cur()['rows']==2,True)
        current='W-002'; keys('j'); keys('j')
        record(current,'press j twice','cursor on row 1',json.dumps(cur()),cur()['row']==1,True)
        current='W-003'; keys('k'); first=cur(); keys('k')
        record(current,'press k twice','row 0 then header',json.dumps([first,cur()]),first['row']==0 and cur()['section']=='header',True)
        current='W-004'; keys('l')
        record(current,'press l','header index 1',json.dumps(cur()),cur()['header']==1,True)
        current='W-005'; keys('-k','Escape'); wait(lambda:not cur()['opened'])
        record(current,'press Escape','popup closes',json.dumps(cur()),not cur()['opened'],True)
        current='W-010'; ipc('open'); time.sleep(.2)
        record(current,'cursor before d','no confirmation; two torrents remain',json.dumps(cur()),not cur()['confirming'] and len(json.loads(state.read_text())['torrents'])==2)
        keys('d'); after=pending(); question=json.loads(ipc('question',target='fixture'))
        record(current,'press d','confirm strip opens with Cancel focused; literal title; no deletion',json.dumps(dict(cursor=after,question=question)),not action_rows() and question['plain'] and torrents[0]['name'] in question['text'])
        assert ipc('park',target='fixture')=='parked'
        keys('-k','Return'); wait(lambda:not cur()['confirming'])
        record(current,'press Enter on Cancel','torrent remains; no deletion dispatched',json.dumps(cur()),len(json.loads(state.read_text())['torrents'])==2 and not action_rows())
        for key in ['Escape','Return']:
            click('trash-'+torrents[0]['hash']); before=pending()
            record(current,'click row trash','confirmation only, Cancel focused',json.dumps(before),not action_rows())
            assert ipc('park',target='fixture')=='parked'
            keys('-k',key); wait(lambda:not cur()['confirming'])
            record(current,'press '+key,'both fixture torrents remain',json.dumps(cur()),len(json.loads(state.read_text())['torrents'])==2 and not action_rows())
        keys('d'); before=pending(); save(list(reversed(torrents))); ipc('refresh'); wait(lambda:cur()['hashes'][0]==torrents[1]['hash']); keys('y')
        wait(lambda:len(json.loads(state.read_text())['torrents'])==1)
        record(current,'confirm y after rows reorder','remove original pending hash only; keep files',json.dumps(dict(before=before,after=cur(),actions=action_rows())),action_rows()==[['delete',torrents[0]['hash']]])
        wait(lambda:cur()['rows']==1)
        click('trash-'+torrents[1]['hash']); pending(); click('confirm-removal')
        wait(lambda:len(json.loads(state.read_text())['torrents'])==0)
        record(current,'click explicit Remove','second removal dispatched exactly once with files kept',json.dumps(action_rows()),action_rows()==[['delete',torrents[0]['hash']],['delete',torrents[1]['hash']]])
    except Exception as error:
        record(current,'run fixture case','all steps complete',str(error),False)
    finally:
        if proc:
            proc.terminate()
            try: proc.wait(timeout=5)
            except subprocess.TimeoutExpired: proc.kill(); proc.wait()
        record(current,'finish case','all case steps pass','failed cases: '+str(sorted(failed)),current not in failed,True)
    if failed:
        for case in sorted(failed):
            for path in [logs/(case+'.jsonl'),qslog]:
                print(str(path)+'\n'+'\n'.join(path.read_text(errors='replace').splitlines()[-20:]))
        raise SystemExit(1)
PY
