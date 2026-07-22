#!/usr/bin/env node
// reels/render.mjs — Phase 0 of docs/REELS_PLAN.md.
//
// Records the real WebGPU/WASM our-space build (driven in *system Chrome*, like
// tools/inspect) to a vertical 9:16 MP4, with no manual clicking. Phase 0 captures the
// existing URL-triggered StoryMap tours (`?story=longitudinal` / `?story=tutorial`) — the
// app plays the scripted camera + captions itself; this tool just drives Chrome, records
// the canvas over CDP screencast, and stitches the frames with ffmpeg.
//
//   ./web/build.sh && python3 -m http.server -d web/dist 8080 &     # (or pass --serve)
//   node tools/reels/render.mjs --story longitudinal --out decade.mp4 --secs 42
//
// The capture honours the screencast frame timestamps (variable rate in → constant-rate
// out via ffmpeg's fps filter), so playback speed is correct regardless of how fast the
// compositor emits frames. Frames land in a temp dir and are deleted unless --keep-frames.
//
// Phase 0 records the app *as-is*, so the right-hand control panel is still visible — a
// dedicated clean/reel mode (hiding it) is Phase 1 (see REELS_PLAN.md §3 G5 / §6).

import net from 'node:net';
import { spawn } from 'node:child_process';
import { mkdir, writeFile, rm, readFile, readdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve, isAbsolute } from 'node:path';
import { createRequire } from 'node:module';

const __dirname = dirname(fileURLToPath(import.meta.url));
// Reuse the playwright-core + system-Chrome setup already installed for tools/inspect
// (resolve as if required from that directory) so there's no second dependency install.
const requireFromInspect = createRequire(join(__dirname, '../inspect/'));
const { chromium } = requireFromInspect('playwright-core');

// ---- args ---------------------------------------------------------------
function parseArgs(argv) {
  const o = {
    url: 'http://localhost:8080/',
    spec: null,         // path to a JSON reel spec (→ ?reelspec=<base64url>)
    all: null,          // --all [dir]: render every *.json spec in dir (default specs/)
    poster: null,       // seconds into the reel to grab a poster still (default 0.65×secs)
    story: null,        // ?story=<id> (longitudinal | tutorial)
    out: null,          // output mp4 (relative → tools/reels/out/)
    secs: 42,           // capture duration (longitudinal ≈ 39 s; leave headroom)
    fps: 30,            // output frame rate (CFR)
    width: 1080,        // 9:16 vertical
    height: 1920,
    settle: 3500,       // ms after boot before recording (fallback: no story signal)
    storyWait: 30000,   // ms to wait for the app's REEL_STORY_START signal (asset load)
    tail: 1.5,          // extra seconds recorded after the tour so the last caption lands
    everyNth: 2,        // screencast: keep 1 of every N compositor frames (~30 fps in)
    quality: 92,        // screencast jpeg quality
    hideChrome: true,   // hide the HTML masthead/footer for a full-bleed canvas
    reelMode: true,     // ?reel=1 — app hides the right control panel + transport chrome
    serve: false,
    headed: false,
    keepFrames: false,
    eval: null,
    timeout: 90000,
  };
  const a = argv.slice(2);
  for (let i = 0; i < a.length; i++) {
    const k = a[i];
    const next = () => a[++i];
    switch (k) {
      case '--url': o.url = next(); break;
      case '--spec': o.spec = next(); break;
      case '--all': o.all = (a[i + 1] && !a[i + 1].startsWith('--')) ? next() : true; break;
      case '--poster': o.poster = Number(next()); break;
      case '--story': o.story = next(); break;
      case '--out': o.out = next(); break;
      case '--secs': o.secs = Number(next()); break;
      case '--fps': o.fps = Number(next()); break;
      case '--width': o.width = Number(next()); break;
      case '--height': o.height = Number(next()); break;
      case '--settle': o.settle = Number(next()); break;
      case '--every-nth': o.everyNth = Math.max(1, Number(next())); break;
      case '--quality': o.quality = Number(next()); break;
      case '--no-hide-chrome': o.hideChrome = false; break;
      case '--no-reel-mode': o.reelMode = false; break;
      case '--serve': o.serve = true; break;
      case '--headed': o.headed = true; break;
      case '--keep-frames': o.keepFrames = true; break;
      case '--eval': o.eval = next(); break;
      case '--timeout': o.timeout = Number(next()); break;
      case '-h': case '--help':
        console.log(`reels/render.mjs — record a StoryMap tour to a vertical MP4

  --spec <file>     JSON reel spec to play (data-driven; see specs/*.json)
  --all [dir]       render every *.json spec in dir (default tools/reels/specs/)
  --poster <s>      seconds into the reel to grab the poster still (default 0.65×len)
  --story <id>      ?story= built-in tour to play (longitudinal | tutorial)
  --url <url>       base URL (default http://localhost:8080/)
  --out <file>      output .mp4 (relative → tools/reels/out/)
  --secs <n>        capture seconds (default 42)
  --fps <n>         output frame rate (default 30)
  --width/--height  viewport (default 1080x1920, 9:16)
  --settle <ms>     wait after boot before recording (default 3500)
  --every-nth <n>   keep 1 of every N compositor frames (default 2)
  --no-hide-chrome  keep the HTML masthead/footer visible
  --no-reel-mode    keep the app's right control panel + transport chrome (skip ?reel=1)
  --serve           auto-start python http.server on :8080 if down
  --headed          show the Chrome window
  --keep-frames     don't delete the captured PNG/JPEG frames
  --eval <js>       run JS in the page after boot (DOM/CSS only)`);
        process.exit(0);
    }
  }
  return o;
}

// ---- helpers (mirrors tools/inspect/inspect.mjs) ------------------------
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
    console.error('  or re-run with --serve.');
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

function run(cmd, args, opts = {}) {
  return new Promise((res, rej) => {
    const p = spawn(cmd, args, { stdio: ['ignore', 'inherit', 'inherit'], ...opts });
    p.on('error', rej);
    p.on('close', (code) => (code === 0 ? res() : rej(new Error(`${cmd} exited ${code}`))));
  });
}

// ---- main ---------------------------------------------------------------
const o = parseArgs(process.argv);
const outDir = join(__dirname, 'out');
await mkdir(outDir, { recursive: true });

// --all: render every spec in the directory. Start the server once, then re-invoke this
// script per spec (reusing the single-render path) so one bad spec doesn't sink the batch.
if (o.all) {
  const specsDir = typeof o.all === 'string' ? o.all : join(__dirname, 'specs');
  const files = (await readdir(specsDir)).filter((f) => f.endsWith('.json')).sort();
  if (!files.length) {
    console.error(`✗ no *.json specs in ${specsDir}`);
    process.exit(2);
  }
  const server = await maybeServe({ url: 'http://localhost:8080/', serve: true });
  let failed = 0;
  for (const f of files) {
    console.log(`\n=== ${f} (${files.indexOf(f) + 1}/${files.length}) ===`);
    const code = await new Promise((res) => {
      const child = spawn('node', [fileURLToPath(import.meta.url), '--spec', join(specsDir, f)], {
        stdio: 'inherit',
      });
      child.on('close', res);
    });
    if (code !== 0) failed++;
  }
  if (server) server.kill();
  console.log(`\n✓ batch: ${files.length - failed}/${files.length} rendered → ${outDir}`);
  process.exit(failed ? 1 : 0);
}

// A JSON spec (--spec) drives a data-driven tour via ?reelspec=<base64url>. Extra fields
// (id/size/fps/secs) are read here (driver-side); the app only reads {title, steps}. The
// capture length auto-sizes to the sum of step dwells unless --secs was given explicitly.
let reelspecB64 = null;
let specName = null;
let specClock = null; // { t?, rate?, play? } → ?t=&rate=&play=
const secsGiven = process.argv.includes('--secs');
if (o.spec) {
  let spec;
  try {
    spec = JSON.parse(await readFile(o.spec, 'utf8'));
  } catch (e) {
    console.error(`✗ could not read/parse spec ${o.spec}: ${e.message}`);
    process.exit(2);
  }
  if (!Array.isArray(spec.steps) || spec.steps.length === 0) {
    console.error(`✗ spec ${o.spec} has no "steps"`);
    process.exit(2);
  }
  specName = spec.id || 'reel';
  if (Array.isArray(spec.size) && spec.size.length === 2) [o.width, o.height] = spec.size;
  if (spec.fps) o.fps = spec.fps;
  if (spec.poster != null && o.poster == null) o.poster = Number(spec.poster);
  if (spec.clock && typeof spec.clock === 'object') specClock = spec.clock;
  const dwell = spec.steps.reduce((s, st) => s + (Number(st.secs) || 0), 0);
  if (!secsGiven) o.secs = spec.secs || Math.ceil(dwell + 1.5); // tail so the last caption lands
  reelspecB64 = Buffer.from(
    JSON.stringify({ title: spec.title || spec.id || 'Reel', steps: spec.steps }),
    'utf8',
  ).toString('base64url'); // URL-safe, no padding → no percent-encoding, clean extraction
}

const stem = (o.out || (specName ? `${specName}.mp4` : o.story ? `${o.story}.mp4` : 'reel.mp4'))
  .replace(/\.mp4$/i, '');
const outPath = isAbsolute(o.out || '') ? o.out : join(outDir, `${stem}.mp4`);
const framesDir = join(outDir, `.frames-${stem}`);
await rm(framesDir, { recursive: true, force: true });
await mkdir(framesDir, { recursive: true });

const target = new URL(o.url);
if (o.story) target.searchParams.set('story', o.story);
if (o.reelMode) target.searchParams.set('reel', '1');
if (specClock) {
  if (specClock.t != null) target.searchParams.set('t', String(specClock.t));
  if (specClock.rate != null) target.searchParams.set('rate', String(specClock.rate));
  if (specClock.play != null) target.searchParams.set('play', specClock.play ? '1' : '0');
}
if (reelspecB64) target.searchParams.set('reelspec', reelspecB64);

const server = await maybeServe(o);
const browser = await chromium.launch({
  channel: 'chrome',
  headless: !o.headed,
  args: [
    '--enable-unsafe-webgpu', '--enable-features=Vulkan', '--ignore-gpu-blocklist',
    // Keep the compositor running at full rate: without these an offscreen/occluded
    // page can be throttled to ~1 fps, which stretches the screencast timestamps and
    // yields a multi-minute, choppy mp4 for a ~20 s tour.
    '--disable-renderer-backgrounding',
    '--disable-background-timer-throttling',
    '--disable-backgrounding-occluded-windows',
    '--disable-features=CalculateNativeWinOcclusion',
  ],
});
const page = await browser.newPage({
  viewport: { width: o.width, height: o.height },
  deviceScaleFactor: 1, // exact WxH frames; a reel doesn't need retina
});
// Don't let the OS reduced-motion preference pause the clock / snap the flies.
await page.emulateMedia({ reducedMotion: 'no-preference' });

const logs = [];
page.on('console', (m) => logs.push(`[${m.type()}] ${m.text()}`));
page.on('pageerror', (e) => logs.push(`[pageerror] ${e.message}`));

// Story-start alignment: resolve when the app logs REEL_STORY_START (storymap_autostart),
// which fires the frame the tour begins — so recording matches the story timeline.
let storyStartSecs = null;
let resolveStoryStart;
const storyStarted = new Promise((r) => (resolveStoryStart = r));
page.on('console', (m) => {
  const mm = m.text().match(/REEL_STORY_START secs=([\d.]+)/);
  if (mm) {
    storyStartSecs = parseFloat(mm[1]);
    resolveStoryStart();
  }
});

// Live-stat export (G4): the app logs `REEL_STAT {json}` when a headline recomputes. Keep
// the latest for a sidecar .stats.json (the {cameras}/{dashcams} caption tokens are filled
// in-app, so this is metadata, not needed to render the numbers).
let lastStat = null;
page.on('console', (m) => {
  const mm = m.text().match(/REEL_STAT (\{.*\})/);
  if (mm) {
    try { lastStat = JSON.parse(mm[1]); } catch { /* ignore malformed */ }
  }
});

let exitCode = 0;
try {
  console.log(`→ ${target.href}  (${o.width}x${o.height}, ${o.headed ? 'headed' : 'headless'})`);
  await page.goto(target.href, { waitUntil: 'load', timeout: o.timeout });

  if (!(await page.evaluate(() => 'gpu' in navigator))) {
    console.error('✗ navigator.gpu missing — Chrome did not expose WebGPU.');
    exitCode = 3;
  }

  // Full-bleed: hide the HTML masthead/footer so #stage (and the fit-to-parent canvas)
  // fill the viewport height. The egui side panel is canvas-drawn and unaffected (Phase 1).
  if (o.hideChrome) {
    await page.addStyleTag({
      content:
        'header{display:none!important}footer{display:none!important}' +
        '#app{height:100vh!important}#stage{flex:1 1 auto!important}',
    });
  }

  // Boot: wait for the app's own "we booted" signal (loading overlay clears).
  let booted = true;
  try {
    await page.waitForFunction(() => {
      const ov = document.getElementById('overlay');
      return !ov || getComputedStyle(ov).display === 'none' || ov.style.opacity === '0';
    }, { timeout: o.timeout });
  } catch { booted = false; }
  const ovTitle = await page.evaluate(
    () => document.getElementById('ov-title')?.textContent?.trim() || '');
  if (/needs WebGPU|Could not open/i.test(ovTitle)) {
    console.error(`✗ App reported failure: "${ovTitle}"`);
    exitCode = 4;
  } else if (!booted) {
    console.error('✗ Overlay never cleared — WASM may still be loading.');
    exitCode = 5;
  } else {
    console.log('✓ WASM booted, overlay cleared.');
  }
  if (exitCode) throw new Error('boot failed');

  if (o.eval) {
    try { await page.evaluate(o.eval); } catch (e) { console.error(`--eval: ${e.message}`); }
  }

  // Align capture to the story timeline: wait for the app's REEL_STORY_START signal, then
  // record exactly the tour's length (+ a tail so the final caption lands). Falls back to a
  // fixed settle + --secs when there's no story (e.g. a bare URL) or the signal never comes.
  if (o.story || o.spec) {
    await Promise.race([storyStarted, page.waitForTimeout(o.storyWait)]);
    if (storyStartSecs != null) {
      if (!secsGiven) o.secs = Math.ceil(storyStartSecs + o.tail);
      console.log(`✓ story start detected (${storyStartSecs.toFixed(1)}s tour) → recording ${o.secs}s`);
      await page.waitForTimeout(300); // let the first scene compose
    } else {
      console.log(`… no REEL_STORY_START in ${o.storyWait}ms; recording ${o.secs}s after settle`);
      await page.waitForTimeout(o.settle);
    }
  } else {
    await page.waitForTimeout(o.settle);
  }

  // ---- record via CDP screencast -------------------------------------
  const client = await page.context().newCDPSession(page);
  const frames = []; // { t: seconds, file }
  let seq = 0;
  let writing = Promise.resolve();
  client.on('Page.screencastFrame', (evt) => {
    const { data, sessionId, metadata } = evt;
    const idx = seq++;
    const file = join(framesDir, `f${String(idx).padStart(6, '0')}.jpg`);
    frames.push({ t: metadata.timestamp ?? idx / 60, file });
    // Ack immediately so Chrome keeps sending; write off the critical path.
    client.send('Page.screencastFrameAck', { sessionId }).catch(() => {});
    writing = writing.then(() => writeFile(file, Buffer.from(data, 'base64')));
  });

  console.log(`● recording ${o.secs}s (every ${o.everyNth} compositor frame${o.everyNth > 1 ? 's' : ''})…`);
  await client.send('Page.startScreencast', {
    format: 'jpeg',
    quality: o.quality,
    maxWidth: o.width,
    maxHeight: o.height,
    everyNthFrame: o.everyNth,
  });
  await page.waitForTimeout(o.secs * 1000);
  await client.send('Page.stopScreencast');
  await writing; // flush all pending frame writes

  if (frames.length < 2) throw new Error(`captured only ${frames.length} frame(s)`);
  console.log(`✓ captured ${frames.length} frames`);

  // ---- assemble: honour per-frame timing, resample to CFR ------------
  // ffconcat lists each frame + the wall-time to the next; ffmpeg's fps filter then
  // resamples the variable-rate input to a constant --fps. Last frame repeated (concat
  // ignores the final duration otherwise).
  const t0 = frames[0].t;
  let list = 'ffconcat version 1.0\n';
  for (let i = 0; i < frames.length; i++) {
    const cur = frames[i].t - t0;
    const nextT = i + 1 < frames.length ? frames[i + 1].t - t0 : cur + 1 / o.fps;
    const dur = Math.max(1 / 240, nextT - cur); // guard against zero/negative deltas
    list += `file '${frames[i].file}'\nduration ${dur.toFixed(4)}\n`;
  }
  list += `file '${frames[frames.length - 1].file}'\n`;
  const listPath = join(framesDir, 'frames.txt');
  await writeFile(listPath, list);

  const vf =
    `fps=${o.fps},scale=${o.width}:${o.height}:force_original_aspect_ratio=decrease,` +
    `pad=${o.width}:${o.height}:(ow-iw)/2:(oh-ih)/2:color=0x0a0a0a,format=yuv420p`;
  console.log('→ ffmpeg encode…');
  await run('ffmpeg', [
    '-y', '-f', 'concat', '-safe', '0', '-i', listPath,
    '-vf', vf, '-c:v', 'libx264', '-crf', '19', '-preset', 'medium',
    '-movflags', '+faststart', outPath,
  ]);
  const dur = ((frames[frames.length - 1].t - t0) || 0).toFixed(1);
  console.log(`✓ reel → ${outPath}  (${dur}s of capture, ${frames.length} frames → ${o.fps}fps)`);
  if (lastStat) {
    const statsPath = outPath.replace(/\.mp4$/i, '.stats.json');
    await writeFile(statsPath, JSON.stringify(lastStat, null, 2));
    console.log(`✓ stats → ${statsPath}  ${JSON.stringify(lastStat)}`);
  }

  // Poster still: a representative frame (default ~65% in — usually the result/hold beat)
  // for thumbnails / social cards. From the encoded MP4 so it matches the final crop.
  const posterT = o.poster != null ? Math.max(0, o.poster) : Number(dur) * 0.65;
  const posterPath = outPath.replace(/\.mp4$/i, '.png');
  await run('ffmpeg', [
    '-y', '-v', 'error', '-ss', String(posterT), '-i', outPath, '-frames:v', '1', posterPath,
  ]);
  console.log(`✓ poster → ${posterPath}  (@${posterT.toFixed(1)}s)`);
} catch (err) {
  console.error(`✗ ${err.message}`);
  if (logs.length) console.error('--- console (tail) ---\n' + logs.slice(-12).join('\n'));
  exitCode = exitCode || 1;
} finally {
  await browser.close();
  if (server) server.kill();
  if (!o.keepFrames) await rm(framesDir, { recursive: true, force: true });
  else console.log(`… frames kept in ${framesDir}`);
}
process.exit(exitCode);
