'use strict';
/* Optional integration check using an installed Chrome. No npm packages required. */
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { pathToFileURL } = require('node:url');

async function main() {
  const profile = await fs.mkdtemp(path.join(os.tmpdir(), 'sidecar-browser-'));
  const chrome = spawn(process.env.CHROME_BIN || 'google-chrome-stable', [
    '--headless', '--remote-debugging-pipe', `--user-data-dir=${profile}`,
    '--no-first-run', '--no-default-browser-check', '--disable-background-networking',
    '--disable-component-update', '--disable-sync', 'about:blank'
  ], { stdio: ['ignore', 'ignore', 'pipe', 'pipe', 'pipe'] });
  let nextId = 0, pending = new Map(), buffer = '', diagnostics = '', errors = [];
  let exited = false;
  chrome.stderr.on('data', chunk => { diagnostics = (diagnostics + chunk).slice(-4000); });
  chrome.on('error', error => { for (const task of pending.values()) task.reject(error); pending.clear(); });
  chrome.on('exit', (code) => {
    exited = true;
    for (const task of pending.values()) task.reject(new Error(`Chrome exited ${code}: ${diagnostics}`));
    pending.clear();
  });
  chrome.stdio[4].on('data', chunk => {
    buffer += chunk;
    let end;
    while ((end = buffer.indexOf('\0')) >= 0) {
      const raw = buffer.slice(0, end); buffer = buffer.slice(end + 1);
      if (!raw) continue;
      const message = JSON.parse(raw);
      if (message.id) {
        const task = pending.get(message.id); if (!task) continue; pending.delete(message.id);
        if (message.error) task.reject(new Error(JSON.stringify(message.error))); else task.resolve(message.result);
      } else if (message.method === 'Runtime.exceptionThrown') errors.push(message.params.exceptionDetails);
    }
  });
  function call(method, params = {}, sessionId) {
    return new Promise((resolve, reject) => {
      const id = ++nextId;
      const timeout = setTimeout(() => { pending.delete(id); reject(new Error(`CDP timeout: ${method}`)); }, 45000);
      pending.set(id, { resolve: value => { clearTimeout(timeout); resolve(value); }, reject: error => { clearTimeout(timeout); reject(error); } });
      chrome.stdio[3].write(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }) + '\0');
    });
  }
  let session;
  async function evaluate(expression) {
    const response = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true }, session);
    if (response.exceptionDetails) throw new Error(JSON.stringify(response.exceptionDetails));
    return response.result.value;
  }
  async function until(expression) {
    const deadline = Date.now() + 45000;
    while (Date.now() < deadline) {
      if (await evaluate(expression)) return;
      await new Promise(resolve => setTimeout(resolve, 80));
    }
    throw new Error(`Page did not settle: ${expression}`);
  }
  const hasWheelTrails = `(()=>{const c=document.getElementById('scene'); if(!c.width||!c.height)return false;const p=c.getContext('2d').getImageData(0,0,c.width,c.height).data;let blue=0;for(let i=0;i<p.length;i+=4)if(p[i+2]>130&&p[i+2]>p[i]*1.25&&p[i+2]>p[i+1]*1.15)blue++;return blue>10})()`;
  const check = (label, expression) => evaluate(expression).then(value => { assert.ok(value, `${label}; browser errors: ${errors.map(error => error.exception?.description || error.text).join(' | ')}`); console.log(`PASS ${label}`); });
  try {
    await call('Browser.getVersion');
    const target = await call('Target.createTarget', { url: 'about:blank' });
    ({ sessionId: session } = await call('Target.attachToTarget', { targetId: target.targetId, flatten: true }));
    await call('Runtime.enable', {}, session);
    await call('Page.enable', {}, session);
    await call('Emulation.setDeviceMetricsOverride', { width: 1440, height: 1100, deviceScaleFactor: 1, mobile: false }, session);
    await call('Page.navigate', { url: pathToFileURL(path.resolve(process.env.SIDECAR_PAGE || path.join(__dirname, 'index.html'))).href }, session);
    await until(`document.getElementById('result-badge')?.dataset.state === 'success'`);
    if (process.env.SIDECAR_PAGE) await check('standalone entry has no external runtime assets', `document.querySelectorAll('script[src],link[rel="stylesheet"]').length===0`);
    await check('default route and four pole cues', `document.querySelectorAll('#pole-table tr').length===4 && !document.getElementById('play').disabled`);
    await check('all numeric results finite', `!['NaN','Infinity'].some(x=>document.querySelector('main').textContent.includes(x))`);
    await evaluate(`document.getElementById('play').click()`);
    await until(`Number(document.getElementById('scrub').value)>2`);
    await evaluate(`document.getElementById('play').click(); document.getElementById('scrub').value=430; document.getElementById('scrub').dispatchEvent(new Event('input')); document.getElementById('view-follow').click();`);
    await check('scrub, steering telemetry and follow camera', `document.getElementById('view-follow').getAttribute('aria-pressed')==='true' && document.getElementById('steer-value').textContent!=='—'`);
    // Persist screenshots only when explicitly requested; application source stays portable.
    if (process.env.SIDECAR_SCREENSHOTS) {
      await fs.mkdir(process.env.SIDECAR_SCREENSHOTS, { recursive: true });
      await evaluate(`document.getElementById('view-overview').click()`);
      await new Promise(resolve => setTimeout(resolve, 150));
      const shot = await call('Page.captureScreenshot', { format: 'png' }, session);
      await fs.writeFile(path.join(process.env.SIDECAR_SCREENSHOTS, 'desktop.png'), Buffer.from(shot.data, 'base64'));
      await evaluate(`document.getElementById('view-follow').click()`);
      await new Promise(resolve => setTimeout(resolve, 150));
      const follow = await call('Page.captureScreenshot', { format: 'png' }, session);
      await fs.writeFile(path.join(process.env.SIDECAR_SCREENSHOTS, 'follow.png'), Buffer.from(follow.data, 'base64'));
    }
    await evaluate(`document.getElementById('param-laneWidth').value='2.1'; document.getElementById('param-laneWidth').dispatchEvent(new Event('input', {bubbles:true}));`);
    await check('changed parameters invalidate stale route', `document.getElementById('play').disabled && document.getElementById('export').disabled && document.querySelectorAll('#pole-table tr').length===0 && document.getElementById('summary-clearance').textContent==='—'`);
    await new Promise(resolve => setTimeout(resolve, 100));
    await check('editing parameters keeps the previous wheel trails visible', hasWheelTrails);
    await check('retained route is clearly marked as stale', `document.getElementById('scene').dataset.routeState==='stale' && document.getElementById('status').textContent.includes('上次线路')`);
    await evaluate(`document.getElementById('parameters').requestSubmit()`);
    await until(`document.getElementById('result-badge')?.dataset.state === 'danger'`);
    await check('infeasible course disables recommended playback', `document.getElementById('play').disabled && document.getElementById('recommendation').textContent.includes('尚未找到')`);
    await evaluate(`document.getElementById('show-trails').checked=false; document.getElementById('show-envelope').checked=false; document.getElementById('reset').click()`);
    await until(`document.getElementById('result-badge')?.dataset.state === 'success'`);
    await check('reset restores trails, envelope and overview', `document.getElementById('show-trails').checked && document.getElementById('show-envelope').checked && document.getElementById('view-overview').getAttribute('aria-pressed')==='true'`);
    await call('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 1, mobile: true }, session);
    await check('mobile layout has no horizontal page overflow', `document.documentElement.scrollWidth <= document.documentElement.clientWidth`);
    if (process.env.SIDECAR_SCREENSHOTS) {
      await evaluate(`document.getElementById('scene-title').scrollIntoView({block:'start'})`);
      await new Promise(resolve => setTimeout(resolve, 150));
      const shot = await call('Page.captureScreenshot', { format: 'png' }, session);
      await fs.writeFile(path.join(process.env.SIDECAR_SCREENSHOTS, 'mobile.png'), Buffer.from(shot.data, 'base64'));
    }
    await evaluate(`document.getElementById('scene').parentElement.style.display='none'; new Promise(resolve=>setTimeout(resolve,150));`);
    await evaluate(`document.getElementById('scene').parentElement.style.display=''; new Promise(resolve=>setTimeout(resolve,150));`);
    await check('route redraws after canvas is hidden and restored', hasWheelTrails);
    await evaluate(`document.getElementById('play').click()`);
    await until(`Number(document.getElementById('scrub').value)>2`);
    await evaluate(`document.getElementById('play').click()`);
    console.log('PASS playback continues after visibility is restored');
    await evaluate(`Object.assign(document.getElementById('scene').style,{width:'60px',height:'90px',minHeight:'0'}); new Promise(resolve=>setTimeout(resolve,100));`);
    await evaluate(`Object.assign(document.getElementById('scene').style,{width:'',height:'',minHeight:''}); new Promise(resolve=>setTimeout(resolve,100));`);
    await check('route redraws after a very small embedded viewport', hasWheelTrails);
    assert.deepEqual(errors, [], 'No uncaught browser exceptions');
    console.log('PASS zero uncaught browser exceptions; file:// direct opening verified');
  } finally {
    if (!exited) {
      try { await call('Browser.close'); } catch { /* Exit can precede its protocol response. */ }
      if (!exited) chrome.kill('SIGTERM');
      if (!exited) await new Promise(resolve => chrome.once('exit', resolve));
    }
    // profile is the exact directory returned by mkdtemp, guarded before cleanup.
    assert.ok(profile.startsWith(path.join(os.tmpdir(), 'sidecar-browser-')));
    assert.equal(await fs.realpath(profile), path.resolve(profile));
    await fs.rm(profile, { recursive: true, force: true });
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
