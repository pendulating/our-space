# Citywide taxi trips: how fast must they arrive to saturate the cap?

> Answers the TODO: *"In city-wide route, estimate the number of trips threshold
> that would be required to render the MAX number of real-time trips in the city,
> per minute. We did a similar analysis for the manhattan-only route; write
> findings to a doc."*

This is the citywide (`?city=nyc`) companion to the Manhattan concurrency
analysis in [`docs/TAXI_GENERATION.md` §5](TAXI_GENERATION.md). It estimates the
**trip arrival rate (trips per minute)** needed to keep the on-screen real-time
vehicle pool full, and whether the real five-borough HVFHV day actually reaches
it. The headline result: **it does — by a wide margin, for most of the day.**

## TL;DR

- The on-screen vehicle pool is **hard-capped at `MAX_VEHICLES = 4,000`**
  concurrent taxis (`crates/app-interactive/src/agents.rs:30`) — the *same* cap in
  both the Manhattan and citywide builds.
- A trip occupies a slot for its whole duration, so by Little's Law the arrival
  rate that keeps 4,000 slots filled is **`4,000 ÷ mean-trip-minutes`**. With the
  measured Manhattan mean effective dwell of **≈ 16.9 min**, that is
  **≈ 236 trips/minute (≈ 4 trips/second)** to saturate the cap.
- **This saturation threshold is the same for both extents** (it depends only on
  the cap and the per-trip dwell). What differs is whether the city's real
  arrival rate *reaches* it.
- **Manhattan never reaches it.** Its busiest minute drives ≈ 226 arrivals/min →
  peak concurrency **3,823 < 4,000**. The cap is "effectively uncapped for this
  day" (`agents.rs:28-29`).
- **Citywide blows past it.** The baked citywide day is **391,380 trips**
  (`docs/TAXI_GENERATION.md:322`). Its *day-average* arrival rate is already
  **≈ 272 trips/min > 236** — i.e. the cap is saturated even at an *average*
  minute. Estimated peak concurrency is **≈ 9,100 (~2.3× the cap)**, so at the
  evening peak only **≈ 44 % of concurrently-active trips are rendered** and the
  rest are dropped by the subsample.

---

## 1. The model: Little's Law against a fixed pool

The runtime taxi replay (`replay_agents`, `agents.rs:691-780`) is a fixed pool of
pre-spawned entities. A trip is **active** for the half-open window
`[pu_min, pu_min + dur_min)` (`agents.rs:714`); while active it holds one of the
`MAX_VEHICLES` slots, and `animate_agents` positions it along its route. New
trips are admitted from a **forward cursor over the start-sorted trip list**; when
all slots are full the admit loop hits `break; // pool full`
(`agents.rs:740-762`) and any further trips that start during the full period are
**skipped permanently** (the cursor has already advanced past them). So:

> **The number of taxis on screen at minute *t* equals the number of active trips
> at *t*, clamped to 4,000.** Above the clamp, the visible set is the
> *earliest-starting* active trips and the remainder are dropped.

This is a classic queue. **Little's Law** relates the average number in system
*L*, the arrival rate *λ*, and the mean time in system *W*:

```
L  =  λ · W
```

Here *L* is concurrent on-screen trips, *λ* is trips started per minute, and *W*
is the mean trip duration (the slot-occupancy time). To **saturate the cap** we
need *L = 4,000*:

```
λ_sat  =  MAX_VEHICLES / W  =  4,000 / W      (trips per minute)
```

### Pinning down *W* from measured Manhattan numbers

We do not have to assume *W* — we can back it out from the two measured Manhattan
quantities (`crates/sim-core/examples/taxi_peak.rs`, reported in
`docs/TAXI_GENERATION.md:32-34, 278-284`):

```
baked trips        = 164,184
mean concurrent    = 1,929   (averaged over the 1,440-minute day)
```

Little's Law over the whole day (∫L dt = Σ trip durations) gives the mean
effective per-trip dwell directly:

```
W  =  mean_concurrent × 1,440 / trips
   =  1,929 × 1,440 / 164,184
   ≈  16.9 minutes
```

(This is the *effective* on-screen dwell as the `taxi_peak` diff-array measures
it — minute-rounded occupancy. The true mean trip duration is ≈ 1 min shorter,
~16 min; using the rounded figure keeps the arithmetic self-consistent with the
measured concurrency, which is what we are comparing against.)

### The saturation threshold

```
λ_sat  =  4,000 / 16.9  ≈  236 trips/minute  ≈  3.9 trips/second
```

**Sensitivity.** If citywide trips run longer than Manhattan's (outer-borough
HVFHV trips cover more distance — see §4), *W* rises and `λ_sat` *falls*, making
the cap **easier** to saturate, not harder:

| mean dwell *W* | `λ_sat = 4000/W` |
|---|---|
| 15 min | 267 /min |
| **16.9 min (measured MN)** | **236 /min** |
| 19 min | 211 /min |
| 21 min | 190 /min |

So **~190–270 trips/min** is the saturation band; ~236/min is the point estimate.

---

## 2. Self-check against the Manhattan baseline

The threshold should reproduce the known Manhattan result, and it does:

```
Manhattan peak concurrency  = 3,823 at 18:20                 (measured)
implied peak arrival rate   = 3,823 / 16.9  ≈ 226 trips/min  (= L_peak / W)
                            = 226 / 236  ≈ 96 % of λ_sat
```

The busiest Manhattan minute runs at **96 % of the saturation rate**, so
concurrency tops out at **3,823 — just 4.4 % under the 4,000 cap.** That is
exactly why `agents.rs:28-29` calls 4,000 "effectively uncapped for this day": the
cap was *deliberately* sized to clear Manhattan's true peak with a sliver of
headroom. The Manhattan day-average arrival is only **164,184 / 1,440 ≈ 114
trips/min** — less than half of `λ_sat` — so off-peak the pool is far from full
(mean 1,929, median 2,379).

---

## 3. The citywide picture: the cap is saturated, not approached

The citywide build swaps in `taxi_day_nyc.ostaxiday`
(`TAXI_DAY_PATH_NYC`, `main.rs:132`, selected at `main.rs:1084`; `dashcam_on =
true` citywide at `main.rs:1053`) and keeps the **same** `MAX_VEHICLES = 4,000`
cap. The asset is **8.0 MB vs Manhattan's 3.7 MB** and holds (per
`docs/TAXI_GENERATION.md:322`):

```
real all-borough day (2024-06-25):  547,263 trips, all 258/259 PU/DO zones
baked (top-5000 O-D routes, 0 no-path):  391,380 trips
```

### Day-average arrival already exceeds the threshold

```
citywide day-average arrival  =  391,380 / 1,440  ≈  272 trips/minute
                              =  272 / 236  ≈  1.15 × λ_sat
```

This is the striking result: **the citywide arrival rate averaged over the entire
24-hour day (272/min) is already above the ~236/min needed to keep all 4,000 slots
full.** Manhattan only reaches that rate at its single busiest minute; citywide
sits above it *on average*. The cap is therefore not merely *touched* at peak — it
is saturated across the bulk of the active day.

### Estimated citywide concurrency (peak and mean)

We do not have a measured citywide concurrency curve (running
`taxi_peak` on `taxi_day_nyc.ostaxiday` would produce one — see §6). Estimating by
**scaling the Manhattan profile by the baked-trip ratio**, under the assumption of
a similar diurnal shape and a similar per-trip dwell:

```
trip ratio        =  391,380 / 164,184  ≈  2.384×

est. peak concurrency  ≈  3,823 × 2.384  ≈  9,100   (Manhattan peak was 18:20)
est. mean concurrency  ≈  1,929 × 2.384  ≈  4,600
est. peak arrival rate ≈  9,100 / 16.9   ≈  539 trips/min  ≈  2.3 × λ_sat
```

Both estimated concurrencies **exceed the 4,000 cap** — the *mean* (≈4,600)
already does, which means for **more than half the day's minutes the pool is
full.** Scaling Manhattan's hourly peak profile (`06=1253 07=2636 08=3535
09=3413 … 17=3743 18=3823 19=3165`, `TAXI_GENERATION.md:283`) by 2.384× crosses
4,000 between **06:00 and 07:00** (Manhattan-equivalent 4,000/2.384 ≈ 1,678,
passed between the 06:00 and 07:00 buckets) and stays above it through the late
evening — an estimated **~16–18 hours of the day at the cap.**

> **These citywide concurrency figures are estimates** (scaled from Manhattan),
> not measured. The *threshold* arithmetic (§1) and the *day-average* arrival
> (272/min, computed directly from the real baked count) are exact; the peak and
> hourly figures inherit Manhattan's temporal shape, which the outer boroughs only
> approximately share (they are more commute-peaked and less late-night-heavy than
> Manhattan). The *direction* of the conclusion is robust to this: a longer
> citywide dwell (§4) would only raise concurrency and lower `λ_sat`.

### What fraction of real trips is actually shown?

Two independent drops sit between the real citywide day and the pixels:

1. **Bake-time drop (top-5000 routes).** Citywide has far more than 5,000 possible
   directed O-D pairs (≈258×259 ≈ 67k), so the `MAX_OD_ROUTES = 5000` head keeps
   only the most frequent pairs: **391,380 / 547,263 ≈ 71.5 %** of real trips are
   even eligible to render; the long-tail 28.5 % are dropped at bake
   (`taxi_day.rs:90-94`). *(Manhattan has at most 69×68 = 4,692 directed pairs —
   under 5,000 — so essentially **none** of its trips are dropped at bake. This
   drop is unique to citywide.)*

2. **Runtime cap drop (the 4,000 subsample).** Of the eligible trips active at a
   given minute, only 4,000 fit on screen. At the estimated evening peak:

   ```
   shown at peak  =  4,000 / 9,100  ≈  44 %   (56 % dropped by the subsample)
   ```

Compounding both, at the evening peak the on-screen system renders roughly

```
0.715 (eligible)  ×  0.44 (under cap)  ≈  31 %
```

of all real concurrent citywide rideshare trips — **fewer than one in three.**
Off-peak hours (and the morning/late-night shoulders below 4,000 concurrent) show
all eligible trips, so the *daily* shown fraction is higher than the peak figure;
but during the long midday-to-late-evening saturated window the rendered fraction
stays well under half.

> Contrast Manhattan, where ~100 % of trips are eligible (no bake drop) and the
> cap is never hit, so **every** baked trip is shown.

---

## 4. Caveats & assumptions (be honest)

- **`W` is borrowed from Manhattan.** The 16.9-min mean dwell is measured on the
  Manhattan asset. Citywide outer-borough HVFHV trips tend to be **longer** in
  both distance and time, so the true citywide `W` is plausibly larger — which
  *strengthens* the saturation conclusion (lower `λ_sat`, higher concurrency).
  Flagged as the main assumption; not yet measured.
- **Peak/mean concurrency are scaled, not measured.** They assume the citywide
  diurnal shape matches Manhattan's. Outer boroughs are more commute-peaked, so
  the real morning/evening peaks may be sharper and the overnight trough deeper
  than the uniform 2.384× scaling implies. The *day-average* arrival (272/min) and
  the *threshold* (236/min) are exact; everything labelled "est." is not.
- **Different real day.** The citywide asset is **Tuesday 2024-06-25**; Manhattan
  is the 2024-06 month labelled **2026-04-21**. Both are weekdays, so the
  comparison is apples-to-apples in shape, but they are not the same calendar day.
- **The 547,263 / 391,380 split** is quoted from `docs/TAXI_GENERATION.md:322`
  (the bake log), not re-measured here.
- **Decorative agents.** As in Manhattan, these taxis are a visualization; none of
  the citable exposure numbers depend on how many render. The cap's only cost is
  *visual completeness*, not analytic accuracy.

---

## 5. So what?

The Manhattan cap (4,000) was tuned to sit *just above* that extent's true peak
(3,823) — a near-perfect, lossless fit. **Carried unchanged into the five-borough
build, the same cap is now the binding constraint for most of the day.** The
citywide day averages ~272 trips/min of arrivals against a ~236/min saturation
rate, so the pool runs full from the morning commute into the late evening, and at
the peak only ~31 % of real concurrent trips reach the screen.

If lossless citywide rendering were a goal, the cap would need to clear the
estimated ~9,100 peak (≈ 2.4× today). But per
[`docs/SCALING.md` §2](SCALING.md), agents are deliberately *cap-bounded* so five
boroughs render the same agent budget spread thinner — the intended fix is
**viewport-aware spawning** (Scaling Phase 3: admit trips near the camera) so the
4,000-slot budget is spent on what's on screen, rather than raising the cap and
the per-frame cost. This doc quantifies *why* that matters citywide: unlike
Manhattan, the citywide build is genuinely over the cap.

---

## 6. Where this lives in code / how to verify

| Concern | File:line |
|---|---|
| On-screen cap `MAX_VEHICLES = 4000` (both builds) | `crates/app-interactive/src/agents.rs:30` |
| Runtime replay + earliest-by-start subsample | `crates/app-interactive/src/agents.rs:691-780` |
| Active-window test `[pu_min, pu_min+dur_min)` | `crates/app-interactive/src/agents.rs:714` |
| `break; // pool full` (the drop) | `crates/app-interactive/src/agents.rs:751-753` |
| Manhattan taxi asset path | `crates/app-interactive/src/main.rs:129` (`TAXI_DAY_PATH`) |
| **Citywide** taxi asset path | `crates/app-interactive/src/main.rs:132` (`TAXI_DAY_PATH_NYC`) |
| Citywide asset selection | `crates/app-interactive/src/main.rs:1084` |
| Citywide `dashcam_on = true` | `crates/app-interactive/src/main.rs:1053` |
| Bake-time top-5000 route drop | `crates/data-pipeline/src/taxi_day.rs:23` (`MAX_OD_ROUTES`), `:90-94` |
| `TaxiTrip { pu_min, route_idx, dur_min }` | `crates/sim-core/src/assets.rs:725-732` |
| Peak-concurrency / dwell report tool | `crates/sim-core/examples/taxi_peak.rs` |
| Manhattan measured figures | `docs/TAXI_GENERATION.md:32-34, 278-284` |
| Citywide trip counts (547,263 / 391,380) | `docs/TAXI_GENERATION.md:322` |
| Five-borough scaling context | `docs/SCALING.md` (§2 caps, §4 Phase 3) |

**To replace the §3 estimates with measured numbers**, run the existing tool on
the citywide asset (a short read-only example, no app build):

```
cargo run -p sim-core --example taxi_peak -- \
  crates/app-interactive/assets/processed/taxi_day_nyc.ostaxiday
```

It prints baked trips, peak/mean/median/p95 concurrency, the minutes over each
threshold, effective speeds, and the 24-hour hourly-peak profile — exactly the
Manhattan figures this analysis scaled from.
