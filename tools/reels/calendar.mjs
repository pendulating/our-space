#!/usr/bin/env node
// reels/calendar.mjs — Phase 2 of docs/REELS_PLAN.md: a posting-schedule generator.
//
// Reads the authored reel specs (specs/*.json) and emits a content calendar: which
// reel posts on which day, to which platforms (with the aspect each platform wants),
// the on-screen hook, the live headline number (from the render's .stats.json), a
// ready-to-paste caption block, and a per-reel asset checklist (which crops exist vs
// still need `crop.mjs`). Output as Markdown (human) + CSV (spreadsheet/automation).
//
//   node tools/reels/calendar.mjs                       # → out/calendar.md + out/calendar.csv
//   node tools/reels/calendar.mjs --start 2026-07-07 --cadence 3
//   node tools/reels/calendar.mjs --platforms tiktok,ig-reels,ig-feed,x
//   node tools/reels/calendar.mjs --print               # also dump the Markdown to stdout
//
// It plans and reports; it never posts anything. Scheduling is deliberately dumb
// (one reel every `--cadence` days from `--start`) so the author can hand-edit.

import { readFile, readdir, writeFile, access } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const specsDir = join(__dirname, 'specs');
const outDir = join(__dirname, 'out');

// Platform → the aspect it wants + the crop label crop.mjs produces for it. 9:16 is the
// native render (no crop needed); 1:1 / 16:9 come from `node crop.mjs`.
const PLATFORMS = {
  'ig-reels':  { name: 'Instagram Reels', aspect: '9:16', label: null },
  'tiktok':    { name: 'TikTok',          aspect: '9:16', label: null },
  'yt-shorts': { name: 'YouTube Shorts',  aspect: '9:16', label: null },
  'ig-feed':   { name: 'Instagram Feed',  aspect: '1:1',  label: '1x1' },
  'x':         { name: 'X / Twitter',     aspect: '16:9', label: '16x9' },
  'yt':        { name: 'YouTube',         aspect: '16:9', label: '16x9' },
};
const DEFAULT_PLATFORMS = ['ig-reels', 'tiktok', 'yt-shorts', 'ig-feed', 'x'];

const SOURCES = 'Sources: OpenStreetMap · Amnesty Decode Surveillance NYC · NYC DOT · NYC Parks · NYC TLC.';

// ---- args ---------------------------------------------------------------
function parseArgs(argv) {
  const o = {
    start: null,               // YYYY-MM-DD (default: today, local)
    cadence: 3,                // days between posts
    perDay: 1,                 // reels scheduled per posting day
    platforms: DEFAULT_PLATFORMS,
    order: 'file',             // file (dir order) | title (alphabetical)
    print: false,
  };
  const a = argv.slice(2);
  for (let i = 0; i < a.length; i++) {
    const k = a[i];
    const next = () => a[++i];
    switch (k) {
      case '--start': o.start = next(); break;
      case '--cadence': o.cadence = Math.max(1, Number(next())); break;
      case '--per-day': o.perDay = Math.max(1, Number(next())); break;
      case '--platforms': o.platforms = next().split(',').map((s) => s.trim()).filter(Boolean); break;
      case '--order': o.order = next(); break;
      case '--print': o.print = true; break;
      case '-h': case '--help':
        console.log(`reels/calendar.mjs — emit a posting schedule from specs/*.json

  --start <YYYY-MM-DD>   first posting date (default: today)
  --cadence <days>       days between posts (default 3)
  --per-day <n>          reels per posting day (default 1)
  --platforms <a,b,..>   ${Object.keys(PLATFORMS).join(', ')}
                         (default: ${DEFAULT_PLATFORMS.join(', ')})
  --order <file|title>   spec ordering (default file)
  --print                also print the Markdown to stdout

Writes out/calendar.md and out/calendar.csv. Plans only — never posts.`);
        process.exit(0);
    }
  }
  const bad = o.platforms.filter((p) => !PLATFORMS[p]);
  if (bad.length) {
    console.error(`✗ unknown platform(s): ${bad.join(', ')}. Known: ${Object.keys(PLATFORMS).join(', ')}`);
    process.exit(2);
  }
  return o;
}

async function exists(p) { try { await access(p); return true; } catch { return false; } }

// Fill the caption tokens the app resolves at render time, from the .stats.json sidecar,
// so the schedule shows the real hook copy. Unknown tokens (e.g. {place}, which isn't in
// the sidecar) are left bracketed as an author TODO.
function fillTokens(caption, stat) {
  if (!caption) return caption;
  const num = (v) => (v == null ? null : Number(v).toLocaleString('en-US'));
  const map = {
    '{cameras}': stat && stat.cameras != null ? num(stat.cameras) : null,
    '{cameras_raw}': stat && stat.cameras_raw != null ? num(stat.cameras_raw) : null,
    '{dashcams}': stat && stat.dashcams != null ? num(stat.dashcams) : null,
    '{minutes}': stat && stat.minutes != null ? String(stat.minutes) : null,
  };
  let out = caption;
  for (const [tok, val] of Object.entries(map)) if (val != null) out = out.split(tok).join(val);
  return out;
}

// Local-date helpers (avoid UTC/DST drift by working in calendar-day integers).
function todayISO() {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}
function addDaysISO(iso, days) {
  const [y, m, d] = iso.split('-').map(Number);
  const dt = new Date(y, m - 1, d);
  dt.setDate(dt.getDate() + days);
  return `${dt.getFullYear()}-${String(dt.getMonth() + 1).padStart(2, '0')}-${String(dt.getDate()).padStart(2, '0')}`;
}
function weekday(iso) {
  const [y, m, d] = iso.split('-').map(Number);
  return ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'][new Date(y, m - 1, d).getDay()];
}

// ---- gather -------------------------------------------------------------
const o = parseArgs(process.argv);
const startISO = o.start || todayISO();
if (!/^\d{4}-\d{2}-\d{2}$/.test(startISO)) {
  console.error(`✗ --start must be YYYY-MM-DD (got "${startISO}")`);
  process.exit(2);
}

const files = (await readdir(specsDir)).filter((f) => f.endsWith('.json')).sort();
if (!files.length) { console.error(`✗ no specs in ${specsDir}`); process.exit(2); }

const reels = [];
for (const f of files) {
  let spec;
  try { spec = JSON.parse(await readFile(join(specsDir, f), 'utf8')); }
  catch (e) { console.error(`… skipping ${f}: ${e.message}`); continue; }
  if (!Array.isArray(spec.steps) || !spec.steps.length) continue;
  const id = spec.id || f.replace(/\.json$/, '');
  const captions = spec.steps.map((s) => s.caption).filter(Boolean);
  const statsPath = join(outDir, `${id}.stats.json`);
  let stat = null;
  if (await exists(statsPath)) {
    try { stat = JSON.parse(await readFile(statsPath, 'utf8')); } catch { /* ignore */ }
  }
  const mp4 = join(outDir, `${id}.mp4`);
  const rendered = await exists(mp4);
  // Which crop variants exist on disk already (for the asset checklist).
  const cropHave = {};
  for (const label of ['1x1', '16x9']) cropHave[label] = await exists(join(outDir, `${id}.${label}.mp4`));
  const dwell = spec.steps.reduce((s, st) => s + (Number(st.secs) || 0), 0);
  reels.push({
    id,
    title: spec.title || id,
    hook: fillTokens(captions[0], stat),
    payoff: fillTokens(captions[captions.length - 1], stat),
    stat,
    lengthSecs: Math.ceil(dwell + 1.5),
    rendered,
    cropHave,
    file: f,
  });
}
if (o.order === 'title') reels.sort((a, b) => a.title.localeCompare(b.title));

// Assign dates: `perDay` reels share a posting day, then advance `cadence` days.
let dateISO = startISO;
let onThisDay = 0;
for (const r of reels) {
  if (onThisDay >= o.perDay) { dateISO = addDaysISO(dateISO, o.cadence); onThisDay = 0; }
  r.date = dateISO;
  r.weekday = weekday(dateISO);
  onThisDay++;
}

// Which aspects does the chosen platform set require? (drives the asset checklist)
const neededLabels = new Set();
for (const p of o.platforms) { const lbl = PLATFORMS[p].label; if (lbl) neededLabels.add(lbl); }

// ---- render Markdown ----------------------------------------------------
function platformCell(r) {
  return o.platforms.map((p) => `${PLATFORMS[p].name} (${PLATFORMS[p].aspect})`).join(', ');
}
function assetStatus(r) {
  const bits = [`9:16 ${r.rendered ? '✓' : '✗ render'}`];
  for (const lbl of neededLabels) {
    const asp = lbl === '1x1' ? '1:1' : '16:9';
    bits.push(`${asp} ${r.cropHave[lbl] ? '✓' : (r.rendered ? '→ crop' : '✗')}`);
  }
  return bits.join(' · ');
}

let md = `# Reels posting calendar\n\n`;
md += `_Generated by \`tools/reels/calendar.mjs\`. ${reels.length} reel(s), `;
md += `starting ${startISO} (${weekday(startISO)}), every ${o.cadence} day(s)`;
md += o.perDay > 1 ? `, ${o.perDay}/day.` : `.`;
md += ` Platforms: ${o.platforms.map((p) => PLATFORMS[p].name).join(', ')}._\n\n`;
md += `> Planning aid only; this tool never posts. Numbers are quoted from each render's `;
md += `\`.stats.json\`; re-render + re-run after the data changes.\n\n`;

md += `## Schedule\n\n`;
md += `| Date | Day | Reel | Length | Platforms | Hook |\n`;
md += `|------|-----|------|--------|-----------|------|\n`;
for (const r of reels) {
  const hook = (r.hook || '').replace(/\|/g, '\\|');
  md += `| ${r.date} | ${r.weekday} | **${r.title}** | ~${r.lengthSecs}s | ${platformCell(r)} | ${hook} |\n`;
}

md += `\n## Assets\n\n`;
md += `| Reel | \`id\` | Deliverables |\n|------|------|------|\n`;
for (const r of reels) {
  md += `| ${r.title} | \`${r.id}\` | ${assetStatus(r)} |\n`;
}
if (neededLabels.size) {
  md += `\nMissing crops? \`node tools/reels/crop.mjs --all\` (or \`--in out/<id>.mp4\`). `;
  md += `Missing 9:16 renders? \`node tools/reels/render.mjs --spec specs/<id>.json\`.\n`;
}

md += `\n## Caption copy\n\n`;
for (const r of reels) {
  md += `### ${r.date} · ${r.title}\n\n`;
  md += `${r.hook || '(no hook caption)'}\n\n`;
  if (r.payoff && r.payoff !== r.hook) md += `${r.payoff}\n\n`;
  md += `${SOURCES} A simulated day (Apr 21 2026), not live.\n\n`;
}

const mdPath = join(outDir, 'calendar.md');
await writeFile(mdPath, md);

// ---- render CSV ---------------------------------------------------------
function csvCell(s) {
  const v = String(s ?? '');
  return /[",\n]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v;
}
let csv = 'date,weekday,id,title,length_secs,platforms,aspects,hook,payoff,headline\n';
for (const r of reels) {
  const platforms = o.platforms.map((p) => PLATFORMS[p].name).join('; ');
  const aspects = [...new Set(o.platforms.map((p) => PLATFORMS[p].aspect))].join('; ');
  const headline = r.stat ? JSON.stringify(r.stat) : '';
  csv += [r.date, r.weekday, r.id, r.title, r.lengthSecs, platforms, aspects, r.hook, r.payoff, headline]
    .map(csvCell).join(',') + '\n';
}
const csvPath = join(outDir, 'calendar.csv');
await writeFile(csvPath, csv);

console.log(`✓ calendar → ${mdPath}`);
console.log(`✓ calendar → ${csvPath}`);
console.log(`  ${reels.length} reel(s), ${startISO}→${reels[reels.length - 1].date}, ` +
            `platforms: ${o.platforms.join(', ')}`);
if (o.print) { console.log('\n' + md); }
