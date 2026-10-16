/* Research-only differential oracle. Not loaded by the Android app or Rust library.
 * Run from repository root after obtaining pinned official artifacts documented in
 * docs/rust-auth.md. Every input below is synthetic; no service calls are made. */
const fs=require('fs'),vm=require('vm'),crypto=require('crypto');
const source=fs.readFileSync('research/native-auth/live-resolver.vendor.js','utf8');
const start=source.indexOf('var vt=rt,yt='),end=source.indexOf('var sc=ts,',start);
const ckeySource=source.slice(start,end).replace('var vt=rt,yt=','var yt=');
const fixtureClock=1700000000000;
class FixtureDate extends Date{constructor(...a){super(...(a.length?a:[fixtureClock]))}static now(){return fixtureClock}}
const fixtures=[];
const cases=Array.from({length:12},(_,i)=>({pid:`fixture-pid-${i}`,cnlid:`fixture-cnlid-${i}`,guid:`fixture-guid-${i}`,ts:String(1700000000+i*17),page:{url:i%2?'https://www.yangshipin.cn/tv/home?synthetic=abcdefghijklmnopqrstuvw':'https://www.yangshipin.cn/tv/home',user_agent:i%3?'Fixture UA 146.0 Linux':'FIXTURE User-Agent Extended Metadata abcdefghijklmnopqrstuv',referrer:i%2?'':'https://www.yangshipin.cn/',app_code_name:'Mozilla',app_name:'Netscape',platform:i%2?'Android TV':'Linux x86_64'}}));
(async()=>{
let exports;const objects=[null];let state={};const keygen=await WebAssembly.instantiate(fs.readFileSync('research/native-auth/keygen_bg.wasm'),{wbg:{__wbg_get_9c1840f7ecd81363(p,n){const k=Buffer.from(exports.memory.buffer,p,n).toString();objects.push(state[k]||'');return objects.length-1},__wbindgen_string_get(p,index){const b=Buffer.from(objects[index]);const q=exports.__wbindgen_malloc(b.length,1);new Uint8Array(exports.memory.buffer,q,b.length).set(b);const view=new DataView(exports.memory.buffer);view.setInt32(p,q,true);view.setInt32(p+4,b.length,true)},__wbindgen_object_drop_ref(){}}});exports=keygen.instance.exports;
function callKeygen(name){const p=exports.__wbindgen_add_to_stack_pointer(-16);exports[name](p);const v=new DataView(exports.memory.buffer);const q=v.getInt32(p,true),n=v.getInt32(p+4,true);const text=Buffer.from(exports.memory.buffer,q,n).toString();exports.__wbindgen_add_to_stack_pointer(16);exports.__wbindgen_free(q,n,1);return text}
const original=fs.readFileSync('research/upstream/RJq7sO71JF.js','utf8');
const m=await new Promise(resolve=>{const context={Module:{wasmBinary:fs.readFileSync('research/native-auth/RJq7sO71JF.wasm'),onRuntimeInitialized(){resolve(context.Module)}},Date:FixtureDate,window:{},document:{},process,require,__dirname:process.cwd()+'/research/native-auth',console,WebAssembly,Buffer,TextDecoder,TextEncoder,setTimeout,clearTimeout,setInterval,clearInterval};vm.createContext(context);vm.runInContext(original,context)});
for(const f of cases){const p=f.page;const navigator={userAgent:p.user_agent,appCodeName:p.app_code_name,appName:p.app_name,platform:p.platform};const window={location:{href:p.url},navigator};const context={window,document:{URL:p.url,referrer:p.referrer,location:window.location},navigator,opener:null};vm.createContext(context);vm.runInContext(ckeySource,context);f.ckey=context.ts(f.cnlid,Number(f.ts),'V1.0.0',f.guid,'5910204');
state={'cctvh5openapi.state.version':'v1','cctvh5openapi.state.guid':f.guid,'cctvh5openapi.state.yspappid':'519748109','cctvh5openapi.state.token':'fixture-token','cctvh5openapi.state.input':'fixture-sdk-input','cctvh5openapi.state.ts':String(fixtureClock),'window.location.host':'www.yangshipin.cn','window.location.protocol':'https:'};f.token_rnd=callKeygen('get_token_rnd');f.openapi_signature=callKeygen('get_signature')+'-fixture-sdk-input';
const inputs=[f.pid,f.ts,f.cnlid,f.guid,'519748109','V1.0.0'];const ptrs=inputs.map(s=>{const b=Buffer.from(s+'\0');const q=m._malloc(b.length);m.HEAPU8.set(b,q);return q});const n=Buffer.byteLength(f.pid)+Buffer.byteLength(f.ts)+Buffer.byteLength(f.guid)+9+14;const out=m._malloc(n);m._aes_encrypt_ctr(...ptrs,out);f.ticket=Buffer.from(m.HEAPU8.subarray(out,out+n)).toString('hex');
f.params={cnlid:f.cnlid,livepid:f.pid,guid:f.guid,cKey:f.ckey,adjust:1,sphttps:'1',platform:'5910204',cmd:'2',encryptVer:'8.1',dtype:'1',devid:'devid',otype:'ojson',appVer:'V1.0.0',app_version:'V1.0.0',channel:'ysp_tx',defn:'hd',stream:'2'};
const canon=(o,locale)=>Object.keys(o).sort(locale?(a,b)=>a.localeCompare(b):undefined).map(k=>k+'='+decodeURI(o[k] instanceof Array?o[k].join():o[k])).join('&');
const md5=s=>crypto.createHash('md5').update(s).digest('hex');f.sdk_input=md5(canon(f.params,true));f.live_signature=md5(canon(f.params,false)+'0f$IVHi9Qno?G');f.auth_signature=md5(canon(f.params,false)+'n@7QKk%YeSjfw%22');
fixtures.push(f)}
fs.writeFileSync('native/crates/atv-auth/tests/oracle-fixtures.json',JSON.stringify({clock:fixtureClock,fixtures},null,2)+'\n');
console.log(`Saved ${fixtures.length} synthetic fixtures from original cKey JS, keygen WASM and ticket WASM`);
})();
