/* Low-speed geometry: +x forward, +y left; rear contact is the origin. */
(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  else root.Slalom = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';
  const RAD = Math.PI / 180;
  const defaults = Object.freeze({
    wheelbase: 1.5, front: 0.38, rear: 0.65, bikeWidth: 0.44,
    track: 1.12, sideLead: 0.25, sideWidth: 0.64, sideFront: 1.55,
    sideRear: 0.5, handleWidth: 0.94, handleX: 1.15, ratio: 1,
    maxSteer: 40, maxRate: 85, tireRadius: 0.3, tireWidth: 0.13,
    side: -1, poleCount: 4, spacing: 5, laneWidth: 5.2,
    poleRadius: 0.035, margin: 0.12, approach: 9, speed: 3, first: 0
  });
  const clamp = (x, lo, hi) => Math.max(lo, Math.min(hi, x));
  function validate(input) {
    const p = { ...defaults, ...input };
    for (const key of Object.keys(defaults)) if (!Number.isFinite(p[key])) throw new Error(`${key} 必须是有限数值`);
    const bounds = {
      wheelbase: [0.8, 2.6], front: [0.1, 1], rear: [0.25, 1.2], bikeWidth: [0.25, 0.9],
      track: [0.65, 1.8], sideLead: [0, 0.8], sideWidth: [0.35, 1],
      sideFront: [0.6, 2.7], sideRear: [0.1, 1.2], handleWidth: [0.5, 1.5],
      handleX: [0.3, 2.5], ratio: [0.5, 2], maxSteer: [15, 55], maxRate: [15, 180],
      tireRadius: [0.15, 0.45], tireWidth: [0.08, 0.25], poleCount: [2, 8],
      spacing: [2.5, 9], laneWidth: [2, 8], poleRadius: [0.015, 0.15],
      margin: [0.02, 0.5], approach: [4, 12], speed: [1, 8]
    };
    for (const [key, [lo, hi]] of Object.entries(bounds)) {
      if (p[key] < lo || p[key] > hi) throw new Error(`${key} 超出范围 ${lo}–${hi}`);
    }
    if (!Number.isInteger(p.poleCount) || ![-1, 1].includes(p.side) || ![-1, 0, 1].includes(p.first)) throw new Error('桩数或方向参数无效');
    if (p.approach - 0.35 * p.spacing < 1) throw new Error('直线引入段过短，请增加首尾引入长度');
    if (p.handleX > p.wheelbase + p.front) throw new Error('车把轴位置不能超过车头');
    if (p.track <= (p.bikeWidth + p.sideWidth) / 2) throw new Error('轮距过小，边斗与主车身重叠');
    return p;
  }
  function course(p) {
    const reach = Math.max(p.wheelbase + p.front, p.sideFront, p.handleX + p.handleWidth / 2, p.wheelbase + p.tireRadius);
    const tail = Math.max(p.rear, p.sideRear, p.tireRadius);
    const start = tail + p.margin + 0.3;
    const poles = Array.from({ length: p.poleCount }, (_, i) => ({ x: start + p.approach + i * p.spacing, y: 0, index: i }));
    const end = poles.at(-1).x + p.approach;
    return { poles, start, end, length: end + reach + p.margin + 0.3, reach, tail };
  }
  function ramp(t, width) {
    if (t <= 0) return [0, 0, 0, 0];
    if (t >= 1) return [1, 0, 0, 0];
    return [t ** 3 * (10 - 15 * t + 6 * t * t),
      30 * t * t * (1 - t) ** 2 / width,
      60 * t * (1 - t) * (1 - 2 * t) / width ** 2,
      (60 - 360 * t + 360 * t * t) / width ** 3];
  }
  // C2 straight entry/exit, cosine + third harmonic in the slalom zone.
  function pathAt(x, q, p, c) {
    const k = Math.PI / p.spacing;
    const t = k * (x - c.poles[0].x + q.phase);
    const den = 1 + q.shape, a = q.amplitude * q.first;
    const g = (Math.cos(t) + q.shape * Math.cos(3 * t)) / den;
    const g1 = -k * (Math.sin(t) + 3 * q.shape * Math.sin(3 * t)) / den;
    const g2 = -k * k * (Math.cos(t) + 9 * q.shape * Math.cos(3 * t)) / den;
    const g3 = k ** 3 * (Math.sin(t) + 27 * q.shape * Math.sin(3 * t)) / den;
    const width = p.approach - 0.35 * p.spacing;
    let w;
    if (x < c.poles[0].x - 0.35 * p.spacing) w = ramp((x - c.start) / width, width);
    else if (x > c.poles.at(-1).x + 0.35 * p.spacing) {
      w = ramp((c.end - x) / width, width); w[1] *= -1; w[3] *= -1;
    } else w = [1, 0, 0, 0];
    const y = q.bias + a * w[0] * g;
    const d1 = a * (w[1] * g + w[0] * g1);
    const d2 = a * (w[2] * g + 2 * w[1] * g1 + w[0] * g2);
    const d3 = a * (w[3] * g + 3 * w[2] * g1 + 3 * w[1] * g2 + w[0] * g3);
    const norm = Math.sqrt(1 + d1 * d1), curvature = d2 / norm ** 3;
    const dkdx = d3 / norm ** 3 - 3 * d1 * d2 * d2 / norm ** 5;
    const steer = Math.atan(p.wheelbase * curvature);
    const steerDx = p.wheelbase * dkdx / (1 + (p.wheelbase * curvature) ** 2);
    return { x, y, theta: Math.atan(d1), curvature, steer, handle: steer / p.ratio,
      handleRate: steerDx * (p.speed / 3.6) / norm / p.ratio, norm,
      slip: Math.atan2(curvature * p.sideLead, 1 - curvature * p.side * p.track) };
  }
  function transform(pose, x, y) {
    const co = Math.cos(pose.theta), si = Math.sin(pose.theta);
    return { x: pose.x + co * x - si * y, y: pose.y + si * x + co * y };
  }
  function rect(pose, x, y, halfX, halfY, angle, name) {
    const center = transform(pose, x, y);
    return { kind: 'rect', ...center, halfX, halfY, angle: pose.theta + angle, name };
  }
  function geometry(pose, p) {
    const a = transform(pose, p.handleX - Math.sin(pose.handle) * p.handleWidth / 2, Math.cos(pose.handle) * p.handleWidth / 2);
    const b = transform(pose, p.handleX + Math.sin(pose.handle) * p.handleWidth / 2, -Math.cos(pose.handle) * p.handleWidth / 2);
    return [
      rect(pose, (p.wheelbase + p.front - p.rear) / 2, 0, (p.wheelbase + p.front + p.rear) / 2, p.bikeWidth / 2, 0, '主车身'),
      rect(pose, (p.sideFront - p.sideRear) / 2, p.side * p.track, (p.sideFront + p.sideRear) / 2, p.sideWidth / 2, 0, '边斗'),
      rect(pose, 0, 0, p.tireRadius, p.tireWidth / 2, 0, '后轮'),
      rect(pose, p.wheelbase, 0, p.tireRadius, p.tireWidth / 2, pose.steer, '前轮'),
      rect(pose, p.sideLead, p.side * p.track, p.tireRadius, p.tireWidth / 2, 0, '边斗轮'),
      { kind: 'capsule', a, b, radius: 0.035, name: '车把端' },
      ...[0.2, Math.min(p.wheelbase * 0.65, p.sideFront - 0.1)].map(x => ({
        kind: 'capsule', a: transform(pose, x, p.side * p.bikeWidth / 2),
        b: transform(pose, x, p.side * (p.track - p.sideWidth / 2)), radius: 0.035, name: '连接架'
      }))
    ];
  }
  function corners(shape) {
    if (shape.kind === 'capsule') return [shape.a, shape.b];
    const co = Math.cos(shape.angle), si = Math.sin(shape.angle);
    return [[-1,-1], [1,-1], [1,1], [-1,1]].map(([a,b]) => ({
      x: shape.x + a * shape.halfX * co - b * shape.halfY * si,
      y: shape.y + a * shape.halfX * si + b * shape.halfY * co
    }));
  }
  function poleDistance(shape, pole) {
    if (shape.kind === 'capsule') {
      const dx = shape.b.x - shape.a.x, dy = shape.b.y - shape.a.y;
      const t = clamp(((pole.x - shape.a.x) * dx + (pole.y - shape.a.y) * dy) / (dx * dx + dy * dy), 0, 1);
      return Math.hypot(pole.x - shape.a.x - t * dx, pole.y - shape.a.y - t * dy) - shape.radius;
    }
    const dx = pole.x - shape.x, dy = pole.y - shape.y;
    const co = Math.cos(shape.angle), si = Math.sin(shape.angle);
    const a = Math.abs(co * dx + si * dy) - shape.halfX, b = Math.abs(-si * dx + co * dy) - shape.halfY;
    return Math.hypot(Math.max(a, 0), Math.max(b, 0)) + Math.min(Math.max(a, b), 0);
  }
  function inspectPose(pose, p, c) {
    let edge = Infinity, pole = Infinity, edgePart = '', polePart = '', edgeName = '', poleIndex = -1;
    const perPole = new Array(p.poleCount).fill(Infinity);
    for (const shape of geometry(pose, p)) {
      const vertices = corners(shape), radius = shape.radius || 0;
      for (const v of vertices) {
        for (const [distance, name] of [[p.laneWidth / 2 - v.y - radius, '左边线'], [p.laneWidth / 2 + v.y - radius, '右边线'], [v.x - radius, '起点边界'], [c.length - v.x - radius, '终点边界']]) {
          if (distance < edge) { edge = distance; edgePart = shape.name; edgeName = name; }
        }
      }
      for (const rod of c.poles) {
        const d = poleDistance(shape, rod) - p.poleRadius;
        perPole[rod.index] = Math.min(perPole[rod.index], d);
        if (d < pole) { pole = d; polePart = shape.name; poleIndex = rod.index; }
      }
    }
    return { edge, pole, edgePart, edgeName, polePart, poleIndex, perPole };
  }
  // Global derivative bounds certify the space between samples, not just frames.
  function derivativeBounds(q, p) {
    const k = Math.PI / p.spacing, s = Math.abs(q.shape), den = 1 + q.shape;
    const width = p.approach - 0.35 * p.spacing;
    const g0 = (1 + s) / den, g1 = k * (1 + 3 * s) / den;
    const g2 = k * k * (1 + 9 * s) / den, g3 = k ** 3 * (1 + 27 * s) / den;
    const w1 = 1.875 / width, w2 = 5.774 / width ** 2, w3 = 60 / width ** 3;
    const f1 = q.amplitude * (w1 * g0 + g1);
    const f2 = q.amplitude * (w2 * g0 + 2 * w1 * g1 + g2);
    const f3 = q.amplitude * (w3 * g0 + 3 * w2 * g1 + 3 * w1 * g2 + g3);
    // max |z|/(1+z²)^(5/2) < .286334.
    const curvatureDx = f3 + 0.859 * f2 * f2;
    const steerDx = p.wheelbase * curvatureDx;
    const tireDiagonal = Math.hypot(p.tireRadius, p.tireWidth / 2);
    const radius = Math.max(Math.hypot(p.wheelbase + p.front, p.bikeWidth / 2),
      Math.hypot(p.rear, p.bikeWidth / 2), Math.hypot(p.sideFront, p.track + p.sideWidth / 2),
      Math.hypot(p.sideRear, p.track + p.sideWidth / 2), p.wheelbase + tireDiagonal,
      Math.hypot(p.track, p.sideLead) + tireDiagonal, p.handleX + p.handleWidth / 2 + 0.035);
    const articulated = Math.max(tireDiagonal, p.handleWidth / (2 * p.ratio));
    const g4 = k ** 4 * (1 + 81 * s) / den, w4 = 360 / width ** 4;
    const f4 = q.amplitude * (w4 * g0 + 4 * w3 * g1 + 6 * w2 * g2 + 4 * w1 * g3 + g4);
    const curvatureDxx = f4 + 2.577 * f2 * f3 + 5 * f2 ** 3;
    const steerDxx = p.wheelbase * curvatureDxx + 0.65 * p.wheelbase ** 2 * curvatureDx ** 2;
    const rateDx = p.speed / 3.6 / p.ratio * (steerDxx + 0.385 * steerDx * f2);
    return { motion: Math.hypot(1, f1) + radius * f2 + articulated * steerDx, curvatureDx, rateDx };
  }
  function evaluate(q, p, opts = {}) {
    const c = course(p), bounds = opts.certify ? derivativeBounds(q, p) : null;
    const step = bounds ? Math.min(0.025, 0.01 / bounds.motion) : (opts.step || 0.16);
    const n = Math.min(200000, Math.ceil((c.end - c.start) / step)), dx = (c.end - c.start) / n;
    const guard = bounds ? bounds.motion * dx / 2 : 0;
    const curveGuard = bounds ? bounds.curvatureDx * dx / 2 : 0;
    const xs = Array.from({ length: n + 1 }, (_, i) => c.start + i * dx);
    for (const knot of [c.poles[0].x - 0.35 * p.spacing, c.poles.at(-1).x + 0.35 * p.spacing]) {
      xs.push(knot - 1e-9, knot, knot + 1e-9);
    }
    xs.push(c.start + 1e-9, c.end - 1e-9);
    xs.sort((a, b) => a - b);
    let minSideForward = Infinity;
    let minEdge = Infinity, minPole = Infinity, maxSteer = 0, maxRate = 0, length = 0, maxSlip = 0, maxCurvature = 0;
    let edgeEvent, poleEvent;
    const perPole = c.poles.map(() => ({ clearance: Infinity, x: 0, part: '' }));
    const samples = [];
    let lastNorm, lastX, lastSaved = -Infinity;
    for (let i = 0; i < xs.length; i++) {
      const pose = pathAt(xs[i], q, p, c), distances = inspectPose(pose, p, c);
      if (distances.edge < minEdge) { minEdge = distances.edge; edgeEvent = { x: pose.x, part: distances.edgePart, name: distances.edgeName }; }
      if (distances.pole < minPole) { minPole = distances.pole; poleEvent = { x: pose.x, part: distances.polePart, index: distances.poleIndex }; }
      for (let j = 0; j < perPole.length; j++) if (distances.perPole[j] < perPole[j].clearance) {
        perPole[j] = { clearance: distances.perPole[j], x: pose.x, part: distances.poleIndex === j ? distances.polePart : '' };
      }
      maxSteer = Math.max(maxSteer, Math.abs(pose.steer) / RAD);
      maxCurvature = Math.max(maxCurvature, Math.abs(pose.curvature));
      minSideForward = Math.min(minSideForward, 1 - pose.curvature * p.side * p.track);
      maxRate = Math.max(maxRate, Math.abs(pose.handleRate) / RAD);
      maxSlip = Math.max(maxSlip, Math.abs(pose.slip) / RAD);
      if (i > 0) length += (pose.x - lastX) * (lastNorm + pose.norm) / 2;
      lastNorm = pose.norm; lastX = pose.x;
      if (opts.record && (pose.x - lastSaved >= 0.025 || i === xs.length - 1)) {
        samples.push({ ...pose, distance: length, edge: distances.edge, pole: distances.pole }); lastSaved = pose.x;
      }
    }
    const certifiedSteer = bounds ? Math.atan(p.wheelbase * (maxCurvature + curveGuard)) / RAD : maxSteer + 0.3;
    const certifiedRate = bounds ? maxRate + bounds.rateDx * dx / (2 * RAD) : maxRate + 1;
    const clearance = Math.min(minEdge, minPole) - (bounds ? guard : 0.008);
    const gateSlack = Math.min(...c.poles.map((rod, i) => q.first * (-1) ** i * pathAt(rod.x, q, p, c).y - p.poleRadius - p.margin));
    const sideForward = minSideForward - curveGuard * p.track;
    const feasible = clearance > p.margin + 1e-6 && certifiedSteer <= p.maxSteer && certifiedRate <= p.maxRate && gateSlack > 0 && sideForward > 0;
    const violation = Math.min(clearance - p.margin, (p.maxSteer - certifiedSteer) / 45, (p.maxRate - certifiedRate) / 180, gateSlack, sideForward);
    const merit = feasible ? 100 + clearance - length * 0.0002 - maxSteer * 0.00005 : violation;
    return { q: { ...q }, c, samples, clearance, minEdge: minEdge - guard, minPole: minPole - guard,
      sampledMinEdge: minEdge, sampledMinPole: minPole, maxSteer, certifiedSteer, maxRate, certifiedRate,
      maxSlip, gateSlack, sideForward, length, duration: length / (p.speed / 3.6), feasible, merit, guard, dx,
      sampleCount: xs.length, certified: !!bounds, edgeEvent, poleEvent,
      perPole: perPole.map(row => ({ ...row, clearance: row.clearance - guard })) };
  }
  async function optimize(input, options = {}) {
    const p = validate(input), c = course(p);
    const sideChoices = p.first === 0 ? [1, -1] : [p.first], center = -p.side * p.track / 2;
    const candidates = [];
    let tested = 0;
    const yieldNow = options.yieldNow || (() => new Promise(resolve => setTimeout(resolve, 0)));
    function retain(q) {
      const r = evaluate(q, p); tested++; candidates.push(r);
      candidates.sort((a, b) => b.merit - a.merit);
      if (candidates.length > 8) candidates.pop();
    }
    for (const first of sideChoices) {
      for (let bi = -3; bi <= 3; bi++) {
        for (let ai = 0; ai <= 8; ai++) {
          for (let pi = -3; pi <= 3; pi++) {
            for (const shape of [-0.08, 0, 0.08]) retain({
              bias: center + bi * 0.17,
              amplitude: 0.5 + ai * (p.laneWidth * 0.52 - 0.5) / 8,
              phase: pi * p.wheelbase / 4, shape, first
            });
          }
        }
        options.onProgress?.({ tested, stage: 'search', progress: tested / (sideChoices.length * 1323 + 400) });
        await yieldNow();
      }
    }
    let seeds = candidates.slice(0, 5).map(r => r.q);
    for (const scale of [1, 0.45, 0.2]) {
      for (const seed of seeds) {
        let current = evaluate(seed, p);
        for (let pass = 0; pass < 3; pass++) {
          for (const [key, distance] of [['bias', 0.12], ['amplitude', 0.13], ['phase', 0.18], ['shape', 0.025]]) {
            for (const dir of [-1, 1]) {
              const q = { ...current.q, [key]: current.q[key] + distance * scale * dir };
              if (q.amplitude < 0.3 || q.amplitude > p.laneWidth || Math.abs(q.bias) > p.laneWidth / 2 || Math.abs(q.phase) > p.spacing / 3 || Math.abs(q.shape) > 0.15) continue;
              const r = evaluate(q, p); tested++; candidates.push(r);
              if (r.merit > current.merit) current = r;
            }
          }
        }
        candidates.sort((a, b) => b.merit - a.merit); candidates.length = Math.min(8, candidates.length);
        await yieldNow();
      }
      seeds = candidates.slice(0, 4).map(r => r.q);
      options.onProgress?.({ tested, stage: 'refine', progress: 0.85 });
    }
    const finalists = [];
    for (const candidate of candidates.slice(0, 4)) {
      options.onProgress?.({ tested, stage: 'verify', progress: 0.9 + finalists.length * 0.02 });
      await yieldNow(); finalists.push(evaluate(candidate.q, p, { certify: true, record: true }));
    }
    finalists.sort((a, b) => b.merit - a.merit);
    const result = finalists[0]; result.tested = tested; result.parameters = p;
    result.crossings = c.poles.map((pole, j) => {
      function cross(offsetX, offsetY) {
        for (let i = 1; i < result.samples.length; i++) {
          const a = result.samples[i - 1], b = result.samples[i];
          const pa = transform(a, offsetX, offsetY), pb = transform(b, offsetX, offsetY);
          if (pa.x <= pole.x && pb.x >= pole.x) {
            const t = (pole.x - pa.x) / (pb.x - pa.x), x = a.x + t * (b.x - a.x);
            const pose = pathAt(x, result.q, p, c);
            return { rearX: x, lateral: transform(pose, offsetX, offsetY).y, handle: pose.handle / RAD };
          }
        }
        return null;
      }
      const rear = cross(0, 0), front = cross(p.wheelbase, 0), side = cross(p.sideLead, p.side * p.track);
      return { index: j, poleX: pole.x, side: rear && rear.lateral > 0 ? '左' : '右', rear, front, sideWheel: side, ...result.perPole[j] };
    });
    return result;
  }
  function poseAtDistance(result, distance) {
    const rows = result.samples, s = clamp(distance, 0, result.length);
    let lo = 0, hi = rows.length - 1;
    while (hi - lo > 1) { const mid = (lo + hi) >> 1; if (rows[mid].distance < s) lo = mid; else hi = mid; }
    const t = (s - rows[lo].distance) / (rows[hi].distance - rows[lo].distance || 1);
    return pathAt(rows[lo].x + t * (rows[hi].x - rows[lo].x), result.q, result.parameters, result.c);
  }
  return { defaults, validate, course, pathAt, transform, geometry, corners, poleDistance,
    inspectPose, derivativeBounds, evaluate, optimize, poseAtDistance, RAD, clamp };
});
