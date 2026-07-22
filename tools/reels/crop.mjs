#!/usr/bin/env node
// reels/crop.mjs — Phase 2 of docs/REELS_PLAN.md: per-platform repurposing.
//
// Turns a finished 9:16 reel (from render.mjs) into the other feed aspect ratios
// — 1:1 for an Instagram feed post, 16:9 for X / YouTube (non-Shorts) — without
// re-capturing the WebGPU canvas. It operates on the encoded MP4 (+ its poster
// PNG) with ffmpeg only, so it's fast and every spec can be repurposed in seconds.
//
//   node tools/reels/crop.mjs --in out/watch-astor.mp4          # → .1x1.mp4 + .16x9.mp4
//   node tools/reels/crop.mjs --all                             # every out/*.mp4 (skips crops)
//   node tools/reels/crop.mjs --in out/x.mp4 --aspect 4:5       # a custom target
//   node tools/reels/crop.mjs --all --mode crop                 # center-crop instead of blur-fit
//
// Modes (how the 9:16 source is fitted into a wider/squarer frame):
//   fit  (default) — scale the whole 9:16 frame to fit inside the target, fill the
//                    empty sides with a blurred, zoomed copy of the same frame. The
//                    "blurred-bars" look ubiquitous on social — and, crucially, it
//                    keeps the WHOLE vertical frame, so the top date plate and the
//                    bottom caption/data callout are never cropped off.
//   crop — scale up and center-crop to fill the target. Fills the frame edge-to-edge
//          but LOSES the top/bottom of the source — i.e. the date plate + caption.
//          Use only when the callouts don't matter (e.g. a pure-motion timelapse).
//   letterbox — scale to fit and pad the sides with a solid colour (0x0a0a0a, the
//               same as render.mjs's encode) instead of a blurred fill.
//
// Silent-first, like the reels themselves: no audio stream is touched.

import { spawn } from 'node:child_process';
import { mkdir, readdir, access } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve, isAbsolute, basename } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const outDir = join(__dirname, 'out');

// ---- args ---------------------------------------------------------------
function parseArgs(argv) {
  const o = {
    in: null,           // a single input .mp4 (relative → tools/reels/out/)
    all: false,         // --all: process every out/*.mp4 that isn't already a crop
    aspects: [],        // --aspect W:H (repeatable); defaults below if none given
    mode: 'fit',        // fit | crop | letterbox
    blur: 22,           // gblur sigma for the fit-mode background fill
    pad: '0x0a0a0a',    // letterbox pad colour (matches render.mjs)
    crf: 19,
    preset: 'medium',
    noPoster: false,    // skip the <stem>.png poster crop
    keep: false,        // (reserved) kept for symmetry with render.mjs
  };
  const a = argv.slice(2);
  for (let i = 0; i < a.length; i++) {
    const k = a[i];
    const next = () => a[++i];
    switch (k) {
      case '--in': o.in = next(); break;
      case '--all': o.all = true; break;
      case '--aspect': o.aspects.push(next()); break;
      case '--mode': o.mode = next(); break;
      case '--blur': o.blur = Number(next()); break;
      case '--pad': o.pad = next(); break;
      case '--crf': o.crf = Number(next()); break;
      case '--preset': o.preset = next(); break;
      case '--no-poster': o.noPoster = true; break;
      case '-h': case '--help':
        console.log(`reels/crop.mjs — repurpose a 9:16 reel to other feed aspect ratios

  --in <file>     input .mp4 (relative → tools/reels/out/)
  --all           process every out/*.mp4 (skips files already suffixed .WxH)
  --aspect <W:H>  target aspect (repeatable). Default: 1:1 and 16:9
  --mode <m>      fit (blurred-bars, keeps captions; default) | crop | letterbox
  --blur <n>      gblur sigma for fit-mode background (default 22)
  --pad <hex>     letterbox pad colour (default 0x0a0a0a)
  --crf <n>       x264 quality (default 19)
  --preset <p>    x264 preset (default medium)
  --no-poster     don't also crop the <stem>.png poster

Each target writes <stem>.<WxH>.mp4 (+ <stem>.<WxH>.png) next to the input.`);
        process.exit(0);
    }
  }
  if (!o.aspects.length) o.aspects = ['1:1', '16:9'];
  if (!['fit', 'crop', 'letterbox'].includes(o.mode)) {
    console.error(`✗ --mode must be fit | crop | letterbox (got "${o.mode}")`);
    process.exit(2);
  }
  return o;
}

function gcd(a, b) { return b ? gcd(b, a % b) : a; }

// "1:1" / "16:9" / "1080x1080" → { w, h, label }. Aspect ratios map to canonical
// pixel sizes (height 1080) so every crop is a clean, platform-friendly resolution.
function resolveAspect(spec) {
  const m = spec.match(/^(\d+)\s*[:x]\s*(\d+)$/i);
  if (!m) throw new Error(`bad --aspect "${spec}" (want W:H like 1:1, 16:9)`);
  const aw = Number(m[1]), ah = Number(m[2]);
  // If the tokens look like real pixel dimensions (either ≥ 200), use them verbatim.
  let w, h;
  if (aw >= 200 || ah >= 200) {
    w = aw; h = ah;
  } else {
    // Canonical: 1080 on the short-ish side. Landscape → height 1080; portrait/square
    // → also height 1080 (width scales). Ensures 16:9→1920x1080, 1:1→1080x1080, 9:16→607→pad.
    h = 1080;
    w = Math.round((aw / ah) * h);
  }
  // ffmpeg needs even dimensions for yuv420p.
  w += w % 2; h += h % 2;
  const g = gcd(aw, ah) || 1;
  const label = `${aw / g}x${ah / g}`;
  return { w, h, label };
}

function filterFor(mode, w, h, o) {
  switch (mode) {
    case 'crop':
      // Scale up to cover the target, then center-crop. Fills edge-to-edge; loses
      // the source's top/bottom (date plate + caption). Motion-only reels only.
      return `[0:v]scale=${w}:${h}:force_original_aspect_ratio=increase,` +
             `crop=${w}:${h},setsar=1,format=yuv420p[v]`;
    case 'letterbox':
      // Fit inside, solid-colour pad. Keeps the whole frame; plain bars.
      return `[0:v]scale=${w}:${h}:force_original_aspect_ratio=decrease,` +
             `pad=${w}:${h}:(ow-iw)/2:(oh-ih)/2:color=${o.pad},setsar=1,format=yuv420p[v]`;
    case 'fit':
    default:
      // Blurred-bars: a zoomed+blurred copy of the frame covers the target as the
      // background; the full unscaled-aspect frame is overlaid, centred, on top. The
      // whole 9:16 frame survives, so every caption/callout stays on-screen.
      return `[0:v]split=2[bg][fg];` +
             `[bg]scale=${w}:${h}:force_original_aspect_ratio=increase,crop=${w}:${h},` +
             `gblur=sigma=${o.blur}[bgb];` +
             `[fg]scale=${w}:${h}:force_original_aspect_ratio=decrease[fgs];` +
             `[bgb][fgs]overlay=(W-w)/2:(H-h)/2,setsar=1,format=yuv420p[v]`;
  }
}

function run(cmd, args) {
  return new Promise((res, rej) => {
    const p = spawn(cmd, args, { stdio: ['ignore', 'inherit', 'inherit'] });
    p.on('error', rej);
    p.on('close', (code) => (code === 0 ? res() : rej(new Error(`${cmd} exited ${code}`))));
  });
}

async function exists(p) { try { await access(p); return true; } catch { return false; } }

// Resolve an input path: absolute as-is; otherwise try cwd (so `out/x.mp4` works),
// then fall back to tools/reels/out (so a bare `x.mp4` works too).
async function resolveInput(p) {
  if (isAbsolute(p)) return p;
  const asCwd = resolve(process.cwd(), p);
  if (await exists(asCwd)) return asCwd;
  const inOut = join(outDir, p);
  if (await exists(inOut)) return inOut;
  return join(outDir, basename(p)); // last resort: bare/out-prefixed name under out/
}

async function cropOne(inPath, o) {
  const stem = inPath.replace(/\.mp4$/i, '');
  const posterIn = `${stem}.png`;
  const havePoster = !o.noPoster && (await exists(posterIn));
  for (const spec of o.aspects) {
    const { w, h, label } = resolveAspect(spec);
    const graph = filterFor(o.mode, w, h, o);
    const outMp4 = `${stem}.${label}.mp4`;
    console.log(`  → ${basename(outMp4)}  (${w}x${h}, ${o.mode})`);
    await run('ffmpeg', [
      '-y', '-v', 'error', '-i', inPath,
      '-filter_complex', graph, '-map', '[v]',
      '-c:v', 'libx264', '-crf', String(o.crf), '-preset', o.preset,
      '-movflags', '+faststart', outMp4,
    ]);
    if (havePoster) {
      const outPng = `${stem}.${label}.png`;
      await run('ffmpeg', [
        '-y', '-v', 'error', '-i', posterIn,
        '-filter_complex', graph, '-map', '[v]', outPng,
      ]);
    }
  }
}

// ---- main ---------------------------------------------------------------
const o = parseArgs(process.argv);
await mkdir(outDir, { recursive: true });

let inputs = [];
if (o.all) {
  const files = (await readdir(outDir))
    .filter((f) => f.endsWith('.mp4'))
    .filter((f) => !/\.\d+x\d+\.mp4$/.test(f)) // skip files that are already crops
    .sort();
  inputs = files.map((f) => join(outDir, f));
  if (!inputs.length) { console.error(`✗ no source *.mp4 in ${outDir}`); process.exit(2); }
} else if (o.in) {
  inputs = [await resolveInput(o.in)];
} else {
  console.error('✗ pass --in <file> or --all. See --help.');
  process.exit(2);
}

let failed = 0;
for (const inPath of inputs) {
  if (!(await exists(inPath))) { console.error(`✗ not found: ${inPath}`); failed++; continue; }
  console.log(`\n=== ${basename(inPath)} → ${o.aspects.join(', ')} ===`);
  try { await cropOne(inPath, o); }
  catch (e) { console.error(`✗ ${basename(inPath)}: ${e.message}`); failed++; }
}
console.log(`\n✓ crop: ${inputs.length - failed}/${inputs.length} source reels repurposed → ${outDir}`);
process.exit(failed ? 1 : 0);
