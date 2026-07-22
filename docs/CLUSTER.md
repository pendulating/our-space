# Running the re-bake on a compute cluster

The surveillance-exposure re-bake — the Rust `batch` exposure runs behind the FAccT
'27 paper — currently runs on a developer laptop and takes ~2.5 hours wall-clock,
almost all of it in two commands. This runbook moves it to the two-node Linux cluster.
It is a runbook, not a design doc: exact commands, exact paths, what to copy, what each
knob does.

The cluster:

| Node | CPUs | GPUs | RAM |
|---|---|---|---|
| **big-cpu** | 196 | — | 1 TB |
| **gpu** | 64 | 8× NVIDIA RTX A6000 | 1 TB |

**Use the 196-CPU node.** The bake is CPU-only (see the GPU note), embarrassingly
parallel, and single-digit-GB in memory, so core count is the only axis that helps —
1 TB of RAM is not a constraint on either node.

---

## What runs where

The heavy work is two `batch` subcommands that rayon-parallelize a per-home-block-group
computation over ~6,587 NYC block groups (route the top-K work destinations by commute
flow, flood a 10-minute walkshed at each end, count cross-source-deduped cameras along
the path). The shared state — sensor set, both graphs, the occlusion index — is built
once and read immutably by every worker, so scaling is near-linear in cores.

| Step | Crate | Cost (~10-core laptop) | Produces |
|---|---|---|---|
| `batch od-exposure-mnl` | batch | dominant (see below) | `A_i_mnl_bg_nyc.csv` (income-heterogeneous mode choice) |
| `batch od-exposure-modal` | batch | dominant (see below) | `A_i_modal_bg_nyc.csv` (observed-ACS-share modes) |
| `batch bg-exposure` | batch | ~1–2 min | `R_i_bg_nyc.csv` (residential walkshed) |
| `batch counterfactual` | batch | ~1–2 min | `counterfactual_bg_nyc.csv` |
| `batch covariates` | batch | ~1–2 min | `covariates_bg_nyc.csv` |
| `batch occlusion-audit` | batch | ~1 min × 5-point sweep | `occl_audit_walk_x{1,2,4,8,16}.csv` |
| `data-pipeline bake-*` | data-pipeline | seconds–minutes each | the `.osgraph` / sensor / `.ossub` assets the above consume |

The two `od-exposure-*` runs dominate: **~149 min combined on a ~10-core laptop** at the
canonical `top_k=900`. Because they are embarrassingly parallel over read-only shared
state, expect near-linear scaling — on the 196-CPU node they should finish in **~10–15
min combined** (~20× the laptop's cores, minus scheduling overhead). Everything else is
minutes.

`tools/cluster_rebake.sh` runs the self-contained heavy path: `bake-subway` →
`od-exposure-mnl` → `od-exposure-modal`. The other `batch` steps (`bg-exposure`,
`counterfactual`, `covariates`, `occlusion-audit`) are in the full checklist at the
bottom; add them to the launcher if you want the whole set in one job.

---

## GPU note — read before reaching for the A6000 node

**The bake uses no GPU.** `batch` and `data-pipeline` are CPU-only Rust; their
parallelism is [rayon](https://docs.rs/rayon) work-stealing over CPU cores. There is no
CUDA, no wgpu, no compute-shader path in either crate — grep their `Cargo.toml` and the
dependency list is `rayon` + `rstar` + `anyhow` (batch), and `geo` + `geojson` + `csv` +
`serde` (data-pipeline). The only GPU code in the workspace is Bevy's renderer in
`app-interactive`, which the bake never builds or runs.

So the 8× RTX A6000 node buys **nothing** for this workload — the GPUs sit idle. Prefer
the 196-CPU node. Do not add a GPU path; there is nothing here to accelerate.

---

## Build

Rust toolchain via rustup (the workspace pins `rust-version = 1.88`):

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
```

Then, from the **workspace root**, build only the two headless crates:

```sh
cargo build --release -p batch -p data-pipeline
```

**Always use `-p`.** A bare `cargo build --release` builds the whole workspace, which
pulls in `app-interactive` and therefore the entire **Bevy 0.18** stack — winit, wgpu,
`bevy_render`, and the `x11`/`wayland` backends — a large compile that needs system
GUI/X11 dev libraries a headless node may not have. `batch` and `data-pipeline` depend on
neither Bevy nor those libraries: `sim-core`'s Bevy ECS layer is an opt-in feature
(`default = []` in `crates/sim-core/Cargo.toml`) and neither binary crate enables it, so
`-p batch -p data-pipeline` is a clean, GUI-free build. (Verified against the workspace
`Cargo.toml` + `crates/{batch,data-pipeline,sim-core}/Cargo.toml`.)

---

## Working directory: always the workspace root

`batch` hardcodes repo-relative asset paths — the `const GRAPH_PATH` / `CAMERAS_NYC` /
`ALPR_NYC` / `DOT_NYC` / `ENFORCE_NYC` / `FOOTPRINTS` constants at the top of
`crates/batch/src/main.rs` all begin `crates/app-interactive/assets/processed/…` and are
resolved relative to the process's current directory. **Every command must run from the
workspace root** (the directory with the top-level `Cargo.toml`). `cluster_rebake.sh`
enforces this with a `cd` guard; if you run commands by hand, `cd` to the repo root first.

---

## Data to copy to the cluster

Code comes from git (`git clone` / `git pull` on the cluster). The **inputs and baked
assets are gitignored** — `/data/snapshots/` and `/crates/app-interactive/assets/processed/`
are both in `.gitignore`, so they never travel with the repo. You rsync them.

Inputs the heavy path reads (verified from `tools/bake_citywide.sh`,
`tools/refresh_results.sh`, and the `batch` source):

Baked assets, under `crates/app-interactive/assets/processed/`:
- `graph_nyc.osgraph`, `graph_nyc_walk.osgraph` — drive + pedestrian networks
- `cameras_fixed_nyc.oscctv`, `alpr.osalpr`, `dot_cameras_nyc.osdot`, `enforcement.oscam` — the four fixed-sensor layers
- `footprints.osbldg` + `footprints_{bronx,brooklyn,queens,statenisland}.osbldg` — the occlusion fabric

Snapshot inputs, under `data/snapshots/`:
- `census/bg_centroids_nyc.csv`, `census/acs_nyc.csv`
- `lodes/bg_od_nyc.csv`
- `gtfs/subway/` — the GTFS tables (`bake-subway` input) plus `stations_subway.csv` (the od-exposure stations argument)
- `gtfs/siferry/` — the NYC DOT Staten Island Ferry GTFS (`bake-subway`'s optional ferry feed; it defaults to the subway dir's sibling `siferry`, so this **must** be in the rsync'd tree for the real-ferry bake — without it `bake-subway` falls back, loudly, to the old parameterized 15-min/25-min pseudo-route)

Simplest is to push both whole trees (`-R` recreates the relative paths under the
checkout):

```sh
# from the repo root on the laptop; CLUSTER = your ssh host, ~/our-space = the cluster checkout
rsync -avzR data/snapshots crates/app-interactive/assets/processed CLUSTER:~/our-space/
```

Outputs to pull back land in `data/derived/` (`exposure/*.csv`, `subway_nyc.ossub`, and
`logs/`):

```sh
# from the repo root on the laptop
rsync -avz CLUSTER:~/our-space/data/derived/ data/derived/
```

> **Discrepancy worth knowing.** `batch` loads `alpr.osalpr` and `enforcement.oscam`, but
> `tools/bake_citywide.sh` does **not** bake either — it bakes CCTV, DOT, graphs,
> footprints, subway and the rest, but the ALPR and enforcement layers come from separate
> `data-pipeline` commands (`bake-alpr`, `bake-enforcement`; see the checklist). If you
> rsync the `assets/processed/` tree as above, both arrive as pre-baked files and the
> cluster never rebuilds them — only a *from-scratch* rebake on the cluster needs those two
> extra commands. Likewise the per-borough `footprints_*.osbldg` are only (re)baked when
> `FOOTPRINTS_GEOJSON` is set (an ~856 MB download); rsync the baked `.osbldg` files rather
> than rebuild them on the cluster.

---

## Threads and environment knobs

**rayon uses every core by default.** Cap it with `RAYON_NUM_THREADS`:

```sh
RAYON_NUM_THREADS=190 ./tools/cluster_rebake.sh   # leave a few cores for the OS
```

The launcher passes `RAYON_NUM_THREADS` straight through (child processes inherit the
environment). Unset means all cores.

Runtime knobs read by `batch` (grepped from `std::env::var("OURSPACE…")` in
`crates/batch/src/main.rs`). **Leave all unset for the canonical bake:**

| Env var | Default | Effect |
|---|---|---|
| `OURSPACE_OCCLUSION` | on | Set `0` to disable building line-of-sight (free-space FOV). `occlusion-audit` needs it **on**; the free-space arm of the audit sets `0`. |
| `OURSPACE_CENSUS_RECALL` | `1.0` | Emit observed (uncorrected) camera counts. Range `(0,1]`; e.g. `0.501` emits recall-corrected. The bake also emits `cameras_unconfirmed`, so any recall is reconstructable downstream without a re-run. |
| `OURSPACE_RANGE_SCALE` | `1.0` | Multiplies every camera's range (occlusion range-sensitivity sweep). |
| `OURSPACE_SUBWAY_CAMS_STATION` | `3.0` | Cameras counted per subway station on a transit path. |
| `OURSPACE_SUBWAY_CAMS_TRAIN` | `2.0` | Cameras counted per train/ferry boarded. |
| `OURSPACE_SUBWAY_KM_PER_TRANSFER` | `12.0` | Distance-per-transfer estimate (crow-flies fallback only). |
| `OURSPACE_SUBWAY_MAX_TRANSFERS` | `3.0` | Transfer cap. |
| `OURSPACE_SUBWAY_SCALE` | `1.0` | Multiplies the whole subway-camera complement (sensitivity sweeps). |
| `OURSPACE_SUBWAY_CIRCUITY` | `1.3` | Straight-line → network line-haul factor (`≥1`; crow-flies fallback only, i.e. when no `.ossub` matrix is passed). |

The `OURSPACE_SUBWAY_*` knobs affect only the transit leg inside `od-exposure-*`. Both
models emit a `commute_subway` column, so subway-camera assumptions are swept downstream
(`tools/sweep_subway_cameras.py`) without re-running the bake.

---

## The od-exposure command lines, exactly as run today

Positional order verified against the `USAGE` strings in `crates/batch/src/main.rs`:

```
od-exposure-mnl|modal <drive.osgraph> <walk.osgraph> <centroids> <od> <acs> <stations> <out> [walk_min] [top_k] [subway.ossub]
```

Both models run at `walk_min=10`, `top_k=900`, and the trailing subway-router asset. The
subway asset is positional arg 11 — after `walk_min` and `top_k` — so both of those must
be given explicitly to reach it.

> `top_k=900` (full LODES job-flow coverage) is the canonical value as of 2026-07-15; it
> is **not** the `USAGE` default of `100`. The old top-100 truncated ~40% of commute flow,
> and that truncation correlated with income (+0.50) — a differential bias since eliminated
> by raising `top_k`. This is the single most important argument to get right.

First build the subway-router matrix (a `data-pipeline` step, seconds):

```sh
target/release/data-pipeline bake-subway data/snapshots/gtfs/subway data/derived/subway_nyc.ossub
```

> `bake-subway`'s full signature is `bake-subway <gtfs_dir> <out.ossub> [service_id=Weekday]
> [win_start_h=7] [win_end_h=10] [ferry_gtfs_dir=<gtfs_dir>/../siferry]`. The 6th positional arg is the
> **Staten Island Ferry GTFS**; omitted here, it defaults to the `gtfs_dir`'s sibling
> `data/snapshots/gtfs/siferry` (fetched by `tools/fetch_gtfs.py`), so the command above picks up the real
> ferry feed automatically **provided `gtfs/siferry/` was rsync'd** (see "Data to copy to the cluster").
> Pass an explicit path as the 6th arg to override. If the feed is absent the bake still succeeds, but
> falls back — with a loud warning — to the old parameterized 15-min/25-min ferry pseudo-route.

Then the two heavy runs:

```sh
target/release/batch od-exposure-mnl \
  crates/app-interactive/assets/processed/graph_nyc.osgraph \
  crates/app-interactive/assets/processed/graph_nyc_walk.osgraph \
  data/snapshots/census/bg_centroids_nyc.csv \
  data/snapshots/lodes/bg_od_nyc.csv \
  data/snapshots/census/acs_nyc.csv \
  data/snapshots/gtfs/subway/stations_subway.csv \
  data/derived/exposure/A_i_mnl_bg_nyc.csv \
  10 900 \
  data/derived/subway_nyc.ossub

target/release/batch od-exposure-modal \
  crates/app-interactive/assets/processed/graph_nyc.osgraph \
  crates/app-interactive/assets/processed/graph_nyc_walk.osgraph \
  data/snapshots/census/bg_centroids_nyc.csv \
  data/snapshots/lodes/bg_od_nyc.csv \
  data/snapshots/census/acs_nyc.csv \
  data/snapshots/gtfs/subway/stations_subway.csv \
  data/derived/exposure/A_i_modal_bg_nyc.csv \
  10 900 \
  data/derived/subway_nyc.ossub
```

`tools/cluster_rebake.sh` runs exactly these three commands, in this order, logging each
to `data/derived/logs/<step>.log`.

---

## The Python analysis half — runs on the laptop, not the cluster

The cluster does **not** need Python. `tools/refresh_results.sh` is the cheap downstream
half: it joins the exposure CSVs (`batch exposure-table`) and runs the Python analyses
that turn them into the paper's JSON + LaTeX tables. It explicitly does **not** re-run the
slow `batch` exposure steps — it reads their CSV outputs — and completes in a few minutes.
So the normal flow is: **bake on the cluster → rsync `data/derived/` back → run
`tools/refresh_results.sh` on the laptop.**

If you do want to run it on the cluster, it needs `uv` + Python ≥3.12 and the deps pinned
in the repo-root `pyproject.toml` / `uv.lock` (`uv run` auto-syncs them): `numpy`,
`pandas`, `scipy`, `statsmodels`, `pyarrow`, `geopandas`, `esda`, `libpysal`, `spreg`
(the spatial-econometrics stack), plus `altair` / `marimo` (notebooks). Install uv with
`curl -LsSf https://astral.sh/uv/install.sh | sh`. Note `refresh_results.sh` invokes the
scripts as `uv run python …`, so `uv` must be on `PATH`.

---

## Full re-bake, start to finish

Assumes the raw `data/snapshots/**` are already fetched — several need API keys (e.g.
`CENSUS_API_KEY` for `acs_nyc.csv`; see `tools/fetch_*.py`). Do that on the laptop, once.

**On the laptop (where the snapshots live):**

1. Bake the static/citywide asset set (seconds–minutes):
   ```sh
   tools/bake_citywide.sh
   ```
   Set `FOOTPRINTS_GEOJSON=<nyc_footprints_all.geojson>` first only if the per-borough
   `footprints_*.osbldg` need rebuilding; otherwise that step is skipped and the existing
   baked files are kept.
2. Bake the two sensor layers `bake_citywide.sh` does **not** (the discrepancy above),
   from the repo root:
   ```sh
   cargo run --release -p data-pipeline -- \
     bake-alpr data/snapshots/deflock/alpr.json crates/app-interactive/assets/processed/alpr.osalpr
   cargo run --release -p data-pipeline -- \
     bake-enforcement data/snapshots/enforcement/enforcement_signs.csv crates/app-interactive/assets/processed/enforcement.oscam
   ```
3. Push code (git) + data (rsync) to the cluster — see "Data to copy to the cluster".

**On the 196-CPU node, from the workspace root:**

4. Build:
   ```sh
   cargo build --release -p batch -p data-pipeline
   ```
5. Run the heavy path (bake-subway + both od-exposure models; logs in `data/derived/logs/`):
   ```sh
   RAYON_NUM_THREADS=190 ./tools/cluster_rebake.sh
   ```
6. The rest of the exposure set — minutes each; add to the launcher or run by hand. All
   walksheds use the **walk** graph:
   ```sh
   B=target/release/batch
   P=crates/app-interactive/assets/processed
   SNAP=data/snapshots
   E=data/derived/exposure
   CENT=$SNAP/census/bg_centroids_nyc.csv
   WALK=$P/graph_nyc_walk.osgraph

   "$B" bg-exposure    "$WALK" "$CENT" "$E/R_i_bg_nyc.csv" 10
   "$B" counterfactual "$WALK" "$CENT" "$SNAP/lodes/bg_od_nyc.csv" "$E/counterfactual_bg_nyc.csv" 10
   # covariates: the 311 file is the NINTH arg, AFTER walk_min (10). Omitting it is silent
   # and breaks the rung-4 crime+311 ladder three scripts downstream.
   "$B" covariates     "$WALK" "$CENT" "$SNAP/lodes/bg_od_nyc.csv" \
       "$SNAP/gtfs/subway/stations_subway.csv" "$SNAP/crime/nypd_points.csv" \
       "$E/covariates_bg_nyc.csv" 10 "$SNAP/crime/nyc311_disorder_points.csv"
   # occlusion range-sensitivity sweep (occlusion is on by default):
   for k in 1 2 4 8 16; do
     OURSPACE_RANGE_SCALE=$k "$B" occlusion-audit "$WALK" "$CENT" "$E/occl_audit_walk_x$k.csv" 10
   done
   ```
7. rsync `data/derived/` back to the laptop.

**On the laptop:**

8. Regenerate every JSON + LaTeX table:
   ```sh
   tools/refresh_results.sh
   ```
