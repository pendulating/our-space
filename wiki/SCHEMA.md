# our-space Wiki Schema

## Domain
The `our-space` project: an interactive + batch geospatial simulation of cameras
entering NYC public space, and the FAccT '27 paper "Digital Eyes on the Street"
that uses the engine as its exposure-measurement instrument. The wiki covers:
the codebase, data layers and sources, the exposure instrument, computed results,
the paper outline and claims, related literature, people, deadlines, and open work.

## Conventions
- File names: lowercase, hyphens (e.g. `exposure-instrument.md`).
- Every page starts with YAML frontmatter (see below).
- Wikilinks `[[page-name]]`, min 2 outbound links per page.
- Bump `updated` on every edit. Every new page goes in `index.md`.
  Every action appends to `log.md`.
- Writing style: ASD-STE100 Simplified Technical English. Short declarative
  sentences. Active voice. No idioms or vague adverbs. Keep technical proper nouns.
- Numbers in pages must match `data/derived/results/` and
  `facct27_digital_eyes_on_the_street/OUTLINE.md`. Cite the source doc when a
  number is load-bearing.

## Frontmatter

```yaml
---
title: Page Title
created: YYYY-MM-DD
updated: YYYY-MM-DD
type: entity | concept | comparison | query | result | layer | person | deadline | moc | codebase | paper
tags: [from taxonomy below]
sources: [docs/..., facct27_digital_eyes_on_the_street/...]
confidence: high | medium | low   # optional but recommended
---
```

## Tag Taxonomy
- Code: `codebase`, `sim-core`, `data-pipeline`, `app-interactive`, `batch`, `web`
- Data: `data-layer`, `open-data`, `license`
- Method: `instrument`, `econometrics`, `routing`, `occlusion`, `undercount`, `mode-choice`
- Paper: `academic-paper`, `facct27`, `claim`, `foresight`, `ethics`, `deadline`
- Society/law: `surveillance`, `privacy`, `equity`, `governance`
- Meta: `comparison`, `moc`, `result`

## Page Types
- `concept` — a method or idea (point-vs-path, capture-recapture, NEAP).
- `layer` — one camera/sensor data layer (CCTV, ALPR, ACE, dashcam, Teslas).
- `entity` — an org, dataset, or external thing (Amnesty, LODES, Nature Cities paper).
- `person` — an author or advisor.
- `result` — a computed headline number with provenance.
- `comparison` — side-by-side analyses.
- `query` — filed answers worth keeping.
- `deadline` — dated obligations.
- `paper` — the manuscript and its sections.
- `moc` — Map of Content hub pages.

## Directory Layout
```
wiki/
├── SCHEMA.md        # This file
├── index.md         # Content catalog
├── log.md           # Append-only action log
├── concepts/
├── entities/
├── comparisons/
├── queries/
└── mocs/
```

## Update Policy
1. Newer results supersede older ones. If numbers conflict across docs, prefer
   `OUTLINE.md` §9 corrections (newest) and mark `contested: true`.
2. Create a page when a thing appears in 2+ docs or is central to one.
3. Split pages over ~200 lines.
