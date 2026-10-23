const {test}=require('node:test')
const assert=require('node:assert/strict')
const fs=require('node:fs')
const path=require('node:path')
const vm=require('node:vm')
const plugin=path.join(__dirname,'../oliver.torrents')
const read=name=>fs.readFileSync(path.join(plugin,name),'utf8')
const plain=value=>JSON.parse(JSON.stringify(value))
function library(name,imports={}) {
  const context=vm.createContext(imports)
  vm.runInContext(read(name).replace(/^\.(?:pragma|import).*$/gm,''),context)
  return context
}
// Compile the real QML JS body. Braces inside strings/comments or nested
// functions cannot end a candidate until the JavaScript parser accepts it.
function handler(file,marker,context) {
  const source=read(file),offset=source.indexOf(marker)
  assert.ok(offset>=0,`${file}: ${marker}`)
  const begin=source.indexOf('function',offset)
  for(let end=source.indexOf('}',begin);end>=0;end=source.indexOf('}',end+1)) {
    let script
    try {script=new vm.Script('('+source.slice(begin,end+1)+')')}
    catch {continue}
    return script.runInContext(context)
  }
  throw Error(`No complete handler: ${file}: ${marker}`)
}
function bind(file,name,context) {
  context[name]=handler(file,'function '+name+'(',context)
  return context[name]
}
const a='a'.repeat(40),b='b'.repeat(40)
function service() {
  const context=vm.createContext({online:true,shutdownRequested:false,bin:'/private/bin/fbittorrent',
    client:{request(){throw Error('QML must not acknowledge playback before the CLI')}},
    store:{torrents:{[a]:{hash:a},[b]:{hash:b}}},contentOpen:{running:false,command:[]},
    contentOpenOutput:{text:''},contentOpenError:{text:''},messages:[],sent:[]})
  context.root=context
  context.notify=text=>context.messages.push(text)
  context.mutate=(op,args,cap,done)=>{
    context.sent.push({op,args,done});return 'request-'+context.sent.length
  }
  for(const name of ['openTarget','openFolder','openTorrent','quit','startDaemon','needsStopConfirm'])bind('Service.qml',name,context)
  return context
}
test('file/folder/index opens go through the CLI once, with startup disabled and literal argv',()=>{
  for(const [args,tail] of [[[a],[a]],[[a,0],['--file','0',a]],[[a,7],['--file','7',a]],[[a,7,true],['--folder',a]]]) {
    const s=service()
    assert.equal(s.openTarget(...args),true)
    assert.deepEqual(plain(s.contentOpen.command),['/usr/bin/env',s.bin,'--no-start','--json','open',...tail])
  }
  const s=service();assert.equal(s.openFolder(a),true)
  assert.deepEqual(plain(s.contentOpen.command).slice(-2),['--folder',a])
})
test('rapid repeated opens preserve the running process command and give feedback',()=>{
  const s=service();s.openTarget(a,7)
  const before=plain(s.contentOpen.command)
  assert.equal(s.openFolder(b),false)
  assert.deepEqual(plain(s.contentOpen.command),before)
  assert.deepEqual(s.messages,['Opening…'])
})
test('open refuses offline, stopping and vanished rows without launching',()=>{
  for(const change of [s=>s.online=false,s=>s.shutdownRequested=true,s=>delete s.store.torrents[a]]) {
    const s=service();change(s)
    assert.equal(s.openTarget(a),false);assert.equal(s.contentOpen.running,false)
  }
})
test('CLI errors expose API JSON, launcher stderr and empty-output failures',()=>{
  for(const [out,err,code,wanted] of [
    ['{"ok":false,"code":1,"error":"file is missing"}','',1,'file is missing'],
    ['{"ok":false,"error":{"message":"file is not ready"}}','raw',1,'file is not ready'],
    ['','player could not start\n',2,'player could not start'],
    ['','/usr/bin/env: missing binary',127,'missing binary'],
    ['','',1,'launcher exited 1'],
  ]) {
    const s=service();s.contentOpenOutput.text=out;s.contentOpenError.text=err
    handler('Service.qml','id:contentOpen',s)(code)
    assert.ok(s.messages[0].startsWith('Could not open: '));assert.ok(s.messages[0].includes(wanted))
  }
  const s=service();handler('Service.qml','id:contentOpen',s)(0);assert.deepEqual(s.messages,[])
})
test('popup Stop closes the initiator before a reply while observers remain untouched',()=>{
  const s=service(),c=vm.createContext({svc:s,stopping:true,owner:{opened:true,close(){this.opened=false}},focuses:0})
  c.forceActiveFocus=()=>c.focuses++
  handler('components/PopupContent.qml','function stop(',c)()
  assert.equal(c.owner.opened,false);assert.equal(c.stopping,false)
  assert.equal(c.focuses,0);assert.equal(s.online,true);assert.equal(s.shutdownRequested,true)
  assert.equal(s.sent.length,1);assert.equal(s.sent[0].op,'app.shutdown')
  assert.equal(s.quit(),false);assert.equal(s.sent.length,1)
  assert.equal(s.startDaemon(),false);assert.ok(s.messages.includes('Stop is still pending'))
  s.sent[0].done(null,{message:'lost reply'})
  assert.equal(c.owner.opened,false);assert.equal(s.shutdownRequested,false)
  assert.equal(s.shutdownError,'Could not stop: lost reply')
  assert.ok(s.messages.includes('Could not stop: lost reply'));assert.equal(s.sent.length,1)
})
test('a synchronously refused Stop leaves the popup open and releases its guard',()=>{
  const s=service();s.mutate=(op,args,cap,done)=>{done(null,{message:'busy'});return false}
  const c=vm.createContext({svc:s,stopping:true,owner:{opened:true,close(){this.opened=false}},focused:false})
  c.forceActiveFocus=()=>c.focused=true
  handler('components/PopupContent.qml','function stop(',c)()
  assert.equal(c.owner.opened,true);assert.equal(c.focused,true);assert.equal(s.shutdownRequested,false)
  assert.ok(s.messages.includes('Could not stop: busy'))
})
test('dashboard accepted Stop dismisses its own window before daemon exit',()=>{
  const s=service(),c=vm.createContext({svc:s,dismissed:0,focusView(){}})
  c.dismiss=()=>c.dismissed++;c.Qt={callLater(){throw Error('Accepted Stop must not refocus')}}
  handler('Dashboard.qml','function stop(',c)()
  assert.equal(c.dismissed,1);assert.equal(s.online,true);assert.equal(s.shutdownRequested,true)
})
test('Stop confirmation keeps active, seeding, unknown and flowing transfers protected',()=>{
  const s=service()
  for(const [status,flow,expected] of [[{},false,true],[{loaded:true},false,false],
    [{loaded:true,active:1},false,true],[{loaded:true,seeding:1},false,true],[{loaded:true},true,true]]) {
    s.status=status;s.transferring=flow;assert.equal(s.needsStopConfirm(),expected)
  }
})
test('a lost shutdown is not retained for automatic replay',()=>{
  const writes=[],api=library('ApiClient.js'),c=api.create({write:text=>writes.push(JSON.parse(text)),flush(){},isConnected(){return false}})
  c.online=true;c.instance='old'
  let failures=0;c.request('app.shutdown',{reason:'user'},{mutating:true,callback:(result,error)=>{if(error)failures++}})
  c.disconnected()
  assert.equal(failures,1);assert.equal(Object.keys(c.pending).length,0)
  assert.equal(c.queue.length,0);assert.equal(Object.keys(c.awaiting).length,0)
  assert.equal(api.IDEMPOTENT.includes('app.shutdown'),false)
  assert.equal(writes.filter(frame=>frame.op==='app.shutdown').length,1)
})
test('popup snapshot reorder preserves selection by hash and removed middle header leaves Power reachable',()=>{
  const c=vm.createContext({rows:[{hash:a},{hash:b}],sections:[],rowIndex:1,groupIndex:0,
    entries:[],expanded:{},cursorActive:true,focusSection:'rows',pendingRemoval:null,
    svc:{online:true,torrents:[{hash:b},{hash:a}],store:{torrents:{}}},PopupState:{group:()=> 'live'}})
  c.syncEntries=items=>c.entries=items;bind('components/PopupContent.qml','selected',c)
  bind('components/PopupContent.qml','rebuild',c)()
  assert.equal(c.selected().hash,b);assert.equal(c.rowIndex,0)
  c.svc.torrents=[];c.rebuild();assert.equal(c.focusSection,'header')
  c.headerIndex=2;c.keySerial=0;c.currentStrip=()=>null
  handler('components/PopupContent.qml','function move(',c)(1,0)
  assert.equal(c.headerIndex,2)
  let powers=0;c.power=()=>powers++
  handler('components/PopupContent.qml','function header(',c)();assert.equal(powers,1)
})
test('file and folder keys dispatch independently in the dashboard and Files tab',()=>{
  for(const file of ['views/TorrentsView.qml','views/DetailsFiles.qml']) {
    const opened=[],c=vm.createContext({cursorHash:a,current:{hash:a},torrent:{hash:a},editing:false,focusPane:'table',
      pane:null,overviewSheet:{item:null},recheckConfirm:{opened:false},removeDialog:{key:()=>false},
      service:{clearFailures(){},openTarget:hash=>opened.push(['file',hash]),openFolder:hash=>opened.push(['folder',hash])},
      flow:{openFile:()=>opened.push(['index',7])},
      Qt:{Key_Escape:1,Key_Q:2,Key_Down:3,Key_Up:4,Key_PageDown:5,Key_PageUp:6,Key_Home:7,Key_End:8,
        Key_Left:9,Key_Right:10,Key_Return:11,Key_Enter:12,Key_Space:13,ControlModifier:1}})
    const key=handler(file,'function '+(file.includes('TorrentsView')?'handleKey':'key')+'(',c)
    assert.equal(key({text:'o',key:0,modifiers:0}),true)
    assert.equal(key({text:'O',key:0,modifiers:2}),true)
    assert.deepEqual(opened,[[file.includes('TorrentsView')?'file':'index',file.includes('TorrentsView')?a:7],['folder',a]])
  }
})
test('legacy alternate settings remain wire compatible without controls, hints or bar indicators',()=>{
  const Glyphs=library('Glyphs.js'),Formats=library('Formats.js'),Palette=library('Palette.js')
  const Model=library('Model.js',{Glyphs,Formats,Palette}),PopupState=library('PopupState.js',{Model})
  const bar=library('BarState.js',{Glyphs,Model,PopupState}),schema=library('SettingsSchema.js')
  const s={online:true,status:{},client:{caps:[]},store:{summary:{alt:true,counts:{downloading:1},dl_bps:1000}}}
  const on=plain(bar.describe(s));s.store.summary.alt=false
  assert.deepEqual(plain(bar.describe(s)),on)
  assert.equal(schema.fields.some(field=>field.key.startsWith('alt_')),false)
  assert.equal('turtle' in Glyphs,false)
  for(const file of ['components/PopupContent.qml','Dashboard.qml','components/PopupKeyHints.qml','components/BarIndicator.qml']) {
    assert.doesNotMatch(read(file),/turtle|toggleTurtle|text\s*===\s*["']z["']/i)
  }
})
test('generated help advertises file/folder case consistently and omits the retired key',()=>{
  const actions=JSON.parse(read('keymap.json')).actions
  const open=actions.find(action=>action.id==='torrents.open')
  const folder=actions.find(action=>action.id==='torrents.open_folder')
  for(const surface of ['dashboard','popup','tui']) {
    assert.ok(open.keys[surface].includes('o'))
    assert.ok(folder.keys[surface].includes('O'))
  }
  assert.equal(actions.some(action=>Object.values(action.keys).some(keys=>keys && keys.includes('z'))),false)
  assert.equal(actions.some(action=>/turtle/i.test(action.label)),false)
})
