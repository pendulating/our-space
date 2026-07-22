# Mobility-aware disparities in observed fixed-camera surveillance — research design

**Status:** pre-registration, now executed (2026-07). Results live in `data/derived/results/` (JSON) and the generated LaTeX tables; §12 tracks build progress.
**One line:** Estimate how the racial/economic composition of a population's *activity
space* relates to its expected fixed-camera surveillance exposure, net of the legitimate
non-demographic drivers of camera placement — decomposed into **residential**,
**mobility**, and **mode** components.

This scopes the empirical claims to the **observed** surveillance layer (fixed cameras)
and uses the `our-space` engine as the *exposure measurement instrument*; the
econometrics happen outside the sim.

---

## 0. Epistemic stance (the load-bearing scope decision)

The `our-space` layers are not epistemically equal. This paper uses only the **observed**
layer for empirical claims:

- **Observed (measurement):** fixed cameras — Amnesty Decode NYC, Dahir et al., DeFlock
  ALPR, DOT — plus ACS demographics and LODES commute flows. A disparity found *here* is
  a finding.
- **Modeled (routing/choice):** how people traverse their activity space (routes, mode
  choice). These are standard, calibrated models, reported with sensitivity — they change
  the *fidelity of exposure along observed flows*, not the underlying observed flows.
- **Speculative (assumption-driven):** Tesla / delivery-robot / smart-glasses layers.
  **Excluded from empirical claims.** Their disparities are circular (spawn ∝ registration
  density → "discover" income gradient). Keep them for a separate prospective/scenario
  paper.

**Tiering of the mobility evidence** (preserves the credibility argument):

- **Tier 1 (primary):** LODES *observed* home→work OD, multimodally routed, with a
  calibrated mode-choice split. Empirical claims live here.
- **Tier 2 (robustness):** gravity/IPF-synthesized non-work trips to broaden the activity
  space. Used only to test whether Tier-1 conclusions hold when the activity space widens.

---

## 1. Estimand and causal stance

Observational with confounding control — **not** causal-from-randomization. Target:

> The association between the racial/economic composition of a population's activity
> space and its expected fixed-camera exposure, net of legitimate non-demographic
> placement drivers, decomposed into residential vs. mobility vs. mode channels.

Anchoring DAG: `demographics → residential/work sorting → activity space (+ mode) →
exposure`, with camera placement driven by {commercial activity, transit, road hierarchy,
crime-justification, possible demographic targeting}. The disparity is the
demographics→exposure path net of the *legitimate* placement channels — and the analysis
turns on which channels count as legitimate (see §5, the crime-control ladder).

**Novelty:** not "cameras near where group X lives" (the residential-proximity
literature) but "group X's daily travel — and the *modes* it can afford — route it through
more surveilled space." The headline is a **decomposition**, not a single coefficient.

---

## 2. Exposure instrument (multimodal, residential + activity-space)

**Unit of analysis:** residential **census block group** (2020 vintage),
population-weighted (~6,500 BGs in NYC → good N for spatial models). Ecological; no
individual-level claims.

Built from engines `our-space` already has (occlusion-aware FOV, cross-source group-dedup,
recall correction), extended to be **mode-specific**:

- **Residential exposure `R_i`** — walkshed engine from BG population-weighted block
  centroids; integrate exposure over the isochrone. "Surveillance where you live."
- **Activity-space exposure `A_i`** — for residents of BG *i* working in BG *j*:
  exposure along the *i→j* **itinerary** + destination-*j* walkshed, aggregated over *j*
  weighted by job-flow mass, and over **mode** (§3): `A_i = Σ_j flow_{ij} Σ_m P(m|i,g)
  · exposure_m(itinerary_{ij})`. "Surveillance where you go, by how you get there."
- **Composite** `E_i = w_home·R_i + w_commute·(route exposure) + w_work·A_i^dest`, with a
  time-budget prior on weights (e.g. 14/1/9 hrs); report sensitivity across weightings.

**Mode-specific exposure profiles** (this is why mode matters and why SubwayBuilder's
model helps):

- **Walk** — street-level FOV encounters along the pedestrian path (your walkshed/route).
- **Drive** — ALPR + DOT traffic-camera encounters along the road path; minimal
  pedestrian-FOV dwell.
- **Transit (subway/bus)** — access/egress walks **plus station/platform camera dwell**.
  NYC subway stations are a major fixed-camera concentration, so a subway commute has a
  *structurally different* exposure profile than a car commute. This is invisible to a
  drive-only routing model and is a first-class reason to add transit routing.

**Primary metric:** expected **distinct fixed cameras encountered per representative day**
(additive, interpretable). **Robustness metrics:** P(≥1 capture)/trip; dwell-weighted
camera-seconds. Compute **per camera type** (CCTV / ALPR / DOT traffic) — the type split
is the sharpest identification lever (§5).

---

## 3. Mobility model (LODES backbone + gravity extension + mode choice + transit routing)

*Informed by the SubwayBuilder simulation
(https://www.subwaybuilder.com/simulation), which applies a distance-based gravity
workplace assignment on LODES + UK census with income-heterogeneous utility mode choice
and RAPTOR multimodal routing. Its public writeup is light on formulas (it cites RAPTOR
and mode-choice papers rather than disclosing equations), so the specification below uses
the standard literature that approach implies.*

### 3a. Trip distribution — observed OD, gravity as extension

- **Tier 1 backbone:** LODES home→work OD is *observed*; use it directly. No gravity needed
  for commuting.
- **Tier 2 extension (non-work trips):** where LODES is silent (shopping/other), synthesize
  trips with a **doubly-constrained (production–attraction) gravity model** — the
  Wilson entropy-maximizing form:

  `T_ij = A_i O_i · B_j D_j · f(c_ij)`,
  `A_i = 1 / Σ_j B_j D_j f(c_ij)`, `B_j = 1 / Σ_i A_i O_i f(c_ij)` (solved by IPF /
  Furness balancing),

  with **impedance** `f(c_ij) = exp(−β c_ij)` (exponential; test a power/gamma form as
  robustness). Productions `O_i` = ambient population (LODES RAC); attractions `D_j` =
  destination opportunities (retail/commercial floor area from MapPLUTO; job counts from
  LODES WAC by sector). **Calibrate β** to reproduce the observed commute trip-length
  distribution (from LODES-derived network distances, or NHTS/NYC region travel-survey
  trip lengths). Non-work trips are Tier 2 only — a robustness widening of the activity
  space, never a primary empirical estimand.

### 3b. Mode choice — income-heterogeneous utility (the key upgrade)

Different groups can afford different modes, and modes carry different exposure profiles →
**mode is a mediation channel for the disparity.** Multinomial logit over
`m ∈ {walk, drive, transit}`:

`U_{m|i,j,g} = −α·( VOT_g · time_m(i,j) + cost_m(i,j) ) + ASC_m + ε`,
`P(m|i,j,g) = exp(U_{m|i,j,g}) / Σ_{m'} exp(U_{m'|i,j,g})`,

with **value of time `VOT_g` by income group `g`** (per ACS block-group income), mode
times from §3c routing, and costs (fare, fuel/parking, none for walk). Calibrate ASCs to
reproduce ACS **B08301 commute-mode shares** by BG (a doubly-constrained fit on the
observed mode marginals). This yields a demographically-patterned mode split, so exposure
decomposes by mode (§5b) — the sharpest novel result the drive-only model cannot produce.

### 3c. Routing — multimodal itineraries (RAPTOR for transit)

- **Walk / drive:** existing A* on the pedestrian / drive graph.
- **Transit:** **RAPTOR** (Round-Based Public Transit Routing, Delling et al.) over the NYC
  **GTFS** feeds → itineraries with access/egress walks, in-vehicle segments, transfers,
  and **station dwell** (which drives station-camera exposure). Time-of-day departure
  distribution "similar to real life" (per SubwayBuilder) — though fixed cameras are 24/7,
  so departure timing mainly affects *which* itinerary/mode is chosen, not camera presence.

**Routed since 2026-07-15 (frequency-based, not RAPTOR).** Transit legs are now actually routed — but over
a **headway-based GTFS subway graph** (`sim_core::subway`, baked by `data-pipeline bake-subway
data/snapshots/gtfs/subway data/derived/subway_nyc.ossub`), not RAPTOR: weekday AM-peak headway/2 waits
(capped 15 min), median observed run times, same-station transfer overheads, and a synthesized Staten
Island Ferry link so all 245,520 station pairs are reachable. Exact **boardings** set the station-camera
complement (n = boardings − 1, capped 3) and `t_transit` comes from the timetable. Full RAPTOR / R5-on-JVM
was considered and rejected — Rust-first pipeline, a frequency-based expectation is the right model for a
*representative* commute, and the all-pairs matrix is O(1) per OD pair. Crow-flies × circuity survives only
as a fallback when no `.ossub` is baked. **Same-day v2 (2026-07-15):** the graph edges are now
**common-lines bundles** (Chriqui–Robillard, α=0.5) — the rider boards the first arrival among the
attractive line set, so waits pool over the *combined* frequency at each boarding/transfer rather than per
line (resolves the earlier per-line-wait approximation; `commute_subway` 6.62 → 6.85, p_transit unchanged).
And the Staten Island Ferry is now parsed from the **real NYC DOT ferry GTFS** (route SIF, ~117 weekday
trips, 25-min crossing; `tools/fetch_gtfs.py`, browser UA + Mobility-Database mdb-518 mirror) rather than
the hand-parameterized 15/25-min pseudo-route, which now survives only as a loud-warning fallback. See §12
and OUTLINE §9 (2026-07-15).

Exposure per itinerary = Σ over legs of leg-appropriate exposure (walk legs → pedestrian
FOV; drive legs → ALPR/traffic; transit legs → station/platform dwell + access/egress
walk FOV).

---

## 4. Data and the joins (concretely)

| Layer | Source / table | Geography | Join key / note |
|---|---|---|---|
| **Cameras** | merged Amnesty + Dahir + DeFlock + DOT (already cross-source deduped) | points (lat/lon) + type | point-in-polygon → 2020 TIGER block groups |
| **Demographics** | ACS 5-yr (2018–22): B03002 (race/eth), B19013 (income), B05002/B05012 (nativity), B08301 (commute mode), B25003 (renter) | block group | 12-digit GEOID |
| **Commute OD** | **LODES8 OD** — `ny_od_main_JT00` **and** `aux` (aux = essential for NJ/CT→NYC commuters) | census **block** (15-digit) h_/w_geocode | block→BG via GEOID prefix / LODES xwalk |
| **Wage-tier flows** | LODES **SE01** (earnings ≤ $1,250/mo) | block | low-wage workers' activity-space exposure straight from LODES |
| **Placement controls** | NYC **MapPLUTO** (commercial FAR/land use), LODES **WAC** (job density), GTFS stops, drive-graph intersection density, **BID** boundaries | lot / point → BG | spatial aggregate to BG |
| **Contested control** | NYPD complaints (Socrata) and/or 311 | point → BG | see §5 — report with and without |
| **Transit network** | NYC **GTFS** (subway + bus) | routes/stops | RAPTOR itineraries (§3c) |
| **Ambient population / attractions** | LODES RAC/WAC + MapPLUTO retail floor area | BG | gravity productions/attractions (§3a) + need-neutral baseline |

**Vintage trap (flag now):** LODES8 is on 2020 blocks — use 2020 TIGER + 2020-vintage ACS
BGs + 2020-consistent camera geocodes so cameras, demographics, and flows land on one
geometry. Mismatched vintages are the #1 silent bug.

---

## 5. Identification strategy

Three complementary moves, reported side by side.

### 5a. Need-neutral counterfactual (least spec-dependent)
Redistribute the fixed cameras ∝ ambient population (or foot traffic), total count fixed;
each group's **excess exposure** = actual − need-neutral. Reframes the result from a
coefficient on `%Black` to "group X experiences N% more exposure than need-neutral
placement predicts" — interpretable and robust to regression-spec disputes.

### 5b. Residential vs. mobility vs. **mode** decomposition (the novel contribution)
Decompose `E_i` into `R_i` (residential), the routed mobility term, and the **mode-mediated
share** (how much disparity flows through groups taking different modes with different
exposure profiles). Publishable hypotheses:
- For low-wage / minority commuters flowing into commercial cores, **mobility amplifies**
  exposure beyond residential.
- The disparity is **mode-mediated**: e.g. transit-dependent commuters accrue
  station-camera exposure that car commuters avoid (while car commuters accrue ALPR
  exposure) — a channel the residential-proximity literature structurally cannot see.

### 5c. The crime-control ladder (do not bury)
Controlling for crime/311 risks laundering the disparity (placement "justified by"
racialized enforcement data). Report a ladder:
1. total disparity (no crime control),
2. net of commercial / transit / land-use only,
3. additionally net of crime.

Interpret (2)→(3) as the share of disparity that *runs through* the crime-justification
channel (explicit mediation reading). The no-crime-control estimate is a headline number,
not a footnote.

### 5d. Negative controls / placebos
- **Type contrast:** if the gradient concentrates in **ALPR** (enforcement-linked) vs.
  **DOT traffic cams** (infrastructure-linked), that's a mechanism-relevant result and a
  built-in placebo.
- **Placebo outcome:** a genuinely neutral street feature (fire hydrants — *not* street
  trees, which are themselves racialized) should show no gradient after commercial-density
  adjustment.

---

## 6. Estimator, MAUP, uncertainty

- Exposure and demographics are both strongly spatially autocorrelated → OLS SEs invalid.
  Default: **Spatial Durbin Model** (nests lag/error; includes spatial lags of covariates)
  with queen-contiguity **W**; select via Anselin LM tests; report **Moran's I** on
  outcome and residuals; report the **direct / indirect (spillover) effects**
  decomposition. Cross-check with **Conley spatial-HAC** SEs so results aren't a
  spatial-spec artifact. (R: `spatialreg`/`spdep`; Python: `pysal`/`spreg`.)
- **MAUP:** re-estimate at tract and a 250 m hex grid; report the coefficient *range*.
- **Uncertainty:** bootstrap over (i) the undercount/detection draws (§7), (ii)
  route/endpoint sampling, (iii) mode-choice parameters + time-budget weights. No
  speculative-sensor parameters here, so the uncertainty budget is tractable and honest.

---

## 7. Undercount model (turn the biggest weakness into a method contribution)

The crowdsourced census undercounts, plausibly correlated with the outcome — the sharpest
reviewer objection. But `our-space` already **groups co-located cameras across independent
sources** (Amnesty / Dahir / DeFlock / DOT) → a **capture–recapture** design. Fit a spatial
**N-mixture / occupancy** model treating each source as an imperfect enumeration to
estimate *true* camera intensity per area with spatially-varying detection. Report
disparities under (a) raw, (b) global recall (current), (c) spatially-varying recall, and
show the estimate's movement. Converts the confound into a modeling contribution, and the
multi-source grouping is the ingredient most surveillance papers lack.

---

## 8. Compute plan in the existing system

The engine covers the hard part; add batch jobs that emit one tidy BG-level table:
1. **Walkshed-exposure per BG** (population-weighted block sample).
2. **Itinerary-exposure over LODES OD** — top-K destinations per origin capped by flow
   mass, per mode (walk/drive A*; transit RAPTOR on GTFS); reuse the A*+exposure batching
   already built for taxi routes.
3. **Mode split** per (i, j, income group) from §3b.
4. **Emit** `{GEOID, R_i, A_i, E_i, exposure_by_type, exposure_by_mode, controls…}` → hand
   to R/Python for §6. Keep the sim as the exposure *instrument*; do econometrics outside.

New engineering vs. reuse: transit routing (RAPTOR + GTFS) and the mode-choice layer are
the genuinely new pieces; walkshed/route/exposure and OD batching already exist.

---

## 9. Results structure (headline shape)

1. Overall fixed-camera exposure disparity by race / income / nativity (with §7 undercount
   bands).
2. **Decomposition:** residential vs. mobility vs. mode (§5b) — does mobility amplify?
   which modes mediate?
3. **Excess over need-neutral** placement by group (§5a).
4. **Crime-ladder** (§5c): total vs. net-of-commerce vs. net-of-crime.
5. **Type contrast** (ALPR vs. traffic vs. CCTV) as mechanism + placebo (§5d).
6. Robustness: MAUP, spatial-spec, undercount, time-budget/mode parameters, Tier-2 non-work
   trips.

---

## 10. Limitations to preempt

Fixed cameras only (speculative layers excluded from empirical claims); exposure = physical
FOV encounter *opportunity*, not actualized surveillance harm; LODES commuting is a
*partial* activity space (misses non-work travel — mitigated by Tier-2 gravity synthesis —
and carries LODES imputation/coverage caveats, incl. federal/informal work); mode choice is
a calibrated model, not observed itineraries; associational not causal (placement isn't
randomized); undercount modeled not eliminated; ecological (BG-level — no individual
claims).

---

## 11. Modeling lineage / references

- **Gravity / entropy trip distribution:** Wilson (1967), *A statistical theory of spatial
  distribution models*; Furness (IPF) balancing.
- **Mode choice:** Ben-Akiva & Lerman, *Discrete Choice Analysis*; income-heterogeneous
  value-of-time.
- **Transit routing:** Delling, Pajor, Werneck (2015), *Round-Based Public Transit Routing
  (RAPTOR)*.
- **Applied comparable:** SubwayBuilder simulation
  (https://www.subwaybuilder.com/simulation) — distance-based gravity workplace assignment
  on LODES + UK census, income-heterogeneous utility mode choice, RAPTOR routing.
- **Spatial econometrics:** Anselin, *Spatial Econometrics*; LeSage & Pace (spatial Durbin,
  direct/indirect effects); Conley spatial-HAC SEs.
- **Data:** US Census LEHD **LODES8** technical documentation; ACS 5-year; NYC MapPLUTO;
  NYC GTFS.
- **Surveillance / camera census:** Amnesty International *Decode Surveillance NYC*; Dahir
  et al.; DeFlock (ALPR).
- **Sensing-power anchor (already in the sim):** O'Keeffe et al. (2019), *PNAS*.

---

## 12. Implementation status (2026-07)

*Numbers below are as-of-run logs; the canonical current values live in `data/derived/results/*.json`.*

Concrete build progress against the plan. Everything here operates on the **observed** layer
(§0): merged Amnesty/Dahir/DeFlock/DOT fixed cameras, cross-source group-deduped, recall-
corrected.

**Architecture rule:** the exposure instrument, the exposure-table generation, and every
simulation/estimation-by-simulation step (gravity/IPF trip synthesis, mode-choice MNL,
need-neutral counterfactual redistribution, capture–recapture undercount draws, bootstrap
resampling) live in **Rust** (`sim-core` / `batch`) — the compute core. **Python** is only for
data acquisition (`tools/fetch_*.py`, I/O + duckdb) and the downstream spatial econometrics
(Durbin/Moran/Conley via pysal/spreg) where the ecosystem is worth it.

**Done**

- **Five-borough drive graph** (`graph_nyc.osgraph`, 74.7k nodes / 111.8k edges). Fixed a
  gap that had silently dropped **all of Staten Island**: CSCL omits the Verrazzano's ~587 m
  centre suspension span, so SI was a disconnected 11.9k-node component that the keep-largest
  prune discarded. Two changes in `graph_osm::bake_cscl`: (a) `keep_components_above` retains
  every street-network component ≥50 nodes (not just the largest), so real islands stay in for
  local residential exposure; (b) `stitch_bridges` reconnects an island across a real vehicular
  bridge split by a centre-span digitization gap (bridge-class nodes both ends, ≤700 m) — the
  Verrazzano rejoins SI to the routable network. Verified: SI block-group centroids now snap to
  SI streets (~50 m; was 3–20 km across the harbour) and SI→mainland drive routes succeed
  (no-path fell from ~62% of SI commutes to 0.2%). Ferry-only Governors Island stays a separate
  component (correct — no bridge).
- **Residential exposure `R_i`** — `batch bg-exposure`. Real inputs wired: **2020 Census
  population-weighted block-group centroids** for all five counties (`tools/fetch_census.py` →
  `bg_centroids_nyc.csv`, 6,587 BGs / 8.80 M residents, keyed by 12-digit **GEOID**). Output
  `data/derived/exposure/R_i_bg_nyc.csv`. Face-valid gradient (pop-weighted cameras/10-min
  walkshed): Brooklyn 90.7, Manhattan 87.7, Bronx 71.1, Queens 56.7, **Staten Island 13.5**.
- **LODES-OD backbone** — `tools/fetch_lodes.py`-style fetch of **LODES8 `ny_od_main_JT00`
  (2022, 2020 blocks)**, duckdb aggregation block→BG → `bg_od_nyc.csv` (1.87 M NYC→NYC BG
  pairs; captures 85–93% of each borough's commute flow within the drive graph).
- **Drive-mode activity-space exposure `A_i^drive`** — `batch od-exposure`. Per home BG,
  routes top-K work destinations by flow over the drive graph, counts road-facing cameras
  (ALPR + DOT + enforcement) capturing the path + destination-BG walkshed (= R_j, cached),
  flow-weighted. Output `data/derived/exposure/A_i_drive_bg_nyc.csv`. Early signal (Staten
  Island): **A_i^drive ≈ 98 vs R_i ≈ 13.5** — mobility amplifies exposure ~7× as SI's
  car-dependent commuters drive into dense cores (plan §5b). Commute flows are dispersed
  (median 281 dest BGs/home BG) so K trades coverage for compute (top-100 ≈ 61% flow; per-BG
  `jobs_covered/jobs_total` recorded for robustness weighting).
- **ACS demographics** — `tools/fetch_census.py acs` pulls ACS 5-yr (2022) BG-level race
  (B03002), income (B19013), commute mode (B08301), tenure (B25003), poverty (B17001) for all
  five counties → `census/acs_nyc.csv` (6,807 BGs × 16 vars, GEOID-keyed). First disparity
  signal (R_i × ACS): pop-weighted R_i falls **84 → 57 cameras** from the poorest to the fourth
  income quintile (Q5 dense-core rebound to 72); corr(R_i, %transit-commute) +0.33.
- **Mode-weighted A_i** — `batch od-exposure-modal` (Rust) adds the mode split. **Key modeling
  point (REVISED):** the street census (Amnesty/Dahir/DeFlock/DOT) covers a subway rider's
  **access/egress walks** (home→nearest station, dest station→work), but the underground line-haul
  is **NOT invisible** — the MTA has cameras in *every station and every subway car*
  (mta.info/document/178926). Each transit trip accrues guaranteed MTA-system captures via
  `SubwayParams::cameras(line_haul)`: distinct on-path cameras = `cams_station`·(2 endpoints + n
  transfers) + `cams_train`·(1 + n trains), n = transfers estimated from subway line-haul distance
  (one per `km_per_transfer`, capped). Central defaults (station 3, train 2, 12 km/transfer, cap 3)
  → no-transfer trip 8, +1-transfer 13, **pop-weighted effective ≈ 12 cameras/trip**. Knobs are
  env-overridable; `OURSPACE_SUBWAY_SCALE` multiplies for sensitivity. Both models emit
  `commute_subway` (flow-wtd subway contribution to the commute leg), so any camera assumption is
  reconstructable from one bake — `tools/sweep_subway_cameras.py` sweeps scale ×k and flat-s without
  re-running. Still **no RAPTOR** (station locations from `tools/fetch_gtfs.py`, 496 stations,
  suffice; we count entry/train/exit, not the routed line). Grounding (pop-wtd): drive commute leg
  22.0, dest walkshed 99.9. **Effect (central model):** the earlier "transit invisible" mode
  *reversal* largely DISSOLVES — A_mnl by income quintile is now nearly flat (Q1 121.8 / Q5 122.9),
  corr(A_mnl, income) ≈ +0.07–0.09 (was +0.17 at floor s=3), %Hispanic flips to slightly
  over-exposed (+0.05), residual %White +0.12 from the destination-core effect. Sensitivity
  (flat-s): %Hispanic crosses 0 at s≈10.5, income at s≈42, %White at s≈76. So mobility + the fact
  that *all modes are surveilled* equalizes the residential disparity to near-neutrality in activity
  space; the sharp, robust disparity is the residential *placement* bias (§5a), not lived activity.
  **Routed (2026-07-15):** `n` is no longer estimated from subway line-haul distance — it is the **routed
  boarding count** (n = boardings − 1, capped 3) from a headway-based GTFS subway graph (`sim_core::subway`,
  `data-pipeline bake-subway`), and `t_transit` is the timetable travel time. `SubwayParams::cameras(...)`
  and the linear `commute_subway` reconstruction are unchanged; only the source of `n` (hence the per-trip
  complement) changed — `commute_subway` mean 7.34 → 6.62 (→ 6.85 after the same-day v2
  common-lines/real-ferry upgrade), sweep central 12.9 → 11.5 (→ 11.9 v2) cams/trip, and every
  non-transit number byte-identical (R_i untouched). See OUTLINE §9 (2026-07-15) and
  `data/derived/results/README.md`.
- **Income-heterogeneous mode choice (MNL)** (§3b) — DONE. `batch od-exposure-mnl` → `A_i_mnl_bg_nyc.csv`.
  Multinomial logit over `{walk, drive, transit}`, utility U_m = −(VOT·time + cost) + ASC_m, income-VOT
  from B19013. KEY: a single global ASC only pins the citywide marginal and lets pure VOT invent a
  monotonic income→drive gradient (Q1 0.03 → Q5 0.71) that contradicts NYC's real non-monotonic pattern.
  Fix: calibrate ASCs **per income quintile** (incremental-logit, 5 groups) to each group's observed ACS
  split → VOT then only shapes within-group distance substitution. Reproduces the real drive share
  (rises Q1→Q4 0.25→0.41, FALLS at Q5 to 0.30 as the rich shift to walk 0.17). Disparity: A_mnl tilts
  income +0.17 / %white +0.20 — BETWEEN observed-shares A_modal (+0.09/+0.10) and broken pure-VOT
  (+0.27/+0.25); reversal real but MODERATE.
- **Need-neutral counterfactual (§5a)** — DONE. `batch counterfactual` → `counterfactual_bg_nyc.csv`
  (analysis `tools/analyze_counterfactual.py`). Redistributes the fixed street-camera stock so intensity
  ∝ need (population, or pop+inbound-jobs foot-traffic); R_neutral_i ∝ need reachable within i's walkshed,
  scaled to conserve the pop-weighted mean R (so Σpop·excess = 0). RESULT: Black-plurality BGs carry
  **+11% / +34%** more than need-neutral (pop / ambient proxy), Hispanic **+7% / +25%**, White **−10% /
  −25%**; income Q5 **−14% / −41%**. Sign robust across proxies; magnitude bounded by proxy choice.
  Reframes the coefficient as "group X carries N% more exposure than need-neutral placement predicts."
- **Exposure table** — `batch exposure-table` (Rust) joins centroids + R_i + A_i^drive + A_modal + A_mnl
  + ACS → `data/derived/exposure/exposure_table_nyc.csv`, one row per BG with composite `E_i` and `E_i_mnl`
  (mode-weighted commute leg; near-identical since E_i is home-weighted 14:1:9), for the Python econometrics.
- **Crime + 311 control ladder (§5c)** — DONE. `batch covariates` (Rust) computes per-BG walkshed
  densities (crime + felony from NYPD YTD `tools/fetch_crime.py`; 311 public-disorder from
  `tools/fetch_311.py`, 855k pts; inbound jobs, ambient pop, transit access) → `covariates_bg_nyc.csv`.
  `tools/crime_ladder.py` runs focal-demographic pop-weighted WLS ladders (rung1 total → rung2
  +land-use → rung3 +crime → rung4 +crime+311). RESULT: %Hispanic carries the sharp residential
  disparity (+9.7 cameras/SD, no controls); even net of land-use + crime + 311, **+2.8 remains
  unexplained** (±0.53). BOTH mediators are racialized (crime ~ %Hisp +0.31 / 311 ~ %Hisp +0.28;
  both −0.15…−0.29 with %White), so controlling for either launders the disparity — the attenuation
  is what the crime/disorder narrative *can rationalize*, not what is legitimate. Lead with the
  no-control number.
- **Undercount / capture-recapture (§7)** — DONE, and **REVISED 2026-07-13 (the earlier reading was
  a units artifact — see below).** `tools/capture_recapture.py`: two independent CCTV censuses
  (Amnesty crowdsource × Dahir ML, raw pre-dedup points, 50 m match) → Chapman N̂. Census recall
  **≈50.1%** (N̂ 28,643 true sites vs 14,357 observed; 95% boot [45.8, 54.4]%); detection nearly
  UNIFORM across boroughs (40.6–53.0%). Conservative lower bound (shared-GSV positive dependence
  biases N̂ down). (a) DETECTION MODEL (`tools/undercount_spatial.py`) — Logit(Amnesty detects a
  Dahir-known camera) ~ demographics+density: NO significant demographic gradient (%Hisp p=0.89,
  %Black p=0.89, income p=0.42) → the undercount is **NON-DIFFERENTIAL**, not racialized.

  ⚠️ **(b) PROPAGATION — the "correction DOUBLES the disparity" claim was WRONG.** It said raw +9.5 →
  corrected +20.0 cameras/SD and read that as "the undercount was masking the disparity." But the
  correction rescales the *outcome* by ~1.94×, and a coefficient denominated in **cameras/SD** must
  grow with it. `tools/sweep_recall.py` decomposes it:

  | | coef (cameras/SD) | ratio | correlation (scale-free) | **genuine change** |
  |---|---|---|---|---|
  | **%Hispanic** | +9.47 → +19.65 | 2.08× | +0.202 → +0.214 | **+6.3%** |
  | %White | −4.17 → −10.26 | 2.46× | −0.089 → −0.112 | +26.0% |
  | income | −2.56 → −7.58 | 2.96× | −0.055 → −0.083 | +51.6% |
  | %Black | −1.51 → −1.48 | 0.97× | −0.032 → −0.016 | **−50.1%** |

  The outcome's SD grows **1.95×**. So ~95% of the %Hispanic "doubling" is a **unit change, not a
  finding** — and this is *internally consistent* with (a): a non-differential undercount is a
  near-uniform rescaling, and a near-uniform rescaling **cannot create or destroy a relative
  disparity**. **The honest claim is a ROBUSTNESS result, not an amplification result:** the
  disparity is not an artifact of differential undercounting, and correcting for it does not
  meaningfully change it.

  The *residual* differential (income +52%, %White +26%, %Black −50%) is real but has a mundane
  mechanism: the correction inflates only **CCTV-census-only** groups, and the *surveyed* layers
  (DOT/ALPR/enforcement) that mark a group "confirmed" are **Manhattan-concentrated** — so Manhattan
  gets less inflation and the income/white gradient steepens. ⚠️ **Treat with suspicion until the
  enforcement layer is re-fetched citywide** (`fetch_enforcement.py` currently filters
  `borough='Manhattan'`), since that coverage bug feeds directly into `confirmed`.

- **Recall — architecture (2026-07-13).** The instrument now reports **OBSERVED** counts (`R_i` =
  `cameras_raw`, default `r = 1.0`), never a silently-corrected estimate. `sim_core::DAHIR_RECALL`
  (0.63) is the **ML detector's** recall and is the *wrong number* for a crowdsourced census — it is
  now documented as such and is not applied to the merged layer. The right number is
  `sim_core::CENSUS_RECALL` (0.501, from capture–recapture). Because the correction inflates only the
  CCTV-census-only sub-population, it is **linear**, so the batch emits `R_i_unconfirmed` and *any*
  recall — including all 2,000 bootstrap draws — is reconstructable downstream with **no re-run**:
  `R(r) = unconfirmed/r + (raw − unconfirmed)`. Pinned by
  `recall_correction_is_linear_and_reconstructable_from_one_bake`. This is what lets the recall CI
  propagate into the coefficient properly instead of baking a point estimate into every number.
- **§6 spatial econometrics + MAUP** — DONE (`tools/spatial_econometrics.py`, uv env; real TIGER-2020
  BG polygons `data/snapshots/tiger/`, Queen contiguity with islands attached). Moran's I R_i **+0.89**
  (p=.001), E_i +0.91, A_mnl +0.83 → OLS SEs invalid. Anselin LM (robust-error ≫ robust-lag → error
  process). **Full ML suite by AIC** (R_i): OLS 54867 > SAR 46349 > SEM 45435 > SDM 45380 > **SDEM
  45377 (selected)** — the Spatial Durbin *Error* model wins, confirming the LM diagnostic. Impacts
  under SDEM (credible; no ρ-explosion): %Hisp on R_i direct +1.5 / indirect (Wβ) +4.2 / total **+5.7**
  cameras/SD (SDM's ρ=0.91 gives an inflated +12.5). Crime+311 ladder under **Conley HAC SEs** holds
  (%Hisp +3.67 ±0.58 survives crime+311; crime +9.5, 311 +12.2). **MAUP**: %Hisp coefficient stable —
  BG +6.98, tract +7.50, 1 km grid +7.42 → NOT a scale artifact.

**Pending / refinements**
- **RAPTOR transit routing (§3c)** — *superseded 2026-07-15*: transit legs are now routed over a
  **frequency-based GTFS subway graph** (`sim_core::subway`, `bake-subway`) that supplies boardings-driven
  station-camera dwell + timetable travel times without RAPTOR/R5. Full RAPTOR remains unneeded for exposure.
- **Broader 311 / housing-quality proxies** — current 311 is the public-disorder subset; a full-311 or
  housing-maintenance robustness could be added.
- **uv env** — Python spatial stack (geopandas/libpysal/esda/spreg/statsmodels) in `pyproject.toml`;
  run spatial scripts via `uv run python tools/...`. Pure-numpy scripts still run on system python.
