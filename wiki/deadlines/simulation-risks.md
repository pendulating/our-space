---
title: Simulation Risks and Integrations
created: 2026-08-23
updated: 2026-08-23
type: query
tags: [result, method, facct27]
sources: [crates/sim-core/src/mobile.rs, crates/sim-core/src/simulation.rs, crates/batch/src/main.rs, docs/surveillance-exposure-disparity-plan.md, facct27_digital_eyes_on_the_street/OUTLINE.md §6.4, raw/papers/okeeffe-sensing-power-2019.md]
confidence: medium
---

# Simulation Risk Register + O'Keeffe Integration Notes

Two halves: (A) the largest present problems with the simulation component that
powers the paper (`sim-core` + `batch`), ranked by threat to the FAccT submission;
(B) integration points from O'Keeffe et al. 2019 ([[okeeffe-sensing-power]]),
whose full text is captured in `raw/papers/okeeffe-sensing-power-2019.md`.

## A. Risk register

### P0 — blocks a claimed contribution

**A1. Mobile classes are not wired into the paper's instrument.** — **M1 code + M2/M3 analysis shipped 2026-08-23 (commits bc8371d, f671c5e); full re-bake pending.**
Handoff runbook for the re-bake: `docs/REBAKE_HANDOFF.md` (SLURM only, never the login
node; new columns to expect; post-bake steps).
`MobileLayers::load()` (`batch/src/main.rs`) loads ACE + dashcam into `bg-exposure`
(`m_ace_res`, `m_dash_res`), `od-exposure*` (`m_ace_act`, `m_dash_act`);
`exposure-table` propagates them into the joined table; `OURSPACE_EMIT_PAIRS`
emits per-pair rows for M3. Analyses ready: `tools/compounding.py` (M2),
`tools/incidence_inversion.py` (M3), both wired into `refresh_results.sh`.
Old-vs-new comparisons: fixed-camera columns byte-identical everywhere.
Sweep knobs: `OURSPACE_DASH_PEN`, `OURSPACE_DASH_CAP`.

**A2. The subway camera complement is three hand-picked constants — and a
headline can flip on them.**
`SubwayParams` (`batch/src/main.rs:404–433`): 3 cams/station, 2/train,
transfers by distance or boardings cap. Not a measurement. The sweep shows
correlations flip sign at s ≈ 27.6 (%Hispanic) vs central model ~12; the tiny
`A_mnl` %Hispanic residual (−0.77) has an unidentified sign across the plausible
range. Any independent count of MTA station/car cameras hardens the entire
equalization result ([[results-equalization]]).

### P1 — robustness vulnerabilities

**A3. Occlusion null holds only below ~60 m assumed range.**
Attenuation 0.42% at 60 m, 5.5% at 120 m, 12.5% at 240 m ([[occlusion-null]]).
Defense is "a camera that can identify a person" — an assumption about capability,
not a measurement; hardware improves. Argue the boundary explicitly in §4.

**A4. Recall ≈ 0.50 is a conservative lower bound with a shared-modality confound.**
Both censuses derive from Street View → positive dependence inflates overlap →
N̂ biased down ([[capture-recapture-undercount]]). Absolute exposure levels are
floors. Residual differential inflation traces to DeFlock's Manhattan
concentration — disclosed, still a known bias in the `confirmed` flag.

**A5. Heading-expectation treatment must be verified as primary everywhere.**
The mixture weight exists and is unit-tested (`scenario.rs`,
`census_camera_weight_is_the_heading_mixture`), but confirm every batch path
consumes weighted groups; otherwise code and disclosure disagree (~12% CCTV
over-count).

### P2 — real but manageable

**A6. `R_i` is time-invariant** while the app and heatmap are diurnal.
Defensible for fixed 24/7 CCTV; DOT monitoring and enforcement are
schedule-sensitive. App-vs-paper number comparisons will diverge.

**A7. One Tuesday of TLC data (2024-06-25)** drives all rideshare intensity;
penetration 0.40 and capture 0.40 unvalidated. Fine while Tier C; becomes
load-bearing when A1 ships — ship the sweep band with it.

**A8. `E_i` composite weights (14/1/9)** are an arbitrary time-budget prior.
Least defensible measure; consider demoting to appendix.

**A9. Reproducibility hygiene:** `data/derived/` untracked (no manifest/hashes);
`spatial_econometrics.py`, `undercount_spatial.py` stdout-only.

**A10. License exposure:** Amnesty CC BY-NC-ND inside a released derived
instrument. Give the NoDerivatives rationale its own ethics paragraph
([[civic-tool-ethics]]).

## B. Integration points from O'Keeffe et al. 2019

Their paper computes **coverage**: what fraction of street segments a fleet
scans. We invert to **exposure**: what fraction of a person's day a fleet sees.
Beyond the inversion, seven concrete integrations:

### B1. Ball-in-bin analytic as an independent cross-check of the Poisson rates
Their Eq. 11, `⟨C⟩ ≈ 1 − (1/N_S) Σ (1−p_i)^{B̄·N_V}`, is a closed-form coverage
prediction from segment popularities `p_i`. Our `exposure_rates_per_minute`
Poisson means should be *consistent* with their covering dynamics: if our
dashcam intensity field implied a fleet that covers far more or less of the
network than Eq. 11 predicts for the same trip volume, the field is wrong. Add a
validation harness: compute empirical `p_i` from the baked taxi day, predict
⟨C⟩(N_V), compare against simulated coverage. Cheap, analytic, and it turns
"the intensity field looks plausible" into a quantitative agreement test.

### B2. Zipf-law segment popularities as a diagnostic for our intensity fields
They show taxi segment popularity follows Zipf's law and that a random-walk null
fails qualitatively. Our baked `taxi_day` routes imply their own `p_i`
distribution. Test it: fit the tail exponent; deviation from Zipf flags bake bugs
(zone-centroid snapping concentrates trips on few nodes — exactly the kind of
artifact this would surface). Also usable in the paper: "our replayed fleet
reproduces the documented universal statistics of taxi motion" is a free
fidelity credential citing their Fig. 2D.

### B3. Preferential-return taxi-drive process as a generative prior for scenario layers
Their taxi-drive process (random destination via shortest path, preferential
return q_n ∝ 1 + ε·v_n) generates realistic trajectories from a street network
alone. Use it where we have *no* observed trajectories: the delivery-robot
foresight layer currently spawns on a static Robotability grid; robots moving
via a taxi-drive-like process would give the scenario temporal structure instead
of a per-minute rate at a point. Label stays Tier D; the motion model becomes
defensible.

### B4. Temporal-resolution extension ⟨C*⟩(N_w) mirrors our time-of-day model
They extend coverage to require ≥1 scan in each of N_w daily subintervals
(N_V = 355 = 3% still scans half of Manhattan at N_w = 3). Direct parallel to our
diurnal multipliers and departure-hour scrubber. Integration: report our mobile
exposure rates at multiple N_w-style resolutions so a reviewer can see exposure
is not an artifact of daily aggregation; also cite their numbers as external
evidence that fleets saturate temporally fast — supporting the saturation reading
in [[results-equalization]].

### B5. Their own spatial-bias caveat hands us our motivation sentence
They write that taxi concentration creates inherent spatial bias which "could
have harmful consequences, such as underservicing socioeconomically disadvantaged
neighborhoods" — and defer it to SI. We measure exactly this bias, for
surveillance rather than city-monitoring, distributionally. Quote it in §2:
they flagged the bias; nobody measured who bears it. This sharpens the
[[okeeffe-sensing-power]] inversion beyond coverage→exposure: public-good
framing acknowledged the equity problem and left it open.

### B6. Diminishing-returns numbers quantify why exposure equalizes
~5% of vehicles cover 50% of segments; 10 taxis cover one-third of Manhattan in
a day. These are the mechanism behind [[results-equalization]]: fleets saturate
network coverage so cheaply that any resident's trajectory intersects them at
near-uniform rates. Use in §6.3's "why it happens" paragraph — it converts
saturation from an observation into a consequence of documented fleet physics.
Also note their B̄ estimates are lower bounds, making saturation stronger.

### B7. Host-type taxonomy organizes our fleet contrast
They rank sensor hosts: random-destination taxis have high sensing power;
fixed-route hosts (buses, trash trucks) low. This is precisely our ACE-vs-
rideshare asymmetry ([[ace-vs-commercial]]) restated as physics: ACE buses sweep
predictable corridors (directed, low marginal coverage, high local dose);
rideshare saturates globally (dragnet). Cite for the claim that the commercial
fleets' exposure profile is qualitatively unlike any municipal fleet — and note
their privacy remark ("putting sensors on private cars might lead to privacy
concerns") reads, six years later, as the moment our paper measures.

## Priority order
Do A1 first (unblocks M2/M3, the novel statistics). Then A2 measurement
(hardens the centerpiece). B1+B2 are one-day validation wins that de-risk A7;
B5–B7 are writing moves requiring no new computation.

Related: [[compounding-test-m1-m3]] · [[results-equalization]] · [[open-work-before-submission]] · [[sensor-model]]
