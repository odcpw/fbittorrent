const {test}=require('node:test')
const assert=require('node:assert/strict')
const fs=require('node:fs')
const vm=require('node:vm')
const path=require('node:path')
const api=vm.createContext({})
vm.runInContext(fs.readFileSync(path.join(__dirname,'../oliver.torrents/TrackerText.js'),'utf8').replace(/^\.pragma.*$/m,''),api)
test('tracker paths, named short credentials and encoded echoes stay private',()=>{
  for(const token of ['abc123passkey','SYNTHETIC_PATH_PASSKEY','0123456789abcdef0123456789abcdef','AbCdEf0123456789+/==']){
    const encoded=encodeURIComponent(token),url='https://tracker.invalid/'+encoded+'/announce?passkey=%73hort'
    assert.equal(api.publicUrl(url),'https://tracker.invalid/REDACTED/announce')
    const message=api.message(token+' '+encoded+' short %73hort "'+url+'",https://other.invalid/'+encoded+'/announce',url)
    for(const secret of [token,encoded,'short','%73hort'])assert.ok(!message.includes(secret),message)
  }
  for(const part of ['passkey/abc/announce','key=abc/announce','announce;token=abc','%70asskey/%61bc/announce']){
    const url='https://tracker.invalid/'+part
    assert.ok(!api.publicUrl(url).includes('abc'))
    const text=api.message('abc token=xyz Key=123 PASSKEY=qrs',url)
    for(const secret of ['abc','xyz','123','qrs'])assert.ok(!text.includes(secret),text)
  }
  for(const [path,echo] of [['passkey%2Fabc/announce','abc'],['passkey%252Fabc/announce','abc'],['key=abc%26def/announce','abc&def abc%26def'],['announce;token=abc%3Bdef','abc;def abc%3Bdef']]){
    const url='https://tracker.invalid/'+path
    assert.ok(!api.publicUrl(url).includes('abc'))
    const text=api.message(echo,url)
    assert.ok(!text.includes('abc')&&!text.includes('def'),text)
  }
  assert.equal(api.publicUrl('udp://tracker.invalid:80/announce'),'udp://tracker.invalid:80/announce')
  const special='https://tracker.invalid/passkey/a%26bc/announce?token=quoted%26value'
  const text=api.message('a%26bc a&bc token=quoted&value token="other&value" key=%71%75%6fted%26value',special)
  for(const secret of ['a%26bc','a&bc','quoted','other&value'])assert.ok(!text.includes(secret),text)
})
