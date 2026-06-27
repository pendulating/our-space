# Camera deduplication across layer sources

**Problem.** A single physical camera is often reported by more than one of our fixed
layers. The clearest case: many **NYC DOT** traffic cameras are *also* crowdsourced onto
**DeFlock** (our ALPR layer), and the Amnesty/Dahir **CCTV census** overlaps both. If the
exposure model counts raw layer rows, that one device is counted two or three times — a
real accuracy bug in the "~N cameras watch your area" headline.

**Goal.** Count one *physical* camera once, no matter how many layers attest to it, while
keeping each layer's attestation visible (the per-source map markers and the
click-through modals still show "DOT says…", "DeFlock says…").

---

## The algorithm — proximity clustering across sources

`sim_core::group_sensors(&mut sensors, radius_m)` (`crates/sim-core/src/scenario.rs`)
clusters the combined fixed-sensor set into physical-camera **groups**:

1. **Combine first.** The app concatenates all fixed layers into one `SensorInstance`
   vector — CCTV census → DeFlock ALPRs → DOT traffic cams → enforcement cams
   (`crates/app-interactive/src/main.rs`, around the `group_sensors` call) — then groups.
2. **Union-find with a grid bucket.** Each sensor starts in its own set. A uniform grid
   (cell = `radius_m`) restricts comparisons to neighbours, so it's ~O(n) not O(n²) over
   ~30 k cameras.
3. **Merge only ACROSS sources.** Two sensors are unioned iff they are **different
   `SourceKind`s** *and* within `radius_m`. Two *same-source* rows at one intersection are
   left distinct — each source is assumed already internally de-duplicated, and two real
   CCTV cameras can share a pole. This is the crucial rule: it collapses the
   DOT⇄DeFlock⇄CCTV overlap without erasing genuinely separate cameras.
4. **Compact + confirm.** Each group gets a small integer `group` id, and a `confirmed`
   flag = *true* iff any member is a **surveyed** (non-recall-corrected) source
   (DOT / ALPR / enforcement). Every member of a group carries the group's `confirmed`
   value, so any one captured member reveals the whole group's status.

`radius_m = FIXED_GROUP_RADIUS_M = 15 m` (`main.rs`). On the real census this merges
~1,175 of ~31,042 rows into shared nodes (≈29,867 distinct cameras) — logged at startup
as `grouped N fixed sensors → M distinct camera nodes`.

---

## How the counts use the groups

The grouping is only useful if every place we count cameras keys on `group`, not on the
raw per-layer row id. Both exposure paths now do:

| Path | Where | De-dup key |
|---|---|---|
| **A walk A→B** | `ExposureTally.fixed_groups` (`exposure.rs`), filled by `record_fixed_capture(.., group, confirmed, ..)`; headline via `grouped_fixed_devices()` | `group` |
| **My area** (walkshed) | `walkshed_exposure` (`scenario.rs`) — `seen: HashMap<group → confirmed>` | `group` |

The My-area path previously keyed its `seen` set on the raw **`s.id`**, so a DOT+DeFlock
pair in your walkshed counted as **two**. It now keys on `s.group`, matching the A→B path
— the two modes agree on what "one camera" means.

**Recall correction is group-aware.** A *confirmed* group (some survey mapped it) counts
at face value `1.0`; a *CCTV-census-only* group keeps the recall inflation `recall_factor`
(it stands in for the cameras the street-view census missed). Both paths compute
`Σ over groups (confirmed ? 1.0 : recall_factor)` — see
`ExposureTally::grouped_fixed_devices` and the `corrected` sum in `walkshed_exposure`.

---

## Surfacing the merge in the UI

The grouping is reflected in *both* the map iconography and the click-through modal, so a
merged camera reads as **one** device end to end:

- **One marker per physical camera.** A group attested by several layers used to stack a
  wordmark per source ("CCTV" + "ENF" on the same pole). The marker loop now emits a single
  primary marker per `group`: priority **ALPR → CCTV → DOT → enforcement**, which matches
  `handle_click`'s ALPR-before-CCTV hit-test, so the icon you see is the one whose popup
  opens (DOT/enforcement only win when no clickable layer shares the group). One FOV cone
  per camera follows the same rule. On the Manhattan census this is **4,590 markers** (one
  per node) instead of 5,447 — 857 stacked duplicates removed. *Distinct* nearby cameras
  (same-source, never merged) still draw separately — that's two cameras, not one.
- **The click modal names the other sources.** When a clicked ALPR/CCTV pin belongs to a
  multi-source group, its popup adds a "CROSS-SOURCE CONFIRMED — also mapped here by {other
  sources}. … merged, so it's counted once in the headline." note. Mechanically: after
  `group_sensors`, `build_world` reads each sensor's `group` back by position (a 0.5 m grid
  keyed on the apex, which equals the pin's `pos`), collects the *other* `SourceKind`s
  sharing that group, and stores their labels on `AlprPin.also_sources` /
  `CctvPin.also_sources`. The shared `ui::cross_source_note` renders them; it's a no-op for
  a single-source pin. On the Manhattan census this lights up ~779 CCTV + ~20 ALPR pins.

---

## What is intentionally *not* de-duplicated

- **The per-source breakdown is per-source.** The "devices" row for each `SourceKind`
  reports that layer's attestations (`fixed_seen` keyed on `(source, id)`); only the
  **headline** collapses groups. So "DOT: 4, ALPR: 3" can sum to more than the headline.
- **Same-source neighbours stay distinct** (see rule 3).

---

## Limitations & tuning

- **15 m radius.** DOT and DeFlock coordinates for the same camera usually agree to within
  a few metres (both are near the pole/intersection), so 15 m catches them with margin. Too
  large would merge genuinely separate adjacent cameras; too small would miss noisier
  duplicates. It is a single tunable constant (`FIXED_GROUP_RADIUS_M`).
- **Geometry only.** Matching is purely positional — it does not yet compare heading, FOV,
  or manufacturer. Adding an attribute agreement test (e.g. require similar bearing for an
  ALPR↔DOT merge) would tighten precision if false merges ever appear; the union-find hook
  is the single `if kinds[i] != kinds[j] && dist² ≤ r²` predicate.
- **Trust in intra-source dedup.** We assume each source arrives already de-duplicated
  within itself; we only reconcile *across* sources.

---

## Where this lives (file:line map)

- `crates/sim-core/src/scenario.rs` — `group_sensors` (the clustering algorithm) and
  `walkshed_exposure` (My-area count, now group-keyed) + tests
  `group_sensors_merges_across_sources_only`,
  `walkshed_dedups_colocated_cross_source_camera`,
  `walkshed_recall_inflates_only_unconfirmed_groups`.
- `crates/sim-core/src/exposure.rs` — `ExposureTally.fixed_groups`,
  `record_fixed_capture`, `grouped_fixed_devices`, `headline_device_count` (A→B count).
- `crates/app-interactive/src/main.rs` — `FIXED_GROUP_RADIUS_M`, the `group_sensors` call
  + the `grouped N fixed sensors → M distinct camera nodes` log line.
