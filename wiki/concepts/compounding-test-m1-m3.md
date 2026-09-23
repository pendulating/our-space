---
title: Compounding Test M1–M3
created: 2026-08-23
updated: 2026-08-23
type: concept
tags: [method, claim, facct27]
sources: [facct27_digital_eyes_on_the_street/OUTLINE.md §8]
confidence: high
---

# Future Work: M1–M3 (Pre-Registered In-Paper)

Stated inside the paper so the follow-up is credible and the flag is planted.

## M1 — Wire mobile classes into block-group tables
A plumbing job, not a research job. The machinery exists (`sim-core::mobile`,
`MobileScenario`, `AceConfig`, `DashcamFieldLayer`, `exposure_rates_per_minute`)
but is used only by `batch heatmap`, never by `bg-exposure` / `od-exposure*`. Emit
`M_i^res` and `M_i^act` for the two observed fleets:
- **ACE buses** — real GTFS shapes and headways (5 min rush / 10 min day / 25 min
  overnight), 20 m capture range, both directions. Trajectories fully observed.
- **Rideshare dashcams** — real TLC day (547,263 trips); penetration 0.40 and
  capture 0.40 are parameters → sweep 0.25–0.45, report bands.

## M2 — The compounding test (the single statistic)
Pop-weighted corr(`R_i`, `M_i^act`) across BGs. > 0 ⇒ mobile surveillance falls on
the same people already most fixed-exposed — it compounds. < 0 ⇒ it offsets. Plus:
share of population in the top decile of both.

**Definition decision (2026-09-23, Matt):** `M_i^act` stays a **dose per commute** —
"expected dashcam/ACE encounters on one trip to work" — not a per-km or per-minute
intensity. Rationale: every measure should be readable by the public and evoke the lived
experience; a per-km rate is detached from the feeling of being watched on the way to
work. Consequence to state in the paper: the dose scales with commute length, so M2 partly
measures that long commuters from low-`R_i` places (Staten Island: `R_i` 18, `M_dash^act`
39) accumulate more mobile encounters. With the citywide dashcam field M2 = −0.50 (dose)
while corr(`R_i`, `M_dash^res`) = +0.47 (rate at the residence); report both and say
which sentence each supports. See `docs/RELATED_METHODS.md` §8.

## M3 — The incidence-inversion test (the novel one)
**Status 2026-09-23:** computed with captures attributed to the borough where they happen
(per-borough pair columns from `batch od-exposure-mnl`; the earlier work-borough attribution
inflated the statistic to 85%). Dashcam: **59.5%** of commute captures happen outside the
bearer's home borough; Manhattan's streets' captures are 77% borne by non-Manhattan residents;
Bronx residents get 21% of their commute captures in the Bronx, Staten Islanders 9%. Details and
matrix: `docs/RELATED_METHODS.md` §8.1; JSON `results/incidence_inversion.json`.

For each BG *j* where devices are dense, decompose the exposure it generates by
home BG of the people captured (LODES gives this directly). Expected headline:
"Manhattan's rideshare dashcams generate X% of their captures on residents of the
Bronx and Queens." Residence-based measurement books 100% of that exposure to
Manhattan.

## M4 — Placebo / negative control
Fire hydrants (not street trees — themselves racialized) should show no gradient
after commercial-density adjustment. Cheap; inoculates §6.1. Never built.

Related: [[open-work-before-submission]] · [[camera-layers]] · [[measurement-paper-thesis]]
