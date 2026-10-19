/* Browser UI; no network requests or external dependencies. */
(() => {
  'use strict';
  const M = window.Slalom, $ = id => document.getElementById(id);
  const colors = { rear: '#20745b', front: '#4381ce', side: '#db8a39', ink: '#294238', danger: '#be4e3e' };
  let result = null, playing = false, distance = 0, camera = 'overview', dirty = false, busy = false;
  let lastFrame = 0, sceneWidth = 0, sceneHeight = 0, staticDirty = true;
  const backdrop = document.createElement('canvas');
  const fields = [
    ['poleCount', '桩杆数量', 2, 8, 1, '根', '依次交替绕过每根杆'],
    ['spacing', '相邻桩距', 2.5, 9, 0.1, 'm', '测量杆中心到中心'],
    ['laneWidth', '场地净宽', 2, 8, 0.1, 'm', '左右边线内缘的距离'],
    ['margin', '预留安全余量', 0.02, 0.5, 0.01, 'm', '全车实体到杆表面、边线的下限'],
    ['speed', '模拟车速', 1, 8, 0.5, 'km/h', '恒定后轮速度，影响转把速率与用时'],
    ['approach', '首尾引入长度', 4, 12, 0.5, 'm', '后轮起点至首杆、末杆至后轮终点'],
    ['poleRadius', '桩杆半径', 0.015, 0.15, 0.005, 'm', '杆径的一半，例如直径7cm填0.035']
  ];
  const vehicleFields = [
    ['wheelbase', '前后轴距', 0.8, 2.6, 0.01, 'm', '前轮与后轮接地点的纵向距离'],
    ['track', '主车—边轮轮距', 0.65, 1.8, 0.01, 'm', '后轮与边斗轮中心的横向距离'],
    ['sideLead', '边轮前移量', 0, 0.8, 0.01, 'm', '边轮比后轮靠前多少'],
    ['handleWidth', '车把全宽', 0.5, 1.5, 0.01, 'm', '含把端；把端另按3.5cm半径包络'],
    ['handleX', '车把轴距后轮', 0.3, 2.5, 0.01, 'm', '转向枢轴沿车身纵向的位置'],
    ['ratio', '前轮角 ÷ 车把角', 0.5, 2, 0.05, '', '默认1:1；按实车测量校准'],
    ['maxSteer', '前轮最大转角', 15, 55, 1, '°', '左右对称限位的简化；取较小实测值'],
    ['maxRate', '最大转把速率', 15, 180, 5, '°/s', '限制车把每秒改变的角度'],
    ['front', '前轮前悬', 0.1, 1, 0.01, 'm', '主车身最前端超过前轮的长度'],
    ['rear', '后轮后悬', 0.25, 1.2, 0.01, 'm', '主车身最后端至后轮的长度'],
    ['bikeWidth', '主车身包络宽', 0.25, 0.9, 0.01, 'm', '含脚踏、护杠及骑手占用；不含车把'],
    ['sideWidth', '边斗包络宽', 0.35, 1, 0.01, 'm', '含边斗外侧翼子板等突出部件'],
    ['sideFront', '边斗前端距后轮', 0.6, 2.7, 0.01, 'm', '沿主车纵向，后轮向前量至边斗前端'],
    ['sideRear', '边斗后端距后轮', 0.1, 1.2, 0.01, 'm', '沿主车纵向，后轮向后量至边斗后端'],
    ['tireRadius', '轮胎平面半长', 0.15, 0.45, 0.01, 'm', '三轮均用相同保守矩形，取较大值'],
    ['tireWidth', '轮胎宽', 0.08, 0.25, 0.01, 'm', '三轮取较大的轮胎宽度']
  ];
  const labels = Object.fromEntries([...fields, ...vehicleFields].map(f => [f[0], f[1]]));
  function addFields(parent, specs) {
    for (const [key, name, min, max, step, unit, hint] of specs) {
      const label = document.createElement('label'); label.className = 'field';
      const title = document.createElement('span'); title.className = 'field-name'; title.textContent = name + (unit ? ` · ${unit}` : '');
      const input = document.createElement('input'); input.type = 'number'; input.id = `param-${key}`; input.name = key;
      Object.assign(input, { min, max, step, required: true, value: M.defaults[key] });
      const help = document.createElement('small'); help.className = 'field-help'; help.textContent = hint;
      label.append(title, input, help); parent.append(label);
    }
  }
  function addSelect(key, name, options) {
    const label = document.createElement('label'); label.className = 'field';
    const text = document.createElement('span'); text.className = 'field-name'; text.textContent = name;
    const select = document.createElement('select'); select.name = key; select.id = `param-${key}`;
    for (const [value, title] of options) { const option = document.createElement('option'); option.value = value; option.textContent = title; select.append(option); }
    select.value = M.defaults[key]; label.append(text, select); $('field-basic').append(label);
  }
  addFields($('field-basic'), fields);
  addSelect('side', '边斗位于', [[-1, '右侧边斗'], [1, '左侧边斗']]);
  addSelect('first', '第1杆从哪侧过', [[0, '自动比较左右起绕'], [1, '从杆左侧过'], [-1, '从杆右侧过']]);
  addFields($('field-vehicle'), vehicleFields);
  function parameters() {
    const p = { ...M.defaults };
    for (const control of $('parameters').elements) if (control.name) p[control.name] = Number(control.value);
    return M.validate(p);
  }
  function fill(p) {
    for (const control of $('parameters').elements) if (control.name) control.value = p[control.name];
  }
  try { const saved = JSON.parse(localStorage.getItem('sidecar-slalom-v1')); if (saved) fill(M.validate(saved)); } catch { /* Storage is optional on file URLs. */ }
  function enablePlayback(enable) { for (const id of ['play', 'restart', 'scrub', 'export']) $(id).disabled = !enable; }
  function markDirty() {
    if (busy) return;
    dirty = true; playing = false; $('play').textContent = '播放线路'; enablePlayback(false);
    $('result-badge').textContent = '参数待计算'; $('result-badge').dataset.state = 'pending';
    $('status').textContent = '参数已改变，请重新计算。';
    $('recommendation').textContent = '参数已改变，旧线路不再适用。计算后查看新的逐杆操作和净距。';
    $('pole-table').replaceChildren();
    for (const id of ['summary-clearance', 'summary-angle', 'summary-length', 'summary-time']) $(id).textContent = '—';
    staticDirty = true;
  }
  $('parameters').addEventListener('input', markDirty);
  $('parameters').addEventListener('submit', event => { event.preventDefault(); calculate(); });
  $('reset').addEventListener('click', () => { fill(M.defaults); calculate(); });
  async function calculate() {
    if (busy) return;
    let p;
    try { p = parameters(); } catch (error) { showError(error.message); return; }
    busy = true; dirty = true; playing = false; enablePlayback(false); $('error').hidden = true;
    $('parameters').setAttribute('aria-busy', 'true');
    for (const el of $('parameters').elements) el.disabled = true;
    $('calculate').textContent = '正在计算…'; $('result-badge').textContent = '搜索与扫掠校验中';
    $('result-badge').dataset.state = 'pending';
    try {
      result = await M.optimize(p, { onProgress: ({ tested, stage }) => {
        $('status').textContent = `${stage === 'verify' ? '加密验证全车扫掠' : stage === 'refine' ? '细化候选线路' : '比较左右非对称线路'} · 已检查 ${tested.toLocaleString()} 个候选`;
      } });
      distance = 0; dirty = false; staticDirty = true;
      try { localStorage.setItem('sidecar-slalom-v1', JSON.stringify(p)); } catch { /* Optional. */ }
      showResult(); enablePlayback(true);
      $('play').disabled = !result.feasible;
      $('play').textContent = result.feasible ? '播放线路' : '未找到可行线路';
      $('status').textContent = result.feasible ? '计算完成。拖动进度可观察每根杆附近的车把与轮差。' : '当前搜索未找到符合全部约束的线路；图中为诊断候选，请调整参数后重算。';
    } catch (error) { showError(error.message); }
    finally {
      busy = false; $('parameters').removeAttribute('aria-busy');
      for (const el of $('parameters').elements) el.disabled = false;
      $('calculate').textContent = '计算推荐线路';
    }
  }
  function showError(message) {
    $('error').textContent = message.replace(/\b[a-zA-Z]+\b/g, key => labels[key] || key); $('error').hidden = false;
    markDirty();
  }
  const cm = value => Number.isFinite(value) ? `${(value * 100).toFixed(1)} cm` : '—';
  const deg = value => `${Math.abs(value).toFixed(1)}°`;
  const turn = value => Math.abs(value) < 0.5 ? '回正' : `${value > 0 ? '左' : '右'} ${deg(value)}`;
  function showResult() {
    const r = result, p = r.parameters;
    $('result-badge').textContent = r.feasible ? '找到可行候选' : '当前候选未通过';
    $('result-badge').dataset.state = r.feasible ? 'success' : 'danger';
    $('summary-clearance').textContent = cm(r.clearance);
    $('summary-angle').textContent = `${r.maxSteer.toFixed(1)}°`;
    $('summary-length').textContent = `${r.length.toFixed(2)} m`;
    $('summary-time').textContent = `${r.duration.toFixed(1)} s`;
    const first = r.crossings[0];
    const box = $('recommendation'); box.replaceChildren();
    const intro = document.createElement('p');
    intro.textContent = r.feasible
      ? `推荐第1杆从${first.side}侧过，再逐杆交替。后轮入场位置在桩列中线${r.q.bias >= 0 ? '左' : '右'}侧 ${Math.abs(r.q.bias).toFixed(2)} m，车头回正。以下距离均以杆中心所在横截面为准。`
      : '这组参数下尚未找到可推荐的线路。当前显示最接近约束的诊断候选，不能按它实车练习。';
    box.append(intro);
    const list = document.createElement('ol');
    const reasons = [];
    if (r.minPole <= p.margin) reasons.push(`杆距净空仅 ${cm(r.minPole)}，不足设定 ${cm(p.margin)}`);
    if (r.minEdge <= p.margin) reasons.push(`边线净空仅 ${cm(r.minEdge)}，不足设定 ${cm(p.margin)}`);
    if (r.certifiedSteer > p.maxSteer) reasons.push(`前轮转角上界 ${r.certifiedSteer.toFixed(1)}°，超过限位 ${p.maxSteer}°`);
    if (r.certifiedRate > p.maxRate) reasons.push(`转把速率上界 ${r.certifiedRate.toFixed(1)}°/s，超过 ${p.maxRate}°/s；可尝试降低速度`);
    if (r.gateSlack <= 0) reasons.push('没有按指定侧交替绕过全部桩杆');
    if (r.sideForward <= 0) reasons.push('内侧边轮进入零纵速或倒向的极端运动区');
    const tips = r.feasible ? [
      `首杆前轮过杆时，后轮仍在杆前 ${(first.poleX - first.front.rearX).toFixed(2)} m；此刻车把为${turn(first.front.handle)}。不要把前轮过杆当作整车已过杆。`,
      `全程最小杆净距 ${cm(r.minPole)}（第 ${r.poleEvent.index + 1} 杆附近，${r.poleEvent.part}）；最小边线净距 ${cm(r.minEdge)}（${r.edgeEvent.name}，${r.edgeEvent.part}）。两者均已扣除连续扫掠余量。`,
      `看后轮绿线控制主车位置，同时留意橙色边斗轮与把端。用“跟车放大”及下表逐杆检查；车把沿曲线连续变化，不存在通用于所有车的固定打把点。`
    ] : [...reasons, '可尝试增加桩距或场宽；若只超转把速率，可先降低车速。搜索失败不代表场地必定无解。'];
    for (const text of tips) { const li = document.createElement('li'); li.textContent = text; list.append(li); }
    box.append(list);
    $('pole-table').replaceChildren();
    for (const row of r.crossings) {
      const tr = document.createElement('tr');
      const values = [`${row.index + 1} 号杆`, `${row.side}侧`, row.front ? `${(row.poleX - row.front.rearX).toFixed(2)} m` : '未识别',
        row.front ? turn(row.front.handle) : '—', row.rear ? `${row.rear.lateral > 0 ? '左' : '右'} ${Math.abs(row.rear.lateral).toFixed(2)} m` : '—', cm(row.clearance)];
      for (const value of values) { const td = document.createElement('td'); td.textContent = value; tr.append(td); }
      if (row.clearance <= p.margin) tr.classList.add('unsafe');
      tr.tabIndex = 0; tr.title = '点击跳转到该杆最窄处';
      const jump = () => { distance = M.clamp(distanceAtX(row.x), 0, r.length); playing = false; $('play').textContent = r.feasible ? '播放线路' : '未找到可行线路'; staticDirty = true; };
      tr.addEventListener('click', jump); tr.addEventListener('keydown', e => { if (e.key === 'Enter') jump(); });
      $('pole-table').append(tr);
    }
    $('search-info').textContent = `搜索 ${r.tested.toLocaleString()} 个候选；后轮曲线 y=b+A·w(x)·[cos(t)+h·cos(3t)]/(1+h)，t=π(x−首杆+相位)/桩距。已找到最佳候选：偏置 ${r.q.bias.toFixed(3)} m，幅度 ${r.q.amplitude.toFixed(3)} m，相位 ${r.q.phase.toFixed(3)} m，形状 ${r.q.shape.toFixed(4)}。加密检查 ${r.sampleCount.toLocaleString()} 点，步长≤${(r.dx * 1000).toFixed(2)} mm，连续净距扣减 ${cm(r.guard)}。前轮转角上界 ${r.certifiedSteer.toFixed(2)}°，转把速率上界 ${r.certifiedRate.toFixed(2)}°/s。优先最大化最窄净距，近似同等时偏好短线路与小转角；不保证全局最优。`;
  }
  function distanceAtX(x) {
    const a = result.samples;
    let lo = 0, hi = a.length - 1;
    while (hi - lo > 1) { const m = (lo + hi) >> 1; if (a[m].x < x) lo = m; else hi = m; }
    return a[lo].distance + (a[hi].distance - a[lo].distance) * M.clamp((x - a[lo].x) / (a[hi].x - a[lo].x), 0, 1);
  }
  $('play').addEventListener('click', () => {
    if (!result || dirty || !result.feasible) return;
    if (distance >= result.length) distance = 0;
    playing = !playing; $('play').textContent = playing ? '暂停回放' : '播放线路';
  });
  $('restart').addEventListener('click', () => { distance = 0; playing = false; $('play').textContent = result.feasible ? '播放线路' : '未找到可行线路'; });
  $('scrub').addEventListener('input', () => { if (result && !dirty) { distance = Number($('scrub').value) / 1000 * result.length; playing = false; $('play').textContent = result.feasible ? '播放线路' : '未找到可行线路'; } });
  for (const view of ['overview', 'follow']) $(`view-${view}`).addEventListener('click', () => {
    camera = view; staticDirty = true;
    for (const v of ['overview', 'follow']) { $(`view-${v}`).setAttribute('aria-pressed', String(v === view)); $(`view-${v}`).classList.toggle('active', v === view); }
  });
  for (const id of ['show-envelope', 'show-trails']) $(id).addEventListener('change', () => { staticDirty = true; });
  document.addEventListener('keydown', event => {
    if (/INPUT|SELECT|TEXTAREA|BUTTON/.test(event.target.tagName) || dirty || !result) return;
    if (event.code === 'Space') { event.preventDefault(); $('play').click(); }
    if (['ArrowLeft', 'ArrowRight'].includes(event.key)) {
      event.preventDefault(); distance = M.clamp(distance + (event.key === 'ArrowRight' ? 1 : -1) * result.length / 100, 0, result.length); playing = false; $('play').textContent = result.feasible ? '播放线路' : '未找到可行线路';
    }
  });
  $('export').addEventListener('click', () => {
    if (!result || dirty) return;
    const report = { format: 'sidecar-slalom-v1', createdAt: new Date().toISOString(),
      status: result.feasible ? 'feasible-candidate' : 'diagnostic-only',
      model: 'Low-speed rear-no-slip bicycle kinematics with rigid sidecar and articulated handlebar; fixed side wheel requires scrub.',
      limits: 'Illustrative parameters, not surveyed. No tire-force, suspension, tipping or global-optimality claim. Boundary clearances and steering bounds include continuous interval guards.',
      ...result };
    const url = URL.createObjectURL(new Blob([JSON.stringify(report, null, 2)], { type: 'application/json' }));
    const a = document.createElement('a'); a.href = url; a.download = 'sidecar-route.json'; a.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
  });
  function resize() {
    const canvas = $('scene'), rect = canvas.getBoundingClientRect(), dpr = Math.min(devicePixelRatio || 1, 2);
    sceneWidth = rect.width; sceneHeight = rect.height;
    canvas.width = Math.round(rect.width * dpr); canvas.height = Math.round(rect.height * dpr);
    backdrop.width = canvas.width; backdrop.height = canvas.height;
    staticDirty = true;
  }
  new ResizeObserver(resize).observe($('scene'));
  function projectFor(pose) {
    const p = result.parameters, w = sceneWidth, h = sceneHeight;
    const scale = camera === 'overview' ? Math.min((w - 90) / result.c.length, (h - 180) / (p.laneWidth + 1.5)) : Math.min((w - 70) / 10, (h - 110) / (p.laneWidth + 1.5));
    const center = camera === 'overview' ? result.c.length / 2 : pose.x + 1.5;
    const at = point => ({ x: w / 2 + (point.x - center) * scale, y: h * 0.44 - point.y * scale });
    return { at, scale, minX: center - w / (2 * scale), maxX: center + w / (2 * scale) };
  }
  function line(ctx, a, b, color, width = 1, dash = []) {
    ctx.beginPath(); ctx.strokeStyle = color; ctx.lineWidth = width; ctx.setLineDash(dash); ctx.moveTo(a.x, a.y); ctx.lineTo(b.x, b.y); ctx.stroke(); ctx.setLineDash([]);
  }
  function text(ctx, value, x, y, color = colors.ink, size = 12, align = 'left') {
    ctx.fillStyle = color; ctx.font = `${size}px system-ui, sans-serif`; ctx.textAlign = align; ctx.fillText(value, x, y);
  }
  function drawShape(ctx, shape, projection, fill, stroke, alpha = 1) {
    const { at, scale } = projection;
    ctx.globalAlpha = alpha;
    if (shape.kind === 'capsule') {
      const a = at(shape.a), b = at(shape.b); ctx.lineCap = 'round';
      line(ctx, a, b, stroke || fill, Math.max(1, shape.radius * 2 * scale)); ctx.lineCap = 'butt';
    } else {
      const vertices = M.corners(shape).map(at); ctx.beginPath(); ctx.moveTo(vertices[0].x, vertices[0].y);
      for (const v of vertices.slice(1)) ctx.lineTo(v.x, v.y); ctx.closePath();
      if (fill) { ctx.fillStyle = fill; ctx.fill(); }
      if (stroke) { ctx.strokeStyle = stroke; ctx.lineWidth = 1; ctx.stroke(); }
    }
    ctx.globalAlpha = 1;
  }
  function drawCar(ctx, pose, projection, annotate = false) {
    const p = result.parameters, shapes = M.geometry(pose, p);
    for (const shape of shapes.slice(6)) drawShape(ctx, shape, projection, '#8b9a8e');
    drawShape(ctx, shapes[0], projection, '#d9e7dc', '#466c58');
    drawShape(ctx, shapes[1], projection, '#e2e5ce', '#778568');
    for (let i = 2; i <= 4; i++) drawShape(ctx, shapes[i], projection, '#34483e', '#f8f6e9');
    drawShape(ctx, shapes[5], projection, '#de8539');
    const head = projection.at(M.transform(pose, p.wheelbase + p.front - 0.1, 0));
    ctx.beginPath(); ctx.fillStyle = '#ffca6b'; ctx.arc(head.x, head.y, Math.max(1.8, projection.scale * 0.09), 0, Math.PI * 2); ctx.fill();
    for (const [x,y,color,name] of [[0,0,colors.rear,'后'], [p.wheelbase,0,colors.front,'前'], [p.sideLead,p.side*p.track,colors.side,'边']]) {
      const point = projection.at(M.transform(pose, x, y)); ctx.beginPath(); ctx.fillStyle = color;
      ctx.arc(point.x, point.y, Math.max(2, projection.scale * 0.05), 0, Math.PI * 2); ctx.fill();
      if (annotate) text(ctx, name, point.x, point.y - 13, color, 11, 'center');
    }
  }
  function drawStatic(ctx, projection) {
    const { at, scale, minX, maxX } = projection, p = result.parameters, c = result.c;
    ctx.fillStyle = '#f3f4eb'; ctx.fillRect(0, 0, sceneWidth, sceneHeight);
    for (let x = Math.floor(minX); x <= maxX; x++) line(ctx, at({ x, y: 15 }), at({ x, y: -15 }), '#e4e8dc');
    for (let y = -15; y <= 15; y++) line(ctx, at({ x: minX, y }), at({ x: maxX, y }), '#e4e8dc');
    const tl = at({ x: 0, y: p.laneWidth / 2 }), br = at({ x: c.length, y: -p.laneWidth / 2 });
    ctx.fillStyle = '#fffef7'; ctx.fillRect(tl.x, tl.y, br.x - tl.x, br.y - tl.y);
    for (const sign of [-1, 1]) {
      line(ctx, at({ x:0, y:sign*p.laneWidth/2 }), at({ x:c.length, y:sign*p.laneWidth/2 }), '#516957', 2);
      line(ctx, at({ x:0, y:sign*(p.laneWidth/2-p.margin) }), at({ x:c.length, y:sign*(p.laneWidth/2-p.margin) }), '#dfc99e', 1, [5,5]);
    }
    line(ctx, at({ x:0, y:0 }), at({ x:c.length, y:0 }), '#bdc8bb', 1, [6,8]);
    line(ctx, tl, at({x:0,y:-p.laneWidth/2}), '#7a8c7a', 1, [4,5]);
    line(ctx, br, at({x:c.length,y:p.laneWidth/2}), '#7a8c7a', 1, [4,5]);
    const upper = at({ x: Math.max(0.4, minX+0.4), y:p.laneWidth/2 });
    text(ctx, '左边线', upper.x, upper.y-12, '#687d68', 11);
    text(ctx, '右边线', upper.x, br.y+20, '#687d68', 11);
    if ($('show-envelope').checked) {
      for (let i = 0; i < result.samples.length; i += 9) {
        const pose = result.samples[i];
        if (pose.x < minX-c.reach-2 || pose.x > maxX+c.tail+2) continue;
        for (const shape of M.geometry(pose, p)) drawShape(ctx, shape, projection, result.feasible ? '#639a78' : '#c7806d', null, 0.035);
      }
    }
    if ($('show-trails').checked) for (const [ox,oy,color,dash] of [[p.wheelbase,0,colors.front,[5,4]], [0,0,colors.rear,[]], [p.sideLead,p.side*p.track,colors.side,[2,4]]]) {
      ctx.beginPath(); ctx.strokeStyle=color; ctx.lineWidth=1.7; ctx.setLineDash(dash);
      let begun=false;
      for (const pose of result.samples) {
        if (pose.x < minX-c.reach-2 || pose.x > maxX+c.tail+2) { begun=false; continue; }
        const point=at(M.transform(pose,ox,oy)); if (!begun) ctx.moveTo(point.x,point.y); else ctx.lineTo(point.x,point.y); begun=true;
      }
      ctx.stroke(); ctx.setLineDash([]);
    }
    for (const rod of c.poles) {
      const center=at(rod); if(center.x < -30 || center.x > sceneWidth+30) continue;
      ctx.beginPath(); ctx.fillStyle='#d98536'; ctx.arc(center.x,center.y,p.poleRadius*scale,0,Math.PI*2); ctx.fill();
      ctx.beginPath(); ctx.strokeStyle='#bd7944'; ctx.lineWidth=1.2; ctx.arc(center.x,center.y,7,0,Math.PI*2); ctx.stroke();
      const label=at({x:rod.x,y:p.laneWidth/2}); line(ctx,{x:center.x,y:label.y-7},{x:center.x,y:center.y-10},'#d6d7c5',1,[3,5]);
      text(ctx,`${rod.index+1}号杆`,center.x,label.y-24,'#8f673e',12,'center');
      if(rod.index < c.poles.length-1) {
        const next=at(c.poles[rod.index+1]), y=br.y+45;
        line(ctx,{x:center.x,y},{x:next.x,y},'#a9b5a1');
        line(ctx,{x:center.x,y:y-4},{x:center.x,y:y+4},'#a9b5a1'); line(ctx,{x:next.x,y:y-4},{x:next.x,y:y+4},'#a9b5a1');
        text(ctx,`${p.spacing.toFixed(1)} m`,(center.x+next.x)/2,y-7,'#7c8974',11,'center');
      }
    }
    const a=at({x:c.start,y:0}); if (a.x>5) text(ctx,'入口 →',a.x,br.y+45,'#6c7e68',11,'center');
    const compact = sceneWidth < 560;
    text(ctx, compact ? `净宽 ${p.laneWidth.toFixed(2)} m · 余量 ${cm(p.margin)}` : `净宽 ${p.laneWidth.toFixed(2)} m · 虚线内缩 ${cm(p.margin)}`,18,25,'#61745f',compact ? 10 : 12);
    text(ctx,'行进 →   上方为车的左侧',compact ? 18 : sceneWidth-18,compact ? 43 : 25,'#61745f',compact ? 9 : 11,compact ? 'left' : 'right');
    const barLength=scale*2; line(ctx,{x:18,y:sceneHeight-22},{x:18+barLength,y:sceneHeight-22},'#526a55',2);
    text(ctx,'2 m',18+barLength/2,sceneHeight-30,'#61745f',11,'center');
    text(ctx,'杆外圈为定位标记；实际碰撞按输入半径计算',sceneWidth-18,sceneHeight-16,'#87917e',10,'right');
  }
  function drawInset(ctx, pose) {
    if(sceneWidth<560 || camera==='follow') return;
    const w=232,h=142,x=sceneWidth-w-18,y=sceneHeight-h-45;
    ctx.fillStyle='rgba(255,254,247,.97)'; ctx.fillRect(x,y,w,h); ctx.strokeStyle='#dce2d2'; ctx.strokeRect(x,y,w,h);
    text(ctx,'车把 / 轮组放大',x+12,y+20,'#687c67',11);
    const p=result.parameters, scale=Math.min(62,(w-35)/(result.c.reach+result.c.tail),(h-35)/(p.track+p.sideWidth+p.handleWidth/2));
    const center={x:x+60,y:y+55+(p.side===1?45:0)};
    drawCar(ctx,{...pose,x:0,y:0,theta:0},{scale,at:point=>({x:center.x+point.x*scale,y:center.y-point.y*scale})},true);
    text(ctx,`轴距 ${p.wheelbase.toFixed(2)} m / 轮距 ${p.track.toFixed(2)} m`,x+12,y+h-7,'#89947f',9);
  }
  function drawSteering(pose) {
    const canvas=$('steering'), rect=canvas.getBoundingClientRect(), dpr=Math.min(devicePixelRatio||1,2);
    if(canvas.width!==Math.round(rect.width*dpr)||canvas.height!==Math.round(rect.height*dpr)){canvas.width=Math.round(rect.width*dpr);canvas.height=Math.round(rect.height*dpr);}
    const ctx=canvas.getContext('2d'); ctx.setTransform(dpr,0,0,dpr,0,0); ctx.clearRect(0,0,rect.width,rect.height);
    const cx=rect.width/2,cy=rect.height/2+6,r=Math.min(rect.width*0.36,rect.height*0.39);
    ctx.beginPath();ctx.strokeStyle='#d8dfcf';ctx.lineWidth=1;ctx.arc(cx,cy,r,-Math.PI,0);ctx.stroke();
    line(ctx,{x:cx,y:cy-8},{x:cx,y:cy-r-5},'#a5b89e',1,[3,4]);
    // Front of the vehicle is up in this small gauge; positive is left steer.
    const co=Math.cos(pose.handle),si=Math.sin(pose.handle);
    line(ctx,{x:cx-r*co,y:cy+r*si},{x:cx+r*co,y:cy-r*si},colors.side,5);
    line(ctx,{x:cx,y:cy},{x:cx-Math.sin(pose.steer)*r*.7,y:cy-Math.cos(pose.steer)*r*.7},colors.front,3);
    ctx.beginPath();ctx.fillStyle=colors.ink;ctx.arc(cx,cy,4,0,Math.PI*2);ctx.fill();
    text(ctx,'左',7,rect.height-7,'#839078',10);text(ctx,'右',rect.width-7,rect.height-7,'#839078',10,'right');
  }
  function frame(timestamp) {
    const elapsed=lastFrame?Math.min((timestamp-lastFrame)/1000,0.1):0;lastFrame=timestamp;
    if(result && !dirty) {
      if(playing) {
        distance=Math.min(result.length,distance+elapsed*result.parameters.speed/3.6*Number($('playback').value));
        if(distance>=result.length){playing=false;$('play').textContent='重新播放';}
      }
      const pose=M.poseAtDistance(result,distance), projection=projectFor(pose), canvas=$('scene'),ctx=canvas.getContext('2d');
      const dpr=canvas.width/sceneWidth;
      if(staticDirty||camera==='follow'){
        const bg=backdrop.getContext('2d');bg.setTransform(dpr,0,0,dpr,0,0);drawStatic(bg,projection);staticDirty=false;
      }
      ctx.setTransform(1,0,0,1,0,0);ctx.drawImage(backdrop,0,0);ctx.setTransform(dpr,0,0,dpr,0,0);
      drawCar(ctx,pose,projection,camera==='follow');drawInset(ctx,pose);
      const checks=M.inspectPose(pose,result.parameters,result.c);
      $('steer-value').textContent=turn(pose.handle/M.RAD);$('front-value').textContent=turn(pose.steer/M.RAD);
      $('pole-value').textContent=cm(checks.pole);$('edge-value').textContent=cm(checks.edge);
      const radius=Math.abs(pose.curvature)<1e-8?Infinity:1/Math.abs(pose.curvature);
      $('offtrack-value').textContent=cm(Number.isFinite(radius)?Math.hypot(radius,result.parameters.wheelbase)-radius:0);
      $('slip-value').textContent=deg(pose.slip/M.RAD);
      $('time').textContent=`${(distance/(result.parameters.speed/3.6)).toFixed(1)} / ${result.duration.toFixed(1)} s`;
      $('scrub').value=Math.round(distance/result.length*1000);
      drawSteering(pose);
      if(!result.feasible){text(ctx,'诊断候选 · 未通过约束',sceneWidth/2,sceneHeight-20,colors.danger,14,'center');}
    } else {
      const ctx=$('scene').getContext('2d'),dpr=$('scene').width/sceneWidth;
      ctx.setTransform(dpr,0,0,dpr,0,0);ctx.fillStyle='#f3f4eb';ctx.fillRect(0,0,sceneWidth,sceneHeight);
      text(ctx,busy?'正在搜索线路并检查整车扫掠…':'参数已改变 · 请计算新的线路',sceneWidth/2,sceneHeight/2,'#75836e',15,'center');
      for (const id of ['steer-value','front-value','pole-value','edge-value','offtrack-value','slip-value']) $(id).textContent='—';
    }
    requestAnimationFrame(frame);
  }
  resize(); requestAnimationFrame(frame); calculate();
})();
