# Related Methods: What the Closest Papers Do

Compiled 2026-09-23 for the FAccT '27 submission. Three literature sweeps (surveillance-disparity
studies; activity-space / NEAP exposure studies; mobile-sensing and exposure simulations), each
compared against what our instrument does. Every DOI below was checked against Crossref or the
publisher page unless marked **[unverified]**; papers read only at abstract level are marked
**[abstract]**. Companion to the simulation/analysis audit of 2026-09-22 (see "Audit context" at
the end) and to `wiki/mocs/related-work-map.md`, which this extends.

**The three things to take away**

1. **Spatial inference.** No paper in the surveillance-disparity literature uses spatial SEs.
   Dahir et al., Sheng et al., and Amnesty report model-based SEs; Chen et al. use robust SEs with
   tract clustering in the SI; only Calacci et al. fit a (county-level) SAR. Correctly implemented
   Conley SEs with a bandwidth sweep would put us *above* the field's bar — but the current
   "Conley" numbers were classical OLS SEs (fixed 2026-09-23, see below).
2. **The Gini collapse is the expected NEAP result, not a finding.** The activity-space literature
   (Kwan 2018; Kim & Kwan 2021; Cai & Kwan 2024) defines dispersion shrinking under mobility as the
   mechanical consequence of averaging over locations — "regression to the mean under spatial
   autocorrelation" (Cai & Kwan). Standard practice is a **time-weighted full-day measure that
   includes home**, computed for **all residents**, with claims made about **group gaps** and
   **who fails to average**, benchmarked against a **null model**. Our R-vs-A Gini comparison
   does none of these.
3. **Simulation constants.** Comparable simulations replace our kinds of constants with observed
   data (speeds, timetables, trip densities) and validate against observed marginals (coverage
   curves, trip counts, screenlines). None runs a global sensitivity analysis, but a FAccT
   audience will expect one (Saltelli et al. 2019).

---

## 1. Surveillance-disparity studies (the placement literature)

| Paper | Unit & exposure | Undercount | Model | Spatial inference | Crime handling | Mobility |
|---|---|---|---|---|---|---|
| **Dahir, Sheng, Yao, Goel & Hwang 2025**, *Nature Cities* 2(7):662–670, doi:10.1038/s44284-025-00274-2 | BG, 10 US cities incl. NYC; camera count from 100k GSV points/city (2015); log road-km offset | Human-verified positives; recall 0.63 **not corrected** ("slightly underestimated") | Zero-inflated Poisson, city FE; change models control for initial count | **None** found (no spatial SEs, clustering, or Moran's I); only spatially lagged change covariates (SLX-style, Supp. Table 10) | Total reported crime per capita as a control, while stating it is "part of the same social process"; robustness with violent crime + MV theft | None |
| **Sheng, Yao & Goel 2021**, AIES '21, doi:10.1145/3461702.3462525 | Image-level, 16 cities | **Only prior correction**: K̂ = n/(c·r), recall r and GSV coverage c treated as exact | Linear probability model, city FE, zoning, %minority (+ quadratic) | Plain OLS SEs | None | None |
| **Amnesty International 2022**, *Decode Surveillance NYC: Methodology*, AMR 51/5205/2022 (no DOI) | Tract; "effective cameras" = union of 120 m viewsheds ÷ one viewshed area, within tract or 200 m, per 1,000 residents | 3 coders per intersection, median kept; expert audit of 104 intersections gives coder/expert ratio 95% CI [0.23, 1]; **explicitly not corrected** | Poisson GLM (stop-and-frisk, pop offset, by borough); logit on median-split surveillance | Model-based GLM SEs only | — | Protest-route demo only (illustrative) |
| **Calacci, Shen & Pentland 2022**, PACM HCI 6(CSCW2), doi:10.1145/3555125 **[abstract]** | County (national), tract (LA); Ring Neighbors posts per capita | — | **Spatial autoregressive lag model**, state FE; LA "white enclave" indicator | SAR (weights matrix not visible) | Crime rates as covariates | None |
| **Chen, Christensen, John, Owens & Zhuo 2025**, *REStat*, doi:10.1162/rest_a_01370 | BG, 21 cities; arsinh(officer-hours) from smartphone GPS | Validated device counts vs FBI force size (ρ=0.98) | OLS on race relative to city share, ± city FE | **Robust SEs; tract-clustered in SI**; no Conley | **Pre-period homicide** (least reporting-contaminated), framed as "demand"; 311 as robustness (NYC) | Police mobility (the watchers, not the watched) |

Other work found (descriptive or agency-level; none is a placement regression with spatial inference):
- Keener, Finn & Baird 2026, SocArXiv, doi:10.31235/osf.io/5ckgv_v1 — 614 Flock ALPRs (Hampton
  Roads, VA) from an unsealed list; GIS + descriptives.
- Hendrix, Taniguchi, Strom, Barrick & Johnson 2018, *Surveillance & Society* 16(1),
  doi:10.24908/ss.v16i1.6709 **[abstract]** — agency-level CCTV/drone adoption vs police–community
  racial asymmetry.
- Johnson, Johnson, McCurdy & Olajide 2022, *Gov. Info. Quarterly* 39:101753 — facial-recognition
  adoption across 1,136 cities, doubly robust PS models; agency-level.
- Kim & Wo 2026, *Am. J. Crim. Justice*, doi:10.1007/s12103-026-09936-w — DC CCTV → crime, using
  overlapping egocentric neighborhoods (supports treating our overlapping walksheds explicitly).
- Berglund 2025, *Urban Geography* 46(10), doi:10.1080/02723638.2025.2524957 — Project Green
  Light Detroit, qualitative/geographic.
- Journalism/advocacy (descriptive): EFF 2015 Oakland ALPR; WIRED Feb 2024 ShotSpotter sensor
  leak **[abstract]**; Cybernews Sept 2026 Flock/DeFlock demographics **[abstract]**.
- No peer-reviewed regression study of ShotSpotter or ALPR placement vs race was found.

**What we do that none of them do:** activity-space exposure; two-census capture–recapture;
walkshed-based residential exposure (Amnesty's 200 m buffer is nearest); spatial model selection;
Conley SEs + MAUP. **What they do that we should borrow:** Chen's pre-period homicide rung as the
least-endogenous crime control; Dahir's explicit "crime is part of the same process" framing;
Dahir's count model (ZIP/NB with an exposure offset) as a comparability robustness check.

## 2. Activity-space exposure and the NEAP

| Paper | Data & unit | Home included? | Time weighting | Inequality metric | NEAP handling |
|---|---|---|---|---|---|
| **Kwan 2012**, *Annals AAG* 102(5):958–968, doi:10.1080/00045608.2012.687349 | Conceptual (UGCoP) | — | — | — | Motivates activity-space measurement |
| **Kwan 2018**, *IJERPH* 15:1841, doi:10.3390/ijerph15091841 | Conceptual (defines NEAP) | — | — | Ranges (max–min, P5–P95) | Mobility-based exposure "tends toward the mean" — **shrinking spread is expected** |
| **Kim & Kwan 2021**, *Annals AAG* 111(1):121–140, doi:10.1080/24694452.2020.1756208; and *Env. Research* 195:110519, doi:10.1016/j.envres.2020.110519 | NHTS 2017 CA diaries, N=3,790 individuals; Google Maps routing | **Yes** | Full 1,440-min day | Sociodemographic coefficients in spatial regressions, residence vs mobility | Up/downward averaging; "doubly disadvantaged" (high residential exposure + low mobility) don't average |
| **Ma, Li, Kwan & Chai 2020**, *IJERPH*, doi:10.3390/ijerph17041223 | GPS + sensors, 117 Beijing residents | Yes | Full day | Paired t-tests; SD comparison | Logit of who *fails to escape*: low-income, blue-collar |
| **Cai & Kwan 2024**, *ES&T*, doi:10.1021/acs.est.4c02464 | CMAP survey, 27,198 people; green space, air, food, transit, **crime** | **Yes** | 1,440 min | corr(mobility − residence, residence); **NEAR ratio**; Mann-Whitney by quintile | **Null models**: permute exposure to set Moran's I 0→0.8; simulate mobility holding residences fixed. NEAP = regression to the mean under spatial autocorrelation |
| **Dewulf et al. 2016**, *IJHG*, doi:10.1186/s12942-016-0042-z | Mobile-network data, Belgium, NO2 | Yes (4 am location = home) | Hourly, full day | Share of users up/down | Exposure rose for 54.5%, fell for 33.1% |
| **Park & Kwan 2017**, *Health & Place*, doi:10.1016/j.healthplace.2016.10.002 **[abstract]** | Travel survey | Yes | Hourly vs daily | Four-way comparison | Estimates differ significantly |
| **Nyhan et al. 2016**, *ES&T*, doi:10.1021/acs.est.6b02385 **[abstract]** | NYC PM2.5; cell-network counts; area-level | Yes (day/night population) | Time-resolved | Pop-weighted "Active Population Exposure" vs "Home" | — |
| **Chaix et al. 2013**, *Health & Place* 21:46–51, doi:10.1016/j.healthplace.2013.01.003 | Conceptual | — | — | — | **Selective daily mobility bias**: keep constrained anchors (home, work) |
| **Browning et al. 2021**, *ASR*, doi:10.1177/0003122421994219 **[abstract]** | Smartphone GPS, 1,405 youth | — | Waking time: **60% home**, 5.7% neighborhood, 34.3% outside | — | Empirical home weight |
| **Athey, Ferguson, Gentzkow & Schmidt 2021**, *PNAS*, doi:10.1073/pnas.2026160118 | GPS, ~17.7M devices | **Yes** (all pings incl. sleep) | Ping-weighted | Isolation index, same estimator for residential and experienced | **Leave-out estimator** to avoid small-sample bias; experienced 0.46 vs residential 0.61, r=0.86 — regression-to-mean pattern |
| **Moro et al. 2021**, *Nat. Comms.*, doi:10.1038/s41467-021-24899-8 | Phone stays, ~1.1M venues | No (home not a venue) | Time at venues | Individual segregation | **Null**: shuffle visits at city level |
| **Wang, Phillips, Small & Sampson 2018**, *PNAS* 115:7735, doi:10.1073/pnas.1802537115 | Geotagged tweets, ~400k | **No** (home BG excluded) | Share of visited BGs | Group gaps vs commuting-zone availability | **Random-destination baseline** |
| **Nilforoshan et al. 2023**, *Nature*, doi:10.1038/s41586-023-06757-3 | 1.6B co-locations, 9.6M people | Yes | All exposures | corr(own SES, mean SES met) | Naive correlation from noisy per-person means is **biased**; mixed-effects correction (cites Reardon et al. 2018, *Demography*, doi:10.1007/s13524-018-0721-4) |
| **Yabe et al. 2023**, *Nat. Comms.*, doi:10.1038/s41467-023-37913-y **[abstract]** | Encounters, 4 US cities | — | — | Income diversity | Counterfactual exploration behaviour |
| **Brazil 2022**, *PNAS*, doi:10.1073/pnas.2117776119 | SafeGraph, tract | Reported separately | Trip-share weighted | Separate scales (home / bordering / visited) | Not merged into one index |
| **deSouza et al. 2024**, *ES&T* 58:280–290, arXiv:2303.12559 **[DOI unverified]** — closest LODES precedent | LODES + pollution | **Yes**: HW = 0.794·H + 0.206·W (1,801 of 8,760 annual hours) | Annual hours | Group means + **Atkinson index**, H vs HW | H − HW as misclassification |
| **Duran-Sala, Mazzoli, Hendrick & Manoli 2026**, arXiv:2603.24782 — closest structural sibling | Phone OD (23 Spanish cities) + **LODES** tract flows (30 US cities); heat | **No** (trips home excluded) | — | **Group gaps** (low vs high income) — commuting **amplifies** gaps | **Nulls**: label permutation (B=100), gravity and radiation models with group-agnostic flows |

**Standard definition.** Activity-space exposure is a time-weighted full-day average **including
home** (Kim & Kwan, Cai & Kwan, Dewulf, Athey, Nilforoshan, deSouza). Home time is ≥60% of waking
hours (Browning) and ~79% of annual hours (deSouza). Papers that exclude home (Wang, Moro,
Duran-Sala) answer a different question — the character of places visited — and compare group gaps
against a null; none claims the population's exposure "equalizes".

**Implication for our headline.** Gini(R) = 0.334 → Gini(A_mnl) = 0.048 compares one walkshed per
BG against an average over ~287 destination walksheds with no home term, over commuters only. The
literature would read the drop as NEAP, i.e. expected. The defensible claims are the group gaps and
who cannot average.

## 3. Mobile-sensing and exposure simulations

| Paper | Sensor / coverage model | Trajectories | Uncertainty & validation |
|---|---|---|---|
| **O'Keeffe, Anjomshoaa, Strogatz, Santi & Ratti 2019**, *PNAS*, doi:10.1073/pnas.1821667116 | Segment covered if traversed ≥1× in T = 1 day | Empirical taxi trips map-matched to OSM (10 datasets, 9 cities); generative "urban explorer" (shortest path to random destination, preferential return β=1.5) | Ball-in-bin analytic vs empirical coverage curves in every city; Zipf segment popularity; ~5% of vehicles cover half the segments. Proposes a time-dependent Poisson extension (≈ our intensity field) |
| **Anjomshoaa et al. 2018**, *IEEE IoT J.*, doi:10.1109/JIOT.2018.2839058 **[abstract]** | Drive-by sensing on municipal trash trucks (Cambridge, MA) | Real routes | Empirical coverage |
| **Ji, Han & Liu 2023**, *Sust. Cities & Soc.*, doi:10.1016/j.scs.2023.104874 (survey); *TR-C*, doi:10.1016/j.trc.2023.104404 | Coverage as spatio-temporal utility (segment × time window) | **Timetabled bus trips** from schedules | Optimisation, not exposure. Han et al., mixed fleets, arXiv:2311.15237 |
| **Guo & Qian 2024**, *IEEE T-ITS*, doi:10.1109/TITS.2024.3394748 — closest NYC ride-hail analogue | Coverage in six 3-h windows | 2017 Manhattan GPS, >20k ride-hail vehicles; OSRM shortest paths, 15-s interpolation | Inferred trips **validated against TLC counts per 15 min**; sensitivity on path-search parameters |
| **Franchi, Zamfirescu-Pereira, Ju & Pierson 2023**, FAccT, doi:10.1145/3593013.3594020 (self; cite in third person) | Empirical — 24.8M Nexar dashcam images | Not simulated | Reweighted to residential BG distribution; detector calibration by subgroup; bootstrap CIs |
| **Sintonen, Turtiainen, Costin et al.**, OSRM-CCTV, arXiv:2108.09369 **[DOI unverified]** | Circle or sector (radius, angle, direction); **unknown heading = one random draw**; no occlusion | Privacy-preserving routes avoiding FOV nodes | Routes 1.5× (low-performance cameras) to 5.0× (high) longer — the spread is itself a sensitivity result |
| **Turtiainen, Costin & Hämäläinen**, CCTV-Exposure, arXiv:2208.02159 **[DOI unverified]** | Default 10 m "privacy invasion" radius, framed as a **lower bound** (real ranges 20 m to hundreds of m); no occlusion | Real GPX tracks at 0.5 m | Example cases only |
| **Choi & Lee 2015**, *Sensors*, doi:10.3390/s150923341 | Ray-traced 3D, known intrinsics/extrinsics | — | **2D analysis overestimates coverage** |
| **Han et al. 2019**, *CEUS*, doi:10.1016/j.compenvurbsys.2019.101396 | Footprint viewsheds; optimise FOV, yaw, distance | LBS demand weighting | — |
| **Piza**, "Creating Viewsheds to Evaluate the Effect of CCTV Camera Systems" (Northeastern repository, no DOI) | Viewsheds digitised from live footage / aerials, clipped at buildings | — | Propensity-score matching |
| **Gurram, Stuart & Pinjari 2019**, *CEUS*, doi:10.1016/j.compenvurbsys.2019.01.002 | DaySim + MATSim + MOVES + R-LINE (Tampa) | ABM | **Resolution check**: low resolution moved exposures from 8× higher to ~90% lower |
| **Ragettli et al. 2014**, *IJERPH*, doi:10.3390/ijerph110505049 | Three pollution models compared | Survey OD, GIS shortest paths, hourly factors | Validated vs 31 monitors (R² 0.41–0.58) |
| **He et al. 2021**, *Transport Policy*, doi:10.1016/j.tranpol.2020.12.011 — MATSim-NYC | 8M-agent synthetic population | ABM | **NYC validation bar**: INRIX speeds (7.2% freeway / 17.1% arterial error), East River screenlines, station counts (8%) |
| **Zhu & Levinson 2015**, *PLOS ONE*, doi:10.1371/journal.pone.0134322 | — | GPS | Most drivers **do not take the shortest path** |

Sensitivity-analysis method references: Saltelli et al. 2019, *Env. Modelling & Software*,
doi:10.1016/j.envsoft.2019.01.012 (one-at-a-time analyses are widespread and misleading);
ten Broeke et al. 2016, *JASSS*, doi:10.18564/jasss.2857 (method choice for ABMs).

**How our constants compare**

| Our input | What comparable work does |
|---|---|
| Dashcam penetration 0.40 × capture 0.40 | No constant anywhere: observed image density (Franchi 2023) or observed trajectories (O'Keeffe, Guo & Qian). Our group's Nexar density could calibrate the field by zone and hour |
| Dashcam field from Manhattan-only zone pickups (bug) | Guo & Qian and O'Keeffe use full observed trip sets; validate against TLC counts |
| 5 m/s street speed | Observed speeds (OSRM timestamps; INRIX). TLC HVFHV `trip_time`/`trip_miles` give zone × hour speeds |
| One TLC day (2024-06-25) | O'Keeffe pools days and replicates across cities; Guo & Qian stratify into 3-h windows. One day gives no day-to-day variance |
| ACE synthetic headways | GTFS `stop_times` timetabled trips (Ji et al.) |
| Subway cameras 3/station, 2/car | Comparable to Amnesty's range assumptions — defensible only as a bracketed scenario parameter |
| Unknown headings → expectation-over-heading mixture | **More principled** than OSRM-CCTV's single random draw |
| 2D footprint occlusion | Matches Han 2019 and Piza; Choi & Lee: 2D overestimates coverage relative to 3D |
| Camera range | Literature spans 10 m (CCTV-Exposure) to 120–200 m (Amnesty) — likely the dominant uncertainty; must be swept |
| A* shortest paths, 2.5 km walk cutoff | Standard (Ragettli), but real routes deviate (Zhu & Levinson) |
| VOT = 0.5 × wage | Matches USDOT 2016 value-of-time guidance for local personal travel **[not re-fetched]**; validate predicted vs ACS mode shares by origin, as He et al. do |

---

## 4. Recommendations, ranked

### Spatial inference (applies now)
1. **Conley SEs with a bandwidth sweep, reported as a table.** A 10-min walk is ~800 m, so
   neighbouring walksheds overlap up to ~1.6 km apart; bandwidths below that under-cover the
   mechanical correlation. Report 1, 2 (headline), and 5 km at minimum. *Done 2026-09-23 in
   `tools/spatial_econometrics.py`; see §5.*
2. **A_i needs cluster-robust inference, not only Conley.** A_i is correlated through shared
   destinations (Midtown, Downtown Brooklyn) — non-local, not distance-decaying. Cluster by origin
   PUMA (55 in NYC; use a wild cluster bootstrap at that count) or NTA (~195), and run a
   permutation/placebo that reshuffles LODES destinations.
3. **Keep Moran's I and LM diagnostics; present the AIC-selected spatial model as a robustness
   layer**, with LeSage–Pace impacts. Resolve the SDM-vs-SDEM contradiction (AIC now selects SDM;
   footnote and wiki say SDEM) on stated grounds.
4. **Pre-empt spatial-noise critiques**: Conley 1999, *J. Econometrics* 92(1),
   doi:10.1016/S0304-4076(98)00084-0; Müller & Watson 2022, *Econometrica*, doi:10.3982/ECTA19465
   (SCPC); Kelly, "The Standard Errors of Persistence", now published in *J. International
   Economics* 2025, doi:10.1016/j.jinteco.2024.104027 (all three verified on Crossref
   2026-09-23). A spatial-noise placebo or SCPC run as a check.
5. **Propagate capture–recapture uncertainty into the regressions** (parametric bootstrap over
   corrected counts, stacked with spatial SEs). Sheng et al. treated recall as exact — a clean
   contrast.
6. **Count-model robustness**: Poisson/NB on R_i with a walkshed road-length offset, for
   comparability with Dahir.
7. **Crime ladder**: add a pre-period homicide rung (Chen); normalise mediators to rates; present
   the ladder as a descriptive decomposition (Dahir's "same social process" framing), not
   mediation.

### Activity-space measure (decides the headline)
8. **Time-weighted person-day exposure over all residents.** E = w_h·R_home + w_w·W_dest +
   w_c·C_route, weights from ATUS/NHTS or deSouza's 0.794/0.206; non-commuters get R_i. Gini over
   all residents.
9. **Matched aggregation.** Report inequality over jobs-weighted OD pairs (synthetic persons) as
   well as BG means; decompose within vs between BG.
10. **Random-destination null.** Hold residences and job counts fixed; reassign destinations
    uniformly, distance-matched, and by gravity/radiation. Report equalization **beyond** the null.
    Our k=20 random-BG average (Gini 0.078) is already a crude version of this null.
11. **Lead with group gaps**: race/income coefficients (Kim & Kwan), group means + Atkinson
    (deSouza), label-permutation p-values (Duran-Sala), Cai & Kwan's NEAR ratio, and a "who cannot
    average" analysis of non-commuters.
12. **Selective mobility**: keep work as the constrained anchor (Chaix); report the routed commute
    segment as its own scale (Brazil).

### Simulation (before or alongside writing)
13. **Global sensitivity analysis** (Sobol or Morris) over camera range, FOV half-angle, dashcam
    penetration/capture, street speed, subway cameras per station and per car, ACE headway, walk
    cutoff, VOT. Report indices on the **disparity metrics** and where rankings flip.
14. **O'Keeffe-style validation of the mobile field**: Zipf fit of segment popularity; coverage vs
    fleet-size curve against map-matched TLC or Nexar counts. Fix the Manhattan-only input first.
15. **Replace constants with data**: a month or more of TLC data (bootstrap across days, weekday vs
    weekend), TLC zone × hour speeds, GTFS timetables for ACE.
16. **Routing realism**: k-shortest or stochastic route choice as a check; compare modelled
    commute times with ACS travel times; MNL predicted vs ACS mode shares by origin.
17. **Label scenario layers with bounds** (glasses, robots); release code, parameters, and seeds.

---

## 5. Change made 2026-09-23: Conley HAC fix

`tools/spatial_econometrics.py` passed the kernel weights `gwk` to spreg's `OLS` without
`robust='hac'`. spreg 1.9 ignores `gwk` in that case, so every "Conley HAC" SE the script printed
and persisted (`ols_conley`, read by `make_tables.py` for `spatial.tex` and the `HispConley*`
macros) was the classical iid OLS SE.

The fix:
- **Two fits per specification.** One classical fit (with `spat_diag`, for the LM model-selection
  diagnostics, which spreg disables under `robust='hac'`) and one HAC fit per bandwidth.
  Coefficients are identical; only SEs differ.
- **Bandwidths 1, 2, 5 km**, triangular kernel, explicit unit diagonal (spreg's requirement).
  2 km stays primary, so `ols_conley` keeps its meaning for `make_tables.py`.
- **New JSON keys.** `ols_classical` (the iid SE, for contrast) and `ols_conley_by_bw`, and the
  crime/311 ladder rungs are now persisted under `ladder_hac` (previously stdout-only).

Verified: every coefficient, AIC, impact, and MAUP value is unchanged (max abs diff < 1e-6); only
SEs moved. The 2 km SEs match an independent numpy Conley recomputation from the 2026-09-22 audit.
`make_tables.py` regenerated; `--check` green. Only `spatial.tex` (HAC row) and `\HispConleySE` in
`macros.tex` changed.

**Before → after (R_i, N = 5,547, coefficient in cameras/SD)**

| Term | β | Reported "Conley" SE (was iid) | HAC 1 km | HAC 2 km (primary) | HAC 5 km | t at 2 km / 5 km |
|---|---|---|---|---|---|---|
| %Hispanic | +8.98 | 0.57 | 1.47 | **2.01** | 2.30 | 4.5 / 3.9 |
| %Black | +7.01 | 0.54 | 1.19 | **1.68** | 2.18 | 4.2 / 3.2 |
| income | −0.46 | 0.56 | 1.27 | **1.67** | 2.11 | −0.3 / −0.2 |

E_i shows the same pattern: the %Hispanic SE goes from 0.35 (iid) to 1.23 at 2 km and 1.44 at 5 km,
with β = +5.14.

**Crime/311 ladder, %Hispanic on R_i under HAC** (now persisted under `ladder_hac`):

| Rung | β | iid SE | 1 km | 2 km | 5 km | t at 2 km / 5 km |
|---|---|---|---|---|---|---|
| 2: demographics + land use | +8.98 | 0.57 | 1.47 | 2.01 | 2.30 | 4.5 / 3.9 |
| 3: + crime | +4.24 | 0.61 | 1.40 | 1.84 | 2.15 | 2.3 / 2.0 |
| 4: + crime + 311 | +3.79 | 0.60 | 1.37 | 1.79 | 2.03 | 2.1 / 1.9 |

**What changes in the claims.** The placement disparity with demographics and land use (rung 2) is
robust to spatial inference at every bandwidth: t ≥ 3.9 for %Hispanic, ≥ 3.2 for %Black. After the
crime and 311 controls it is **marginal**: t = 2.1 at 2 km and 1.9 at 5 km. "Survives every control"
(wiki `results-placement-bias`, `spatial-econometrics`; wiki quotes "Conley rung 4 +3.88 ±0.60")
must become "attenuates to marginal significance once crime and 311 enter", which also fits the
post-treatment reading of those controls (§4 item 7). The SEs grow monotonically with bandwidth and
have not plateaued by 5 km, so report the sweep, not one number. Items 2 (clustered or permutation
inference for A_i) and 4 (SCPC) are still open.

Not changed (flagged): the `spatial.tex` footnote in `make_tables.py:612–623` says we "rely on the
AIC-selected error specification", but AIC now selects SDM (46092 vs SDEM 46102). The LM tests
favour the error model, so the model-selection rule needs deciding.

---

## 6. Change made 2026-09-23: clustered inference for the activity-space regressions

New `tools/activity_space_inference.py` re-runs every `crime_ladder.py` ladder (3 outcomes × 4
focal variables × 4 rungs). It uses the same sample, population weights, and z-scoring, and asserts
that its coefficients and HC1 SEs match `crime_ladder.json` to 1e-8. For each focal coefficient it
adds:
- CR1 SEs clustered by origin tract (2,242), NTA 2020 (216), and PUMA 2020 (55);
- Conley HAC at 2 and 5 km;
- a wild cluster bootstrap-t (null imposed, Webb 6-point weights, B = 9,999) at NTA and PUMA for
  rungs 1 and 4.

Crosswalks come from the new `geo_clusters` dataset in `fetch_snapshots.py` (Census 2020
tract→PUMA file; NYC DCP NTA 2020 polygons). Output: `data/derived/results/activity_space_inference.json`.
Both new scripts are wired into `refresh_results.sh`.

### Validation: `tools/inference_montecarlo.py`

The Monte Carlo holds the real design fixed: the A_mnl sample, pop weights, z-scored %Hispanic,
real locations, clusters, and LODES shares. It simulates outcomes with true β = 0, 500 sims per
scenario. The table shows size at the 5% level (0.05 is correct).

| Error structure | HC1 | tract | NTA | PUMA | Conley 2 km | Conley 5 km | WCB PUMA |
|---|---|---|---|---|---|---|---|
| iid | 0.056 | 0.056 | 0.058 | 0.054 | 0.056 | 0.070 | 0.050 |
| spatial (1 km cell shocks) | 0.416 | 0.296 | 0.104 | 0.078 | 0.078 | 0.088 | **0.058** |
| shared destination (Σ_k p_ik u_k) | 0.876 | 0.832 | 0.646 | 0.500 | 0.630 | 0.464 | 0.466 |
| mixed | 0.496 | 0.376 | 0.158 | 0.140 | 0.140 | 0.136 | 0.116 |

What the Monte Carlo shows:
- **Local spatial dependence.** PUMA clustering, Conley, and above all the PUMA wild cluster
  bootstrap are close to correct size. Tract clustering is not.
- **Shared-destination dependence.** No estimator comes close: PUMA recovers only 37% of the true
  SD. This is the structure of A_i itself, since A_i is 85% Σ_k p_ik R_k.
- **So the clustered SEs are lower bounds for A_mnl and E_i.** Valid inference there needs the
  design-based random-destination null (recommendation 10).
- **Dropped estimator.** A destination-overlap HAC (kernel = cosine similarity of destination
  share vectors, in the spirit of Adão, Kolesár & Morales 2019) was tried and dropped. It was
  biased low even under iid errors (SE ratio 0.78) and recovered only 36% under destination
  shocks.

### Results (β in cameras per SD; p from the PUMA wild cluster bootstrap)

| Outcome | Focal | Rung 1 β (HC1 SE → PUMA SE) | p (WCB PUMA) | Rung 4 β (HC1 → PUMA) | p (WCB PUMA) |
|---|---|---|---|---|---|
| R_i | %Hispanic | +10.74 (0.66 → 3.67) | **0.006** | +2.84 (0.51 → 2.04) | 0.180 |
| R_i | %Black | −1.57 (0.62 → 3.79) | 0.676 | +1.53 (0.49 → 1.72) | 0.403 |
| R_i | %White | −4.61 (0.71 → 4.80) | 0.360 | −2.85 (0.62 → 2.81) | 0.366 |
| R_i | income | −1.74 (0.84 → 4.39) | 0.769 | −2.55 (0.60 → 1.87) | 0.266 |
| A_mnl | %Hispanic | −0.84 (0.13 → 0.76) | 0.277 | −1.04 (0.13 → 0.56) | 0.083 |
| A_mnl | %Black | −1.22 (0.13 → 1.01) | 0.264 | +0.08 (0.16 → 0.77) | 0.920 |
| A_mnl | %White | +3.35 (0.15 → 0.92) | **0.001** | +3.91 (0.20 → 0.76) | **<0.001** |
| A_mnl | income | +3.08 (0.14 → 0.60) | **0.002** | +2.71 (0.16 → 0.54) | **<0.001** |
| E_i | %Hispanic | +6.00 (0.42 → 2.40) | **0.020** | +0.98 (0.31 → 1.23) | 0.449 |
| E_i | %Black | −1.44 (0.40 → 2.55) | 0.590 | +1.22 (0.29 → 1.04) | 0.255 |

What changes for the paper:
- **HC1 SEs are far too small.** They are 4–7× too small against PUMA for A_mnl and 5–6× for R_i.
  Every HC1-based "significant" statement in the ladder tables needs re-checking.
- **R_i: the placement disparity survives only as the total.** The %Hispanic gradient (rung 1,
  +10.74) survives spatial inference (p = 0.006). No rung-4 coefficient does (%Hispanic p = 0.18).
  This is consistent with §5: robust as a total association, not after crime and 311 controls.
- **A_mnl: the %Hispanic sign flip is not identified.** The coefficient the paper repo's macro
  `\HispAmnl` reports (−0.84, flipped from +0.58) is not distinguishable from zero (p = 0.28), even
  before the shared-destination caveat.
- **A_mnl: the robust gradient runs toward White and higher-income residents** (p ≤ 0.002 at PUMA),
  who have higher activity-space exposure. This is plausibly the Manhattan-destination pull (the
  Manhattan job share explains 56% of A_dest). These p-values are also lower bounds on uncertainty
  under shared-destination dependence. The random-destination null is the test that can confirm
  or dissolve them.
- **E_i: the %Hispanic total survives** (p = 0.02). The rung-4 coefficient does not.

---

## 7. Verification record (2026-09-23) and peer-reviewed basis of every method used

### 7.1 Independent checks of this session's code

| Component | Check | Result |
|---|---|---|
| Conley HAC (`spatial_econometrics.py`) | Coefficients, AIC, impacts, MAUP before vs after fix | Identical to < 1e-6; only SEs changed |
| Conley HAC | 2 km %Hispanic SE vs independent numpy kernel (2026-09-22 audit) | 2.01 vs 2.01 |
| `conley_kernel` (`activity_space_inference.py`) | Symmetry, unit diagonal, self-pair handling | Symmetric; diag = 1; `sparse_distance_matrix` returns both (i,j) and (j,i) |
| `kernel_v` | Same design as spreg `OLS(robust='hac')`, N = 5,547 | 2.0059 vs 2.0059 |
| `cr1` | statsmodels `cov_type='cluster'` (tract) | 0.8546 vs 0.8546 |
| HC1 | statsmodels `HC1` | 0.5870 vs 0.5870 |
| Ladder reproduction | All 48 (β, HC1) pairs vs `crime_ladder.json` | Asserted equal to 1e-8; passes |
| `wild_cluster_p` | `wildboottest` package (WCR, Webb, PUMA), R_i ~ %Hispanic | t = 2.922 both; p = 0.0066–0.0076 (mine) vs 0.0068–0.0072 (package) at B = 19,999 over three seeds; the earlier 0.0051 vs 0.0074 gap was seed noise |
| Negative-variance clamping | Scan of all Conley/CR1 SEs in the JSON | None occurred |
| Monte Carlo size under iid | All estimators | 0.050–0.058 (WCB 0.050) |
| Headline "averaging" numbers | Independent recomputation | A_dest share 0.848; Gini(A_dest) 0.067; Gini(commute) 0.205; Gini(A+R) 0.147; Gini(E_i) 0.201; random-k Gini 0.342 / 0.156 / 0.078 / 0.035 / 0.020 at k = 1 / 5 / 20 / 100 / 287; pair-level destination Gini 0.284; corr(R, commute) −0.404 |
| Citations | Every DOI in this document against the Crossref API | All resolve to the stated title and venue |

Known limits of the checks: the Monte Carlo uses one bivariate regressor (%Hispanic); the
destination scenario draws iid equal-variance shocks per work BG, whereas real R_k is heavy-tailed
and spatially clustered, so the real distortion is plausibly larger, not smaller. The triangular
kernel in two dimensions is not guaranteed positive semi-definite (Conley 1999 uses a product
Bartlett kernel); no negative variance occurred here, and the code clamps at zero and would show a
zero SE if one did.

### 7.2 Logical-soundness review of the claims made this session

- **"HC1 SEs are 4–7× too small."** Correct as stated relative to PUMA-clustered SEs. The Monte
  Carlo shows PUMA is itself about right under local spatial dependence (ratio 0.90) and too small
  under shared-destination dependence, so the factor is a floor.
- **"Clustering cannot fix shared-destination dependence."** Correct and expected: cluster and
  kernel estimators assume dependence dies off across clusters or with distance. Origins across
  the whole city load on the same Midtown destinations, so no partition of origins is independent.
  This is the setting of Adão, Kolesár & Morales 2019 and Borusyak, Hull & Jaravel 2022, whose
  answer is inference at the level of the shocks (here, destinations) or a design-based null, not
  origin clustering. It also matches Abadie, Athey, Imbens & Wooldridge (2020, 2023): when the
  sample is the whole population, the uncertainty is design-based, and the relevant "experiment"
  is the assignment of destinations. This is why the random-destination null is the right next
  step rather than a further SE correction.
- **"The Gini collapse is mostly averaging."** The literature reading is supported (Kwan 2018;
  Kim & Kwan 2021; Cai & Kwan 2024). One refinement: the k-random-BG null (0.078 at k = 20) is a
  *uniform* random-destination null. Real destinations are concentrated in Manhattan, so a
  distance- or gravity-matched null can give a different reference Gini. Report both.
- **"Dashcam field is Manhattan-only."** Verified from `fetch_snapshots.py:480–541` and the pairs
  file (intra-borough median m_dash = 0.00 outside Manhattan). The compounding and inversion
  results follow from it.
- **Model-selection footnote.** Still inconsistent (AIC → SDM; footnote and wiki → SDEM). Not
  changed; needs a decision.
- **Dropped estimator.** The destination-overlap HAC was removed because it failed the iid
  check (ratio 0.78). The failure is the documented downward bias of wide kernels (the residuals
  are orthogonal to X, so a near-constant kernel subtracts variance), not a coding error.

### 7.3 Coverage matrix: each method → peer-reviewed basis (DOIs verified on Crossref)

| Component of the instrument or analysis | Method as implemented | Peer-reviewed basis | Status |
|---|---|---|---|
| Residential exposure R_i | Cameras reachable in a 10-min walk on the OSM pedestrian graph (Dijkstra) | Dijkstra 1959, doi:10.1007/BF01386390; Kwan 2012 UGCoP (why a walkshed, not the polygon), doi:10.1080/00045608.2012.687349; Amnesty 2022 uses a 200 m buffer (grey literature) | **Backed** (the 10-min radius itself is a planning convention; sweep it) |
| Unknown camera heading | Expectation over uniform heading; bullet FOV 70°, p_bullet 444/2,710 from the typed subset | Sector/frustum model as in Sintonen et al. (arXiv, not peer-reviewed); Choi & Lee 2015 doi:10.3390/s150923341; Han et al. 2019 doi:10.1016/j.compenvurbsys.2019.101396 | **Partially backed**: the heading mixture is our own; the sector model is standard. Document as such |
| Building occlusion | 2D footprint ray test | Han et al. 2019; Piza (viewsheds clipped at buildings); Choi & Lee 2015 show 2D overestimates | **Backed**, with the 2D-vs-3D caveat |
| Cross-source dedup | Union-find at 15 m | No specific source; standard record linkage | **Convention**; state the radius sensitivity |
| Undercount correction | Chapman estimator on two censuses, 50 m match | Chapman 1951 (*Univ. Calif. Publ. Stat.*, no DOI); Seber 1982/1986 reviews, doi:10.2307/2531423; Chao 1987 for heterogeneous capture, doi:10.2307/2531532 | **Backed**, but the independence assumption is violated in both directions (§Audit); direction of bias unknown, not "conservative" |
| Commute flows | LODES 2022 JT00 home→work, NYC→NYC | Census LEHD documentation (grey); used the same way by Duran-Sala et al. 2026 (arXiv) and deSouza et al. 2024 (*ES&T*) | **Backed by practice**; use JT01 primary jobs as a check |
| Route choice | A* shortest path on the drive/walk graph | Hart, Nilsson & Raphael 1968, doi:10.1109/TSSC.1968.300136; but Zhu & Levinson 2015 show real routes deviate, doi:10.1371/journal.pone.0134322 | **Backed**, with a realism caveat |
| Transit routing | GTFS subway routing | Delling, Pajor & Werneck 2015 RAPTOR, doi:10.1287/trsc.2014.0534 (cited in the plan) | **Backed** |
| Mode choice | MNL over {walk, drive, transit}, VOT = 0.5 × wage, ASCs calibrated per income quintile | McFadden 1974 (book chapter); Ben-Akiva & Lerman 1985 (book); USDOT 2016 VOT guidance (grey) | **Backed in form**; fit to ACS shares is weak (r 0.40–0.57) and the walk cutoff bug is unresolved |
| Subway camera complement | 3 per station, 2 per car, assumed | None | **Not backed**: scenario parameter; sweep (done: `subway_sweep.json`) |
| Rideshare dashcam field | Poisson intensity ∝ TLC pickup density, penetration 0.40 × capture 0.40 | O'Keeffe et al. 2019 propose exactly a time-dependent Poisson field, doi:10.1073/pnas.1821667116; Guo & Qian 2024 observed-trajectory analogue, doi:10.1109/TITS.2024.3394748 | **Backed in form**; input is Manhattan-only (bug); the two constants have no source |
| ACE bus cameras | Synthetic headway curve along GTFS routes, 20 m capture | Ji, Han & Liu 2023 use timetabled trips, doi:10.1016/j.trc.2023.104404 | **Weakly backed**; replace with `stop_times` |
| Inequality measures | Population-weighted Gini, top-decile share, P90/P10 | Standard; the R-vs-A comparison is contested by Kwan 2018 doi:10.3390/ijerph15091841; Kim & Kwan 2021 doi:10.1080/24694452.2020.1756208; Cai & Kwan 2024 doi:10.1021/acs.est.4c02464 | **Not backed as a finding**; needs the time-weighted measure and null model |
| Random-destination null (planned) | Reassign destinations uniformly / distance-matched / gravity | Wang et al. 2018 baseline, doi:10.1073/pnas.1802537115; Moro et al. 2021 shuffle, doi:10.1038/s41467-021-24899-8; Duran-Sala et al. 2026 gravity/radiation nulls; Lenormand, Bassolas & Ramasco 2016 for trip-distribution models, doi:10.1016/j.jtrangeo.2015.12.008; Simini et al. 2012 radiation model, doi:10.1038/nature10856 | **Backed** (to build) |
| Segregation-index small-sample bias | (Not yet applied) | Cortese, Falk & Cohen 1976, doi:10.2307/2094840; Reardon & Bischoff 2011, doi:10.1086/657114; Reardon et al. 2018, doi:10.1007/s13524-018-0721-4; Athey et al. 2021, doi:10.1073/pnas.2026160118 | **To apply** if pair-level indices are reported |
| Need-neutral counterfactual | Cameras reallocated ∝ population or ∝ population + jobs | No direct precedent; closest are Dahir's offsets (road-km) | **Not backed**; present as descriptive; bootstrap |
| Crime/311 ladder | Pop-weighted WLS, controls added in stages | Dahir et al. 2025 include crime while calling it endogenous, doi:10.1038/s44284-025-00274-2; Chen et al. 2025 pre-period homicide, doi:10.1162/rest_a_01370 | **Backed as a descriptive ladder**, not as mediation |
| Spatial autocorrelation diagnostics | Moran's I, Anselin LM-lag/LM-error and robust forms | Anselin, Bera, Florax & Yoon 1996, doi:10.1016/0166-0462(95)02111-6 | **Backed** |
| Spatial models and impacts | ML SAR/SEM/SDM/SDEM; LeSage–Pace direct/indirect/total | LeSage & Pace 2009, doi:10.1201/9781420064254; Kelejian & Prucha 2010, doi:10.1016/j.jeconom.2009.10.025 | **Backed**; selection rule undecided |
| Spatial HAC SEs | Triangular kernel, 1/2/5 km | Conley 1999, doi:10.1016/S0304-4076(98)00084-0; Kelejian & Prucha 2007, doi:10.1016/j.jeconom.2006.09.005 (spreg implements this) | **Backed**; now correctly implemented |
| Cluster-robust SEs | CR1 by tract/NTA/PUMA | Cameron & Miller 2015, doi:10.3368/jhr.50.2.317; Bester, Conley & Hansen 2011 on spatial clusters, doi:10.1016/j.jeconom.2011.01.007 | **Backed** |
| Wild cluster bootstrap | WCR-t, Webb weights, B = 9,999 | Cameron, Gelbach & Miller 2008, doi:10.1162/rest.90.3.414; MacKinnon & Webb 2018, doi:10.1111/ectj.12107; Webb 2023, doi:10.1111/caje.12661; Roodman et al. 2019 (boottest), doi:10.1177/1536867X19830877 | **Backed**; verified against `wildboottest` |
| Shared-destination dependence | Diagnosed by Monte Carlo; no valid SE estimator | Adão, Kolesár & Morales 2019, doi:10.1093/qje/qjz025; Borusyak, Hull & Jaravel 2022, doi:10.1093/restud/rdab030; Abadie, Athey, Imbens & Wooldridge 2020, doi:10.3982/ECTA12675 and 2023, doi:10.1093/qje/qjac038 | **Backed diagnosis**; the remedy is design-based (null model) |
| MAUP | Re-estimation at tract and 1 km grid | Fotheringham & Wong 1991, doi:10.1068/a231025 | **Backed** |
| Spatial-noise placebo (planned) | — | Kelly 2025, doi:10.1016/j.jinteco.2024.104027; Müller & Watson 2022, doi:10.3982/ECTA19465 | **To do** |
| Global sensitivity analysis (planned) | — | Saltelli et al. 2019, doi:10.1016/j.envsoft.2019.01.012; ten Broeke et al. 2016, doi:10.18564/jasss.2857 | **To do** |

Coverage gaps that a reviewer can name, in priority order: (1) the R-vs-A Gini contrast has no
peer-reviewed basis as a finding and a peer-reviewed literature that predicts it mechanically;
(2) the subway camera constants and dashcam penetration/capture constants have no source;
(3) the capture–recapture independence assumption; (4) the need-neutral counterfactual has no
precedent and no uncertainty; (5) heading mixture and 15 m dedup are our conventions and should
be swept; (6) ACE uses synthetic headways although GTFS timetables exist.

---

## 8. Change made 2026-09-23: citywide dashcam field

**Bug.** `tools/fetch_snapshots.py` built `tlc/zone_trips.csv` from pickups in Manhattan taxi
zones only. `bake-dashcam-field` then gave every non-Manhattan zone intensity 0, so the dashcam
class contributed nothing to any trip that stayed outside Manhattan. The M2 compounding
correlation (−0.38) and the M3 incidence inversion (87% "outside the home borough") were both
consequences of this.

**Fix.**
- Fetch: count trip **ends** (pickups + dropoffs) for all zones 2–263 (EWR excluded; 264/265 are
  TLC "unknown"), from the same June 2024 HVFHV monthly file. 39.2 M trip-ends = 19.6 M trips.
  Share by borough: Manhattan 37%, Brooklyn 28%, Queens 21%, Bronx 12%, Staten Island 1.5%.
- Bake (`crates/data-pipeline/src/dashcam.rs`): the intensity anchor is now explicitly the
  **median Manhattan zone** (329,183 trip-ends/km² over the month), which `DashcamConfig::
  vehicles_per_min_peak = 12` was implicitly calibrated to. Manhattan intensities are therefore
  nearly unchanged (zone-level corr 0.995 with the old field, median ratio 0.997, peak 2.7× at
  Times Sq), while the other boroughs enter on the same absolute scale: Bronx mean 0.18,
  Brooklyn 0.24 (Williamsburg 0.94, Downtown Brooklyn 0.74), Queens 0.11, Staten Island 0.01.
  The bake now logs the reference density and zones-with-trips per borough so a truncated input
  is loud. Provenance `as_of` corrected from "2024-12" to the 2024-06 file.
- Built with `RUSTUP_TOOLCHAIN=1.95-x86_64-unknown-linux-gnu` (the shared `stable` toolchain is
  half-updated and has no `rustc`; left untouched).
- `tools/cluster_dashcam_rebake.sh` (new, sbatch) re-runs bg-exposure, od-exposure (drive, modal,
  mnl + pair emission) and exposure-table, keeps the previous CSVs under
  `data/derived/exposure/prev_<date>/`, and checks that the fixed-camera columns are
  byte-identical. Then `tools/refresh_results.sh`.

**Remaining limitation (state in the paper).** Trip-end density is a presence proxy for where
vehicles start and finish, not where they drive in between; through-traffic corridors (e.g.
Queens expressways, bridge approaches) are under-weighted relative to a routed, O'Keeffe-style
segment field. The citywide routed `taxi_day` bake exists and is the natural upgrade
(recommendation 14). Penetration 0.40 and capture 0.40 remain unsourced constants.

**Re-bake results (SLURM job 990636, 1 h 47 min at 16 threads; `refresh_results.sh` clean).**
Fixed-camera and ACE columns byte-identical in all four exposure files and the pairs file; only
`m_dash_*` changed. Of 16 result JSONs, only `compounding.json` and `incidence_inversion.json`
moved.

| | Before (Manhattan-only) | After (citywide) |
|---|---|---|
| M_dash_res, pop-wtd mean: Bronx / Brooklyn / Manhattan / Queens / SI | 0.00 / 0.00 / 1.21 / 0.00 / 0.00 | 0.30 / 0.33 / 1.16 / 0.17 / 0.02 |
| M_dash_act (per commute), same order | — | 23.9 / 15.2 / 11.8 / 17.3 / 39.3 |
| M2: pop-wtd corr(R_i, M_act) | −0.378 | **−0.495** |
| M2: corr(R_i, M_dash_act) / corr(R_i, M_ace_act) | −0.333 / −0.461 | −0.485 / −0.461 |
| corr(R_i, M_dash_res) | (undefined: zeros) | **+0.466** |
| M3: dash captures with work borough ≠ home borough | 0.867 | 0.850 |
| M3: median m_dash, same-borough vs cross-borough pair | 0.00 vs 15.8 | 3.1 vs 20.0 |

**Reading.** The citywide field did not change the sign of M2; it made it more negative, and the
borough table shows why. `M_act` is expected encounters per commute *traversal*, so it scales with
route length: Staten Island has the lowest R_i (18) and the highest M_act (39). The compounding
statistic as defined measures "long commuters from low-R_i places accumulate more mobile
encounters", not whether mobile surveillance falls on the same people as fixed surveillance. The
rate-based term says the opposite: corr(R_i, M_dash_res) = +0.47, i.e. at the residence, dashcam
intensity compounds fixed exposure. **Decision (2026-09-23):** the mobile
term stays a **dose per commute trip** — "expected encounters on one trip to work" — because the
measures are meant to be readable by the public and to evoke the lived experience, and a per-km
rate is detached from the feeling of being watched on the way to work. The paper must then say
plainly that the dose scales with commute length, and report the residence-rate correlation
(+0.47) beside the dose correlation (−0.50) so the two sentences are not confused. M3 has the same length dependence (cross-borough pairs 6× the
same-borough median) on top of the work-borough attribution problem, so 85% is still not the
statistic the headline describes.

### 8.1 M3 fixed: captures attributed to where they happen (2026-09-23, later the same day)

`batch od-exposure-mnl` now books every 50 m route sample to the borough of its nearest block-group
centroid and emits per-borough columns in the pairs file (`m_dash_{bx,bk,mn,qn,si,unk}`,
`m_ace_{…}`; `MobileLeg` in `crates/batch/src/main.rs`). `incidence_inversion.py` uses them; the
work-borough view is kept in the JSON as `legacy_work_boro_view` for comparison only. Re-bake job
13385 (rustc 1.98.1 after the toolchain repair): every existing output file is byte-identical to
the previous bake, all columns; the per-borough columns sum to `m_dash` within 4-decimal rounding;
0.00% of captures fell in the "unknown" slot.

| Statistic (dashcam, dose per commute, LODES-weighted) | Work-borough view (old) | Where-captured view (new) |
|---|---|---|
| Share of captures outside the bearer's home borough | 85.0% | **59.5%** (ACE: 44.6%) |
| Manhattan streets' captures borne by non-Manhattan residents | 88% | **77%** (Brooklyn 25, Bronx 25, Queens 17, SI 10) |
| Share of residents' commute captures that happen in their own borough: Mn / Bk / Qn / Bx / SI | — | 84 / 49 / 38 / **21** / **9** % |

Who bears what each borough's streets generate (rows sum to 100%): Manhattan ← Bk 25, Bx 25, Mn 23,
Qn 17, SI 10; Brooklyn ← Bk 50, SI 22, Qn 20; Queens ← Qn 70, Bk 13, Bx 10; Bronx ← Bx 74, Qn 11,
Mn 8; Staten Island ← SI 79, Bk 11.

**Reading.** The inversion is real but smaller than the flawed statistic claimed: three in five
dashcam captures on a commute happen in a borough the person does not live in, and Manhattan's
rideshare cameras record mostly outer-borough residents. The sharpest form is the last row:
place-based measurement books a Bronx resident's mobile exposure to the Bronx, but four fifths of
it happens elsewhere. The dose-per-trip definition still means long commuters weigh more (a
deliberate choice, §8 decision); a per-person or per-borough-resident normalisation would be the
robustness check.

---

## Audit context (2026-09-22)

Issues found in our own pipeline that motivate the recommendations above:
- Dashcam field built from Manhattan-only zone pickups (`tools/fetch_snapshots.py:537–541`), so
  M2 compounding (corr −0.38) and M3 incidence inversion (87%) are artifacts.
- A_mnl is 85% `A_dest`, the jobs-weighted mean of destination walkshed counts over ~287
  destinations. The home walkshed is excluded, and the commute leg is netted against the home and
  destination walksheds.
- Spatial model selection flipped to SDM on re-bake (ΔAIC ≈ 10), and the hard-coded footnote in
  `make_tables.py` was not updated.
- Capture–recapture bias direction is unknown: mid-block Dahir cameras can never match Amnesty's
  intersections, which pushes recall down, while the shared-GSV dependence pushes it up. The
  "non-differential" test is underpowered (201 matched).
- `undercount_spatial.py` inflates all of `R_cctv` (the capture_recapture bug fixed on 2026-07-14).
- R_i uses `cameras_raw`, while A and the counterfactual use `cameras_corrected`.
