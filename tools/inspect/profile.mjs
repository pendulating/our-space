#!/usr/bin/env node
// Phase-0 perf matrix (docs/SCALING.md): sweep {build x zoom} in the real WebGPU/WASM
// build (system Chrome) at evening-rush peak and record frame-time percentiles, then
// write docs/perf-baseline.json. Avg FPS is vsync-capped (~120 on ProMotion), so the
// regression signal is p95 / max frame time (the headroom + hitches), not mean FPS.
//
//   ./web/build.sh && python3 -m http.server -d web/dist 8080 &
//   node tools/inspect/profile.mjs            # -> docs/perf-baseline.json
import { chromium } from 'playwright-core';
import { writeFile } from 'node:fs/promises';

const BASE = process.argv[2] || 'http://localhost:8080';
const SETTLE_MS = 47000; // let the day clock climb to the ~17:30 concurrency peak
const SAMPLE_MS = 6000;

// {build x zoom}; wheel = canvas zoom-in ticks applied after settle (0 = launch framing).
const CELLS = [
  { cell: 'manhattan / default', url: `${BASE}/`, wheel: 0 },
  { cell: 'nyc / whole-city', url: `${BASE}/?city=nyc`, wheel: 0 },
  { cell: 'nyc / neighborhood', url: `${BASE}/?city=nyc`, wheel: 95 },
];

const SAMPLE = (ms) =>
  new Promise((res) => {
    const dts = [];
    let t0 = performance.now(), last = t0;
    function tick(now) {
      const dt = now - last; last = now;
      if (dt > 0 && dt < 1000) dts.push(dt);
      if (now - t0 < ms) requestAnimationFrame(tick);
      else {
        dts.sort((a, b) => a - b);
        const q = (p) => (dts.length ? +dts[Math.min(dts.length - 1, Math.floor(p * dts.length))].toFixed(2) : 0);
        res({
          fps: +(dts.length / ((now - t0) / 1000)).toFixed(1),
          p50_ms: q(0.5), p95_ms: q(0.95),
          max_ms: +(dts[dts.length - 1] || 0).toFixed(2),
          frames: dts.length,
        });
      }
    }
    requestAnimationFrame(tick);
  });

const browser = await chromium.launch({
  channel: 'chrome', headless: false,
  args: ['--enable-unsafe-webgpu', '--enable-features=Vulkan', '--ignore-gpu-blocklist'],
});
const results = [];
for (const c of CELLS) {
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1 });
  const logs = [];
  page.on('console', (m) => logs.push(m.text()));
  try {
    await page.goto(c.url, { waitUntil: 'load', timeout: 90000 });
    await page.waitForFunction(() => {
      const ov = document.getElementById('overlay');
      return !ov || getComputedStyle(ov).display === 'none' || ov.style.opacity === '0';
    }, { timeout: 90000 }).catch(() => {});
    await page.waitForTimeout(SETTLE_MS).catch(() => {});
    if (c.wheel > 0) {
      const box = await page.locator('#bevy-canvas').boundingBox().catch(() => null);
      if (box) {
        await page.mouse.move(box.x + box.width * 0.3, box.y + box.height * 0.5);
        for (let i = 0; i < c.wheel; i++) { await page.mouse.wheel(0, 120); await page.waitForTimeout(40).catch(() => {}); }
      }
      await page.waitForTimeout(3000).catch(() => {});
    }
    const r = await page.evaluate(SAMPLE, SAMPLE_MS);
    const replay = logs.find((l) => l.includes('real-day replay')) || '';
    results.push({ cell: c.cell, ...r, loaded: /real-day replay/.test(replay) });
    console.log(`${c.cell.padEnd(22)} ${String(r.fps).padStart(6)} fps   p50 ${r.p50_ms}  p95 ${r.p95_ms}  max ${r.max_ms} ms`);
  } catch (e) {
    console.log(`${c.cell}: ERROR ${e.message}`);
    results.push({ cell: c.cell, error: e.message });
  }
  await page.close().catch(() => {});
}
await browser.close().catch(() => {});

const baseline = {
  measured_at: new Date().toISOString(),
  context: {
    viewport: '1440x900 @1x',
    browser: 'system Google Chrome (WebGPU / Metal)',
    note: 'requestAnimationFrame cadence = on-screen render rate. Mean FPS is vsync-capped; treat p95_ms / max_ms as the regression signal (frame budget = 16.6 ms @60Hz / 8.3 ms @120Hz).',
    settle_ms: SETTLE_MS, sample_ms: SAMPLE_MS,
  },
  cells: results,
};
await writeFile(new URL('../../docs/perf-baseline.json', import.meta.url), JSON.stringify(baseline, null, 2) + '\n');
console.log('\nwrote docs/perf-baseline.json');
process.exit(0);
