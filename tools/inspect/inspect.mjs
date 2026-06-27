#!/usr/bin/env node
// Inspect the WebGPU/WASM our-space web build by driving the *system* Google
// Chrome (channel:'chrome') via playwright-core. Real Chrome on macOS gets the
// Metal GPU, so WebGPU actually initializes and the Bevy canvas renders — the
// bundled Playwright Chromium can't do this reliably headless on macOS.
//
//   node inspect.mjs                          # screenshot localhost:8080 -> out/
//   node inspect.mjs --story intro            # deep-link ?story=intro
//   node inspect.mjs --width 1920 --height 1080 --out hero.png
//   node inspect.mjs --headed --keep          # watch it live, leave Chrome open
//   node inspect.mjs --console --dom          # also dump console logs + panel text
//   node inspect.mjs --eval "window.scrollTo(0,0)"   # run JS before the shot
//   node inspect.mjs --serve                  # auto-start python server if :8080 is down
//
// Exit code is non-zero if WebGPU failed to initialize (so it's CI-friendly).

import { chromium } from 'playwright-core';
import { spawn } from 'node:child_process';
import { mkdir, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve, isAbsolute } from 'node:path';
import net from 'node:net';

const __dirname = dirname(fileURLToPath(import.meta.url));

// ---- args ---------------------------------------------------------------
function parseArgs(argv) {
  const o = {
    url: 'http://localhost:8080/',
    out: null,
    width: 1440,
    height: 900,
    scale: 2,
    story: null,
    wait: 4000,          // extra settle after overlay clears (asset load + first frames)
    timeout: 90000,      // max wait for the WASM boot / overlay to clear
    headed: false,
    keep: false,
    fullPage: false,
    console: false,
    dom: false,
    eval: null,
    serve: false,
    wheel: 0,            // total wheel deltaY to dispatch over the canvas (+ = zoom out)
    pan: null,           // "dx,dy" screen px to drag the map after zooming
    click: null,         // "x,y" viewport px to click after zoom/pan (test interactions)
  };
  for (let i = 2; i < argv.length; i++) {
    const a = argv[i];
    const next = () => argv[++i];
    switch (a) {
      case '--url': o.url = next(); break;
      case '--out': o.out = next(); break;
      case '--width': o.width = +next(); break;
      case '--height': o.height = +next(); break;
      case '--scale': o.scale = +next(); break;
      case '--story': o.story = next(); break;
      case '--wait': o.wait = +next(); break;
      case '--timeout': o.timeout = +next(); break;
      case '--headed': o.headed = true; break;
      case '--keep': o.keep = true; o.headed = true; break;
      case '--full-page': case '--fullPage': o.fullPage = true; break;
      case '--console': o.console = true; break;
      case '--dom': o.dom = true; break;
      case '--eval': o.eval = next(); break;
      case '--wheel': o.wheel = +next(); break;
      case '--pan': o.pan = next(); break;
      case '--click': o.click = next(); break;
      case '--serve': o.serve = true; break;
      case '-h': case '--help': printHelp(); process.exit(0); break;
      default: console.error(`Unknown arg: ${a}`); printHelp(); process.exit(2);
    }
  }
  return o;
}

function printHelp() {
  console.log(`our-space frontend inspector (WebGPU/WASM via system Chrome)

Usage: node inspect.mjs [options]
  --url <url>        Target (default http://localhost:8080/)
  --story <id>       Append ?story=<id> deep-link
  --out <file>       PNG path (default out/shot-<ts>.png)
  --width/--height   Viewport px (default 1440x900)
  --scale <n>        Device scale factor (default 2 = retina)
  --wait <ms>        Settle time after overlay clears (default 4000)
  --timeout <ms>     Max wait for WASM boot (default 90000)
  --full-page        Capture full scroll height
  --headed           Show the Chrome window
  --keep             Leave Chrome open (implies --headed)
  --console          Print browser console logs
  --dom              Print visible text of the overlay/UI panels
  --eval <js>        Run JS in the page before the screenshot
  --wheel <dy>       Scroll the canvas to zoom (+dy = zoom out, e.g. 900)
  --pan <dx,dy>      Drag the map by dx,dy screen px after zooming (e.g. 200,0 = pan right)
  --click <x,y>      Click at viewport px x,y after zoom/pan (test map interactions)
  --serve            Start 'python3 -m http.server' on :8080 if it's down`);
}

// ---- helpers ------------------------------------------------------------
function tsName() {
  // No Date.now() games — just enough to avoid clobbering.
  const d = new Date();
  const p = (n) => String(n).padStart(2, '0');
  return `shot-${p(d.getMonth() + 1)}${p(d.getDate())}-${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}.png`;
}

function portOpen(host, port, timeout = 800) {
  return new Promise((res) => {
    const s = net.connect({ host, port });
    const done = (ok) => { s.destroy(); res(ok); };
    s.setTimeout(timeout);
    s.once('connect', () => done(true));
    s.once('timeout', () => done(false));
    s.once('error', () => done(false));
  });
}

async function maybeServe(o) {
  if (!o.url.includes('localhost:8080') && !o.url.includes('127.0.0.1:8080')) return null;
  if (await portOpen('127.0.0.1', 8080)) return null;
  if (!o.serve) {
    console.error('✗ Nothing is serving :8080. Start it with:');
    console.error('    python3 -m http.server -d web/dist 8080');
    console.error('  or re-run with --serve to start it automatically.');
    process.exit(1);
  }
  const dist = resolve(__dirname, '../../web/dist');
  console.log(`→ starting static server: python3 -m http.server -d ${dist} 8080`);
  const proc = spawn('python3', ['-m', 'http.server', '-d', dist, '8080'], { stdio: 'ignore' });
  for (let i = 0; i < 40; i++) {
    if (await portOpen('127.0.0.1', 8080)) return proc;
    await new Promise((r) => setTimeout(r, 250));
  }
  throw new Error('server did not come up on :8080');
}

// ---- main ---------------------------------------------------------------
const o = parseArgs(process.argv);
const outDir = join(__dirname, 'out');
await mkdir(outDir, { recursive: true });
const outPath = o.out
  ? (isAbsolute(o.out) ? o.out : join(outDir, o.out))
  : join(outDir, tsName());

const target = new URL(o.url);
if (o.story) target.searchParams.set('story', o.story);

const server = await maybeServe(o);

const browser = await chromium.launch({
  channel: 'chrome',
  headless: !o.headed,
  args: [
    '--enable-unsafe-webgpu',
    '--enable-features=Vulkan',
    '--ignore-gpu-blocklist',
  ],
});

const page = await browser.newPage({
  viewport: { width: o.width, height: o.height },
  deviceScaleFactor: o.scale,
});

const logs = [];
const netFails = new Set();
page.on('console', (m) => logs.push(`[${m.type()}] ${m.text()}`));
page.on('pageerror', (e) => logs.push(`[pageerror] ${e.message}`));
page.on('response', (r) => { if (r.status() >= 400) netFails.add(`${r.status()} ${r.url()}`); });
page.on('requestfailed', (r) => netFails.add(`ERR ${r.url()}`));

let exitCode = 0;
try {
  console.log(`→ ${target.href}  (${o.width}x${o.height} @${o.scale}x, ${o.headed ? 'headed' : 'headless'})`);
  await page.goto(target.href, { waitUntil: 'load', timeout: o.timeout });

  // WebGPU present at all?
  const hasGpu = await page.evaluate(() => 'gpu' in navigator);
  if (!hasGpu) {
    console.error('✗ navigator.gpu is missing — Chrome did not expose WebGPU.');
    exitCode = 3;
  }

  // Wait for the boot overlay to clear (set to display:none once wasm runs),
  // which is the app's own "we booted" signal.
  let booted = true;
  try {
    await page.waitForFunction(() => {
      const ov = document.getElementById('overlay');
      return !ov || getComputedStyle(ov).display === 'none' || ov.style.opacity === '0';
    }, { timeout: o.timeout });
  } catch {
    booted = false;
  }

  // Did the page hit its own WebGPU/load failure path?
  const ovTitle = await page.evaluate(() => document.getElementById('ov-title')?.textContent?.trim() || '');
  const failed = /needs WebGPU|Could not open/i.test(ovTitle);
  if (failed) {
    console.error(`✗ App reported failure: "${ovTitle}"`);
    exitCode = 4;
  } else if (!booted) {
    console.error('✗ Overlay never cleared within timeout — WASM may still be loading.');
    exitCode = 5;
  } else {
    console.log('✓ WASM booted, overlay cleared.');
  }

  if (o.eval) {
    try { await page.evaluate(o.eval); } catch (e) { console.error(`--eval error: ${e.message}`); }
  }

  // Zoom by scrolling the canvas: real wheel events go through CDP so winit (Bevy's
  // input backend) receives them, unlike synthetic DOM events. Chunked so the
  // multiplicative zoom integrates smoothly and the mesh-rescale systems keep up.
  if (o.wheel) {
    // Aim at the left-map region, not the canvas center — the egui panel (drawn on
    // the right ~40% of the canvas) would otherwise swallow the scroll.
    const box = await page.locator('#bevy-canvas').boundingBox();
    if (box) await page.mouse.move(box.x + box.width * 0.25, box.y + box.height * 0.5);
    const step = o.wheel > 0 ? 120 : -120;
    const ticks = Math.ceil(Math.abs(o.wheel) / 120);
    for (let i = 0; i < ticks; i++) {
      await page.mouse.wheel(0, step);
      await page.waitForTimeout(60);
    }
    console.log(`↕ wheel ${o.wheel} (${ticks} ticks) — ${o.wheel > 0 ? 'zoom out' : 'zoom in'}`);
  }

  // Pan by dragging the map (left-mouse drag over the map area, not the panel).
  if (o.pan) {
    const [dx, dy] = o.pan.split(',').map(Number);
    const box = await page.locator('#bevy-canvas').boundingBox();
    if (box) {
      const sx = box.x + box.width * 0.28, sy = box.y + box.height * 0.5;
      await page.mouse.move(sx, sy);
      await page.mouse.down();
      // Step the drag so winit integrates it as motion, not a teleport.
      const steps = 12;
      for (let i = 1; i <= steps; i++) {
        await page.mouse.move(sx + (dx * i) / steps, sy + (dy * i) / steps);
        await page.waitForTimeout(16);
      }
      await page.mouse.up();
      console.log(`✣ pan ${dx},${dy}`);
    }
  }

  // Click at a viewport pixel. Explicit move → down → pause → up so winit/Bevy see a
  // press and release on separate frames (a fast synthetic click can get coalesced).
  if (o.click) {
    const [cx, cy] = o.click.split(',').map(Number);
    await page.mouse.move(cx, cy);
    await page.waitForTimeout(60);
    await page.mouse.down();
    await page.waitForTimeout(140);
    await page.mouse.up();
    await page.waitForTimeout(400);
    console.log(`● click ${cx},${cy}`);
  }

  // Settle: let Bevy finish asset load + render a few frames.
  if (o.wait > 0) await page.waitForTimeout(o.wait);

  await page.screenshot({ path: outPath, fullPage: o.fullPage });
  console.log(`✓ screenshot → ${outPath}`);

  if (o.dom) {
    const text = await page.evaluate(() => {
      const grab = (sel) => Array.from(document.querySelectorAll(sel))
        .map((el) => el.innerText?.trim()).filter(Boolean);
      return {
        overlay: grab('#overlay'),
        panels: grab('#app .panel, #app aside, #app [class*=panel]'),
      };
    });
    console.log('\n--- visible UI text ---');
    console.log(JSON.stringify(text, null, 2));
  }

  if (o.console) {
    console.log('\n--- browser console ---');
    console.log(logs.length ? logs.join('\n') : '(no console output)');
  }

  if (netFails.size) {
    // Collapse to unique URL paths so a hundred tile 404s read as one line.
    const byPath = new Map();
    for (const f of netFails) {
      const m = f.match(/^(\S+)\s+(.*)$/);
      const status = m ? m[1] : '?';
      let key = m ? m[2] : f;
      try { key = new URL(key).pathname.replace(/\/[^/]*$/, '/…'); } catch {}
      byPath.set(`${status} ${key}`, (byPath.get(`${status} ${key}`) || 0) + 1);
    }
    console.log('\n--- failed requests (collapsed) ---');
    for (const [k, n] of byPath) console.log(`  ${k}  ×${n}`);
  }
} catch (err) {
  console.error(`✗ ${err.message}`);
  if (logs.length) console.error('--- console ---\n' + logs.join('\n'));
  exitCode = 1;
} finally {
  if (o.keep) {
    console.log('… --keep set: leaving Chrome open. Ctrl-C to exit.');
  } else {
    await browser.close();
    if (server) server.kill();
  }
}

process.exit(exitCode);
