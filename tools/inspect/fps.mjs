#!/usr/bin/env node
// Measure on-screen FPS of the WebGPU/WASM build via system Chrome (playwright-core).
// Counts requestAnimationFrame cadence = real render rate. Headed for real vsync/GPU.
//
//   node fps.mjs [url]
import { chromium } from 'playwright-core';

const url = process.argv[2] || 'http://localhost:8080/?city=nyc';

const browser = await chromium.launch({
  channel: 'chrome',
  headless: false,
  args: ['--enable-unsafe-webgpu', '--enable-features=Vulkan', '--ignore-gpu-blocklist'],
});
const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1 });
const logs = [];
let crashed = null;
page.on('console', (m) => logs.push(`[${m.type()}] ${m.text()}`));
page.on('pageerror', (e) => logs.push(`[pageerror] ${e.message}`));
page.on('crash', () => { crashed = 'page crash event'; });
page.on('close', () => { crashed ||= 'page close event'; });

const tail = (n = 12) => logs.slice(-n).join('\n');

async function sample(label, ms) {
  if (page.isClosed()) { console.log(`FPS[${label}] SKIPPED (page closed: ${crashed})`); return null; }
  try {
    const res = await page.evaluate((ms) => new Promise((resolve) => {
      let frames = 0, t0 = performance.now(), last = t0, worst = 0;
      function tick(now) {
        const dt = now - last; last = now;
        if (frames > 0 && dt > worst) worst = dt;
        frames++;
        if (now - t0 < ms) requestAnimationFrame(tick);
        else resolve({ fps: +(frames / ((now - t0) / 1000)).toFixed(1), worstFrameMs: +worst.toFixed(1), frames });
      }
      requestAnimationFrame(tick);
    }), ms);
    console.log(`FPS[${label}] ${JSON.stringify(res)}`);
    return res;
  } catch (e) {
    console.log(`FPS[${label}] ERROR ${e.message} (crashed=${crashed})`);
    return null;
  }
}

try {
  console.log(`→ ${url}`);
  await page.goto(url, { waitUntil: 'load', timeout: 90000 });
  console.log('navigator.gpu:', await page.evaluate(() => 'gpu' in navigator));
  try {
    await page.waitForFunction(() => {
      const ov = document.getElementById('overlay');
      return !ov || getComputedStyle(ov).display === 'none' || ov.style.opacity === '0';
    }, { timeout: 90000 });
    console.log('✓ booted, overlay cleared @', new Date().toLocaleTimeString());
  } catch { console.log('✗ overlay never cleared'); }

  // Poll up to 60s for the citywide replay log (asset fully loaded + Sim built).
  let loaded = false;
  for (let i = 0; i < 60 && !page.isClosed(); i++) {
    if (logs.some((l) => l.includes('real-day replay'))) { loaded = true; break; }
    await page.waitForTimeout(1000).catch(() => {});
  }
  console.log('citywide replay loaded:', loaded, '| crashed:', crashed);
  console.log('replay line:', logs.find((l) => l.includes('real-day replay')) || '(none)');

  // Sample now (early ramp), then again after the clock climbs toward peak.
  await sample('early', 5000);
  if (!page.isClosed()) { await page.waitForTimeout(45000).catch(() => {}); await sample('peak-default-zoom', 6000); }
  if (!page.isClosed()) await page.screenshot({ path: 'out/fps-default.png' }).catch(() => {});

  // Worst case: zoom all the way out to the whole 5 boroughs so the cull admits the
  // full pool. Real wheel events via CDP so winit/Bevy receive them; aim left of the
  // egui panel. Then let the cull rebuild settle and sample.
  if (!page.isClosed()) {
    const box = await page.locator('#bevy-canvas').boundingBox().catch(() => null);
    if (box) {
      await page.mouse.move(box.x + box.width * 0.25, box.y + box.height * 0.5);
      for (let i = 0; i < 40; i++) { await page.mouse.wheel(0, 120); await page.waitForTimeout(50).catch(() => {}); }
      console.log('↕ zoomed out to whole-city');
    }
    await page.waitForTimeout(3000).catch(() => {});
    await sample('peak-citywide-zoom', 6000);
    await page.screenshot({ path: 'out/fps-citywide.png' }).catch(() => {});
  }

  console.log('\n--- console tail ---\n' + tail(16));
} catch (err) {
  console.log(`✗ ${err.message} (crashed=${crashed})`);
  console.log('--- console tail ---\n' + tail(16));
} finally {
  if (!page.isClosed()) await browser.close().catch(() => {});
}
process.exit(0);
