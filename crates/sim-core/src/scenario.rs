//! High-level orchestration: turn baked assets into a placed sensor set and run
//! a route end-to-end into an exposure summary. Used by the app, the batch host,
//! and the headless `route_demo` example.

use std::collections::HashMap;

use crate::assets::{DashcamFieldLayer, FixedSensorLayer, RobotabilityField, TeslaField};
use crate::exposure::{ConfidenceTier, ExposureTally, SourceKind};
use crate::geometry::FrustumWedge;
use crate::occlusion::OccluderIndex;
use crate::graph::{Route, RouteError, StreetGraph, Walkshed};
use crate::math::Vec2;
use crate::mobile::{MobileScenario, RealDayRates};
use crate::simulation::{simulate_full, SensorInstance, SimParams};
use crate::spatial::{AceGrid, SensorIndex};

/// Default model assumptions for a fixed camera (the source data has only point
/// locations). User-tunable in the app.
#[derive(Debug, Clone, Copy)]
pub struct FixedCameraDefaults {
    pub full_fov_deg: f64,
    pub range_m: f64,
    pub frame_rate: f64,
}

impl Default for FixedCameraDefaults {
    fn default() -> Self {
        FixedCameraDefaults {
            full_fov_deg: 70.0,
            range_m: 15.0,
            frame_rate: 15.0,
        }
    }
}

impl FixedCameraDefaults {
    /// Model assumptions for NYC DOT traffic cameras: PTZ units mounted high at
    /// intersections, covering the roadway from any bearing. Wider reach than a
    /// storefront CCTV, but treated as a *monitoring* class — a live public feed
    /// refreshing roughly once a second, not a high-frame-rate recorder — so its
    /// per-pass frame contribution stays modest. The feed publishes no bearing,
    /// so these are placed omnidirectional (`heading_deg = None`).
    pub fn dot_monitoring() -> Self {
        FixedCameraDefaults {
            full_fov_deg: 360.0,
            range_m: 30.0,
            frame_rate: 1.0,
        }
    }
}

/// Convert a baked fixed-sensor layer into capture-testable instances. The
/// vector index becomes the device id for distinct-device counting.
pub fn sensors_from_layer(layer: &FixedSensorLayer, d: FixedCameraDefaults) -> Vec<SensorInstance> {
    layer
        .sensors
        .iter()
        .enumerate()
        .map(|(i, s)| SensorInstance {
            wedge: FrustumWedge::from_degrees(
                Vec2::new(s.x, s.y),
                s.heading_deg,
                d.full_fov_deg,
                d.range_m,
            ),
            frame_rate: d.frame_rate,
            id: i as u64,
            kind: s.kind,
            // Default: each sensor its own group, confirmed iff it's a surveyed
            // (non-recall-corrected) kind. `group_sensors` overwrites both once the
            // layers are combined and clustered.
            group: i as u32,
            confirmed: !s.kind.recall_corrected(),
            // Filled in by the caller once the occlusion index exists (batch::load_fixed_sensors).
            host_poly: None,
        })
        .collect()
}

/// Cluster fixed sensors into physical-camera groups by proximity (≤ `radius_m`),
/// **across sources**, so a camera attested by several layers (e.g. the CCTV census
/// + a DOT survey + an enforcement sign) is one node in the headline. Assigns each
/// sensor a compact `group` id and `confirmed` = whether the group has any surveyed
/// (non-CCTV-census) attestation. Returns the number of distinct groups.
pub fn group_sensors(sensors: &mut [SensorInstance], radius_m: f64) -> usize {
    let n = sensors.len();
    if n == 0 {
        return 0;
    }
    let apex: Vec<Vec2> = sensors.iter().map(|s| s.wedge.apex).collect();
    let surveyed: Vec<bool> = sensors.iter().map(|s| !s.kind.recall_corrected()).collect();
    let kinds: Vec<SourceKind> = sensors.iter().map(|s| s.kind).collect();

    // Union-find with a grid bucket (cell = radius) so only nearby pairs are tested.
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    let cell = radius_m.max(1e-6);
    let r2 = radius_m * radius_m;
    let key = |p: Vec2| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64);
    let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> = std::collections::HashMap::new();
    for (i, p) in apex.iter().enumerate() {
        grid.entry(key(*p)).or_default().push(i);
    }
    for i in 0..n {
        let (cx, cy) = key(apex[i]);
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(bucket) = grid.get(&(cx + dx, cy + dy)) {
                    for &j in bucket {
                        if j <= i {
                            continue;
                        }
                        // Only merge ACROSS sources: a camera the CCTV census and a
                        // DOT/ALPR/enforcement survey both record is one node, but two
                        // distinct same-source detections at one intersection stay two
                        // (each source is already internally de-duplicated).
                        if kinds[i] != kinds[j] {
                            let (ddx, ddy) = (apex[i].x - apex[j].x, apex[i].y - apex[j].y);
                            if ddx * ddx + ddy * ddy <= r2 {
                                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                                if a != b {
                                    parent[a] = b;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Compact group ids; confirmed = any surveyed member.
    let mut group_of: std::collections::HashMap<usize, u32> = std::collections::HashMap::new();
    let mut confirmed: Vec<bool> = Vec::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        let g = *group_of.entry(root).or_insert_with(|| {
            confirmed.push(false);
            (confirmed.len() - 1) as u32
        });
        if surveyed[i] {
            confirmed[g as usize] = true;
        }
    }
    for i in 0..n {
        let root = find(&mut parent, i);
        let g = group_of[&root];
        sensors[i].group = g;
        sensors[i].confirmed = confirmed[g as usize];
    }
    group_of.len()
}

/// Maximum spacing between exposure sample points along street geometry, in meters.
///
/// The capture predicate is evaluated at discrete points, so this stride bounds what a
/// query can miss: with samples every `s` meters, a camera at perpendicular offset `d`
/// from the centerline is guaranteed detected iff its along-street detection half-window
/// `sqrt(range² − d²)` is at least `s/2`. At the modeled 15 m CCTV range a 10 m stride
/// detects every camera within ~14 m of the street — effectively complete coverage.
///
/// This replaced first/middle/last-vertex sampling (3 points per edge), under which only
/// ~35% of citywide street length sat within 15 m of any sample (67% of CSCL edges are
/// straight 2-vertex lines, so mid-block cameras on them were structurally invisible),
/// while route legs were densified and saw them — an R_i-vs-A_i asymmetry, not noise.
pub const EXPOSURE_SAMPLE_STRIDE_M: f64 = 10.0;

/// Arc-length sample points along an edge polyline at ≤`stride` spacing, covering the
/// first `frac` of the edge's length measured from the `from_start` end (`frac = 1.0`
/// covers the whole edge, making direction irrelevant). Both ends of the covered span are
/// always emitted, so degenerate polylines still yield their endpoint.
///
/// This is THE sampling rule for every exposure query over street geometry — walksheds
/// (full and boundary-partial edges), the occlusion audit, and the batch route legs all
/// call it, so "which points can a camera see" cannot quietly diverge between the place
/// and trajectory measures again.
pub fn sample_polyline(poly: &[[f64; 2]], stride: f64, from_start: bool, frac: f64) -> Vec<Vec2> {
    let mut out = Vec::new();
    sample_polyline_into(poly, stride, from_start, frac, &mut out);
    out
}

/// Like [`sample_polyline`] but fills a caller-owned buffer (clear-and-reuse),
/// avoiding a fresh allocation per edge in hot loops.
pub fn sample_polyline_into(poly: &[[f64; 2]], stride: f64, from_start: bool, frac: f64, out: &mut Vec<Vec2>) {
    out.clear();
    if poly.is_empty() {
        return;
    }
    let entry = if from_start { poly[0] } else { poly[poly.len() - 1] };
    if poly.len() == 1 || frac <= 0.0 {
        out.push(Vec2::new(entry[0], entry[1]));
        return;
    }
    let pts: Vec<Vec2> = if from_start {
        poly.iter().map(|p| Vec2::new(p[0], p[1])).collect()
    } else {
        poly.iter().rev().map(|p| Vec2::new(p[0], p[1])).collect()
    };
    let mut cum = Vec::with_capacity(pts.len());
    cum.push(0.0);
    for w in pts.windows(2) {
        cum.push(cum.last().unwrap() + w[0].distance(w[1]));
    }
    let total = *cum.last().unwrap();
    if total <= f64::EPSILON {
        out.push(pts[0]);
        return;
    }
    let span = total * frac.clamp(0.0, 1.0);
    let n = ((span / stride).ceil() as usize).max(1);
    out.reserve(n + 1);
    let mut seg = 0usize;
    for k in 0..=n {
        let target = span * (k as f64 / n as f64);
        while seg + 2 < cum.len() && cum[seg + 1] < target {
            seg += 1;
        }
        let seg_len = cum[seg + 1] - cum[seg];
        let t = if seg_len > f64::EPSILON { (target - cum[seg]) / seg_len } else { 0.0 };
        let (a, b) = (pts[seg], pts[seg + 1]);
        out.push(Vec2::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t));
    }
}

/// Expectation model for the street-view CCTV census's unknown camera geometry.
///
/// The census (Amnesty + Dahir) records **no heading** for any camera, and types only
/// 15.5% of them (among those: domes outnumber bullets 5:1 — 2,266 vs 444). A dome sees
/// 360°; a bullet with unknown, uniformly-random heading Θ sees a point-set S iff Θ lands
/// in the union of FOV windows centred on the bearings to S (OCCLUSION_PLAN §10b). So a
/// census camera's expected capture over S is the mixture
///
/// ```text
/// w(S) = p_dome + p_bullet · |⋃_{p∈S} [bearing(p) ± θ/2]| / 2π
/// ```
///
/// At a single point w ≈ p_dome + p_bullet·θ/2π (≈0.87 with the defaults); over a
/// walkshed the street points near a camera subtend a wide arc and w → 1 — the paper's
/// point-vs-path structure a third time. Applies ONLY to `SourceKind::FixedCctv`:
/// surveyed kinds (DOT/ALPR/enforcement) have known semantics (ALPR wedges carry real
/// headings; a captured wedge is certain). Env knobs: `OURSPACE_FOV_MODEL=disc` restores
/// the omnidirectional-disc behaviour (the A/B arm), `OURSPACE_FOV_P_BULLET`,
/// `OURSPACE_FOV_BULLET_DEG`.
#[derive(Debug, Clone, Copy)]
pub struct FovModel {
    /// P(bullet | census camera): typed-subset share, 444/(2266+444).
    pub p_bullet: f64,
    /// Bullet horizontal FOV, radians (default 70°).
    pub bullet_fov_rad: f64,
    /// `false` ⇒ every census camera is a 360° disc (the pre-2026-07-16 model).
    pub enabled: bool,
}

impl Default for FovModel {
    fn default() -> Self {
        FovModel { p_bullet: 444.0 / 2710.0, bullet_fov_rad: 70f64.to_radians(), enabled: true }
    }
}

impl FovModel {
    pub fn from_env() -> Self {
        let envf = |k: &str, d: f64| {
            std::env::var(k)
                .ok()
                .and_then(|s| s.trim().parse::<f64>().ok())
                .filter(|v| v.is_finite() && *v >= 0.0)
                .unwrap_or(d)
        };
        let d = FovModel::default();
        FovModel {
            p_bullet: envf("OURSPACE_FOV_P_BULLET", d.p_bullet).min(1.0),
            bullet_fov_rad: envf("OURSPACE_FOV_BULLET_DEG", 70.0).to_radians().min(std::f64::consts::TAU),
            enabled: std::env::var("OURSPACE_FOV_MODEL").as_deref() != Ok("disc"),
        }
    }
    pub fn describe(&self) -> String {
        if self.enabled {
            format!(
                "arc-union expectation (p_bullet {:.3}, bullet FOV {:.0}°; point-level w {:.3})",
                self.p_bullet,
                self.bullet_fov_rad.to_degrees(),
                self.weight_for(&[0.0])
            )
        } else {
            "360° discs (OURSPACE_FOV_MODEL=disc)".into()
        }
    }
    /// Expected-capture weight for a census camera whose visible sample points subtend
    /// `bearings` (radians, any origin). 1.0 when the model is disabled or no bearing.
    pub fn weight_for(&self, bearings: &[f64]) -> f64 {
        if !self.enabled || bearings.is_empty() {
            return 1.0;
        }
        (1.0 - self.p_bullet) + self.p_bullet * arc_union_fraction(bearings, self.bullet_fov_rad)
    }
}

/// Fraction of the circle covered by the union of windows `bearing ± fov/2` — the
/// probability a uniformly-random heading sees at least one of the bearings.
pub fn arc_union_fraction(bearings: &[f64], fov_rad: f64) -> f64 {
    use std::f64::consts::TAU;
    if bearings.is_empty() || fov_rad <= 0.0 {
        return 0.0;
    }
    if fov_rad >= TAU {
        return 1.0;
    }
    let half = fov_rad / 2.0;
    // Wrap-safe: split any arc crossing 0 into two linear intervals in [0, TAU].
    let mut ivs: Vec<(f64, f64)> = Vec::with_capacity(bearings.len() * 2);
    for &b in bearings {
        let lo = (b - half).rem_euclid(TAU);
        let hi = lo + fov_rad;
        if hi <= TAU {
            ivs.push((lo, hi));
        } else {
            ivs.push((lo, TAU));
            ivs.push((0.0, hi - TAU));
        }
    }
    ivs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut covered = 0.0;
    let (mut cur_lo, mut cur_hi) = ivs[0];
    for &(lo, hi) in &ivs[1..] {
        if lo <= cur_hi {
            cur_hi = cur_hi.max(hi);
        } else {
            covered += cur_hi - cur_lo;
            (cur_lo, cur_hi) = (lo, hi);
        }
    }
    covered += cur_hi - cur_lo;
    (covered / TAU).clamp(0.0, 1.0)
}

/// Result of a one-point walkshed exposure query.
#[derive(Debug, Clone)]
pub struct WalkshedSummary {
    pub max_minutes: f64,
    pub reachable_edges: usize,
    /// Distinct fixed cameras whose coverage touches the walkshed (as detected).
    pub cameras_raw: u32,
    /// Of `cameras_raw`, the groups attested ONLY by the street-view CCTV census —
    /// i.e. the sub-population the census recall applies to, and therefore the only
    /// part of the count that a recall correction inflates. Emitted separately because
    /// the correction is **linear**:
    ///
    /// ```text
    /// cameras(r) = cameras_unconfirmed / r + (cameras_raw - cameras_unconfirmed)
    /// ```
    ///
    /// so any recall `r` — including every draw of a bootstrap — is reconstructable
    /// from a single bake, with no re-run. (Same trick as `commute_subway`.)
    ///
    /// Under the arc-union [`FovModel`] this is a **weighted** sum (census groups carry
    /// their heading-expectation w ≤ 1), hence `f64` — the linear reconstruction above
    /// holds unchanged with weighted sums on both terms.
    pub cameras_unconfirmed: f64,
    /// Recall-corrected estimate (the headline).
    pub cameras_corrected: f64,
    /// ENU positions of those cameras (for highlighting on the map).
    pub camera_points: Vec<Vec2>,
    /// The source layer of each highlighted camera (aligned 1:1 with
    /// `camera_points`), so the map can style the highlight per layer.
    pub camera_kinds: Vec<SourceKind>,
    /// Physical-camera group ids captured anywhere in the walkshed (sorted). Lets the
    /// OD bake dedup commute legs against home/destination walkshed coverage.
    pub groups: Vec<u32>,
}

/// Count the distinct fixed cameras that could capture you anywhere within a
/// walkshed (their FOV covers any reachable street point).
pub fn walkshed_exposure(
    graph: &StreetGraph,
    ws: &Walkshed,
    sensors: &[SensorInstance],
    occ: &crate::occlusion::OccluderIndex,
    recall_factor: f64,
) -> WalkshedSummary {
    walkshed_exposure_with(graph, ws, sensors, occ, recall_factor, &FovModel::from_env(), None)
}

/// Per-group accumulator: certainty, recall class, and (for census cameras under the
/// arc-union model) the bearings of the visible sample points.
struct GroupAcc {
    confirmed: bool,
    /// A non-census member captured (known geometry) ⇒ the group counts in full.
    certain: bool,
    bearings: Vec<f64>,
}

/// [`walkshed_exposure`] with an explicit [`FovModel`] (batch passes one logged copy;
/// tests construct their own; the env-reading wrapper serves the app).
///
/// `sensor_index` (when provided) spatially culls the per-sample-point sensor loop
/// to only sensors within range, giving 50–200× fewer wedge tests on the app path.
pub fn walkshed_exposure_with(
    graph: &StreetGraph,
    ws: &Walkshed,
    sensors: &[SensorInstance],
    occ: &crate::occlusion::OccluderIndex,
    recall_factor: f64,
    fov: &FovModel,
    sensor_index: Option<&SensorIndex>,
) -> WalkshedSummary {
    let edges = &graph.asset().edges;
    // De-duplicate by physical-camera GROUP (assigned by `group_sensors`): a camera the
    // CCTV census *and* a DOT / ALPR / enforcement survey all record at one spot is one
    // device, counted once — not once per attesting layer. `s.confirmed` already carries
    // the group's confirmation (true if any member is surveyed). Under the arc-union
    // FOV model a census (`FixedCctv`) group cannot early-exit: every visible sample
    // point contributes a bearing to its heading-expectation, so its weight can only
    // grow toward 1 as the walkshed wraps around it. Groups with any surveyed-geometry
    // member (DOT disc, ALPR wedge with a real heading, enforcement) are certain on
    // first capture and short-circuit as before.
    let mut seen: HashMap<u32, GroupAcc> = HashMap::new();
    let mut camera_points: Vec<Vec2> = Vec::new();
    let mut camera_kinds: Vec<SourceKind> = Vec::new();

    let test_point = |pt: Vec2,
                      seen: &mut HashMap<u32, GroupAcc>,
                      camera_points: &mut Vec<Vec2>,
                      camera_kinds: &mut Vec<SourceKind>| {
        let candidates: Vec<usize> = match sensor_index {
            Some(idx) => idx.candidates(pt),
            None => (0..sensors.len()).collect(),
        };
        for si in candidates {
            let s = &sensors[si];
            let arc_mode = fov.enabled && s.kind == SourceKind::FixedCctv;
            if seen.get(&s.group).is_some_and(|acc| acc.certain) {
                continue;
            }
            if s.wedge.covers_unoccluded(pt) && !occ.blocked(s.wedge.apex, pt, s.host_poly) {
                let acc = seen.entry(s.group).or_insert_with(|| {
                    camera_points.push(s.wedge.apex);
                    camera_kinds.push(s.kind);
                    GroupAcc { confirmed: s.confirmed, certain: false, bearings: Vec::new() }
                });
                if arc_mode {
                    let (dy, dx) = (pt.y - s.wedge.apex.y, pt.x - s.wedge.apex.x);
                    acc.bearings.push(dy.atan2(dx));
                } else {
                    acc.certain = true;
                }
            }
        }
    };

    let mut scratch: Vec<Vec2> = Vec::new();
    for &ei in &ws.edges {
        sample_polyline_into(&edges[ei as usize].polyline, EXPOSURE_SAMPLE_STRIDE_M, true, 1.0, &mut scratch);
        for &pt in &scratch {
            test_point(pt, &mut seen, &mut camera_points, &mut camera_kinds);
        }
    }
    for &(ei, entry, frac) in &ws.partial {
        let e = &edges[ei as usize];
        sample_polyline_into(&e.polyline, EXPOSURE_SAMPLE_STRIDE_M, entry == e.from, frac, &mut scratch);
        for &pt in &scratch {
            test_point(pt, &mut seen, &mut camera_points, &mut camera_kinds);
        }
    }

    let raw = seen.len() as u32;
    // Confirmed (surveyed) groups count at face value; CCTV-census-only groups keep the
    // recall inflation (they stand in for cameras the street-view census missed). Census
    // groups additionally carry the heading-expectation weight w ∈ [mixture floor, 1].
    let mut unconfirmed = 0.0f64;
    let mut corrected = 0.0f64;
    let mut groups: Vec<u32> = Vec::with_capacity(seen.len());
    for (&g, acc) in &seen {
        let w = if acc.certain { 1.0 } else { fov.weight_for(&acc.bearings) };
        if acc.confirmed {
            corrected += w;
        } else {
            unconfirmed += w;
            corrected += w * recall_factor;
        }
        groups.push(g);
    }
    groups.sort_unstable();
    WalkshedSummary {
        max_minutes: ws.max_seconds / 60.0,
        // Fully reachable edges plus the boundary edges walkable in part.
        reachable_edges: ws.edges.len() + ws.partial.len(),
        cameras_raw: raw,
        cameras_unconfirmed: unconfirmed,
        cameras_corrected: corrected,
        camera_points,
        camera_kinds,
        groups,
    }
}

/// One fixed camera whose field of view directly covers a query point (for the
/// "Direct capture" mode). Carries the wedge geometry so the map can draw its facing cone.
#[derive(Debug, Clone, Copy)]
pub struct CapturingCamera {
    pub apex: Vec2,
    pub kind: SourceKind,
    pub heading_rad: f64,
    pub half_fov_rad: f64,
    pub range_m: f64,
}

/// Result of a "who is pointed directly at this exact spot" query.
#[derive(Debug, Clone, Default)]
pub struct DirectCaptureSummary {
    /// Distinct fixed cameras whose FOV covers the point (as detected).
    pub cameras_raw: u32,
    /// Recall-corrected estimate (the headline).
    pub cameras_corrected: f64,
    /// The capturing cameras (deduped by group), for map highlighting + facing cones.
    pub cameras: Vec<CapturingCamera>,
}

/// Count the distinct fixed cameras whose field of view **directly covers `point`** —
/// facing it, within range, sightline unoccluded. The stricter cousin of
/// [`walkshed_exposure`]: not "could a camera see you *somewhere* in a 10-minute walk",
/// but "is a camera pointed at *this exact address*". Deduped by physical-camera `group`
/// (like the walkshed / A→B paths) so co-located cross-source attestations count once,
/// and recall-corrected the same way (surveyed groups at face value, CCTV-census-only
/// groups keep the recall inflation).
pub fn direct_capture_exposure(
    sensors: &[SensorInstance],
    occ: &crate::occlusion::OccluderIndex,
    point: Vec2,
    recall_factor: f64,
) -> DirectCaptureSummary {
    let mut seen: HashMap<u32, bool> = HashMap::new();
    let mut cameras: Vec<CapturingCamera> = Vec::new();
    for s in sensors {
        if !seen.contains_key(&s.group)
            && s.wedge.covers_unoccluded(point)
            && !occ.blocked(s.wedge.apex, point, s.host_poly)
        {
            seen.insert(s.group, s.confirmed);
            cameras.push(CapturingCamera {
                apex: s.wedge.apex,
                kind: s.kind,
                heading_rad: s.wedge.heading_rad,
                half_fov_rad: s.wedge.half_fov_rad,
                range_m: s.wedge.range_m,
            });
        }
    }
    let cameras_corrected = seen
        .values()
        .map(|&confirmed| if confirmed { 1.0 } else { recall_factor })
        .sum();
    DirectCaptureSummary {
        cameras_raw: seen.len() as u32,
        cameras_corrected,
        cameras,
    }
}

/// Per-class exposure for the breakdown panel.
#[derive(Debug, Clone, Copy)]
pub struct SourceBreakdown {
    pub kind: SourceKind,
    pub tier: ConfidenceTier,
    /// Expected distinct devices that captured you (recall-corrected for fixed).
    pub devices: f64,
    /// Poisson probability of at least one capture from this class.
    pub p_at_least_one: f64,
}

/// A compact, display-ready summary of a routed walk's exposure.
#[derive(Debug, Clone)]
pub struct RouteSummary {
    pub route_len_m: f64,
    pub duration_s: f64,
    /// Headline: "~N cameras could have captured you" (recall-corrected).
    pub headline_devices: u32,
    pub total_expected_frames: f64,
    pub fraction_surveilled: f64,
    /// Per-class detail (only classes that contributed any exposure).
    pub breakdown: Vec<SourceBreakdown>,
    pub tally: ExposureTally,
}

/// Route between two ENU points, simulate full exposure (fixed + mobile), and
/// summarize. `departure_hour` (0–24) scales the time-dependent mobile classes.
#[allow(clippy::too_many_arguments)]
pub fn run_route(
    graph: &StreetGraph,
    sensors: &[SensorInstance],
    occ: &OccluderIndex,
    mobile: &MobileScenario,
    from: Vec2,
    to: Vec2,
    params: SimParams,
    departure_hour: f64,
    dashcam_field: Option<&DashcamFieldLayer>,
    robot_field: Option<&RobotabilityField>,
    tesla_field: Option<&TeslaField>,
    real: Option<&RealDayRates>,
    sensor_index: Option<&SensorIndex>,
    ace_grid: Option<&AceGrid>,
) -> Result<(Route, RouteSummary), RouteError> {
    let route = graph.route_points(from, to)?;
    let summary = summarize(
        &route, sensors, occ, mobile, params, departure_hour, dashcam_field, robot_field,
        tesla_field, real, sensor_index, ace_grid,
    );
    Ok((route, summary))
}

/// Simulate exposure for an already-computed route and summarize. Lets the app
/// re-evaluate when scenario sliders / departure hour change without re-routing.
#[allow(clippy::too_many_arguments)]
pub fn summarize(
    route: &Route,
    sensors: &[SensorInstance],
    occ: &OccluderIndex,
    mobile: &MobileScenario,
    params: SimParams,
    departure_hour: f64,
    dashcam_field: Option<&DashcamFieldLayer>,
    robot_field: Option<&RobotabilityField>,
    tesla_field: Option<&TeslaField>,
    real: Option<&RealDayRates>,
    sensor_index: Option<&SensorIndex>,
    ace_grid: Option<&AceGrid>,
) -> RouteSummary {
    let tally = simulate_full(
        route, sensors, occ, mobile, params, departure_hour, dashcam_field, robot_field,
        tesla_field, real, sensor_index, ace_grid,
    );

    let breakdown: Vec<SourceBreakdown> = SourceKind::ALL
        .iter()
        .filter(|&&k| tally.adjusted_devices(k) > 1e-6)
        .map(|&k| SourceBreakdown {
            kind: k,
            tier: k.tier(),
            devices: tally.adjusted_devices(k),
            p_at_least_one: tally.p_capture(k),
        })
        .collect();

    RouteSummary {
        route_len_m: route.total_m,
        duration_s: route.total_m / params.speed_mps,
        headline_devices: tally.headline_device_count(),
        total_expected_frames: tally.total_expected_frames(),
        fraction_surveilled: tally.fraction_surveilled(),
        breakdown,
        tally,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sensor(x: f64, y: f64, kind: SourceKind, id: u64) -> SensorInstance {
        SensorInstance {
            wedge: FrustumWedge::from_degrees(Vec2::new(x, y), None, 360.0, 10.0),
            frame_rate: 1.0,
            id,
            kind,
            group: id as u32,
            confirmed: false,
            host_poly: None,
        }
    }

    /// One straight 2-vertex street of `len_m` meters — the CSCL common case (67% of
    /// citywide edges), and exactly the shape whose interior the old first/middle/last
    /// vertex sampling could never see.
    fn line_graph(len_m: f64) -> StreetGraph {
        use crate::assets::{EdgeData, GraphAsset, NodePoint, Provenance};
        use crate::projection::GeoOrigin;
        StreetGraph::from_asset(GraphAsset {
            origin: GeoOrigin::MANHATTAN,
            nodes: vec![NodePoint { x: 0.0, y: 0.0 }, NodePoint { x: len_m, y: 0.0 }],
            edges: vec![EdgeData {
                from: 0,
                to: 1,
                length_m: len_m,
                polyline: vec![[0.0, 0.0], [len_m, 0.0]],
                segment_id: None,
            }],
            provenance: Provenance {
                source: String::new(),
                url: String::new(),
                license: String::new(),
                as_of: String::new(),
                notes: String::new(),
            },
        })
    }

    /// Geometry/recall tests run under the 360°-disc model: they exercise sampling,
    /// grouping and recall linearity, not the census-FOV expectation (tested separately).
    fn walkshed_exposure_disc(
        g: &StreetGraph,
        ws: &crate::graph::Walkshed,
        sensors: &[SensorInstance],
        occ: &OccluderIndex,
        recall: f64,
    ) -> WalkshedSummary {
        let fov = FovModel { enabled: false, ..FovModel::default() };
        walkshed_exposure_with(g, ws, sensors, occ, recall, &fov, None)
    }

    fn cam15(x: f64, y: f64, id: u64) -> SensorInstance {
        SensorInstance {
            wedge: FrustumWedge::from_degrees(Vec2::new(x, y), None, 360.0, 15.0),
            frame_rate: 1.0,
            id,
            kind: SourceKind::FixedCctv,
            group: id as u32,
            confirmed: false,
            host_poly: None,
        }
    }

    /// Consecutive samples are never more than the stride apart, both span ends are
    /// emitted, and a partial fraction stops at the cut — the properties the 10 m
    /// detection-completeness argument rests on.
    #[test]
    fn sample_polyline_respects_stride_and_cut() {
        let poly = vec![[0.0, 0.0], [37.0, 0.0], [37.0, 25.0], [90.0, 25.0]];
        let pts = sample_polyline(&poly, 10.0, true, 1.0);
        assert!(pts.first().unwrap().distance(Vec2::new(0.0, 0.0)) < 1e-9);
        assert!(pts.last().unwrap().distance(Vec2::new(90.0, 25.0)) < 1e-9);
        for w in pts.windows(2) {
            assert!(w[0].distance(w[1]) <= 10.0 + 1e-9, "gap {}", w[0].distance(w[1]));
        }
        // Half the edge from the far end covers arc length 56 of 112: the cut lands on the
        // middle segment, and no sample crosses it toward the near end.
        let half = sample_polyline(&poly, 10.0, false, 0.5);
        assert!(half.first().unwrap().distance(Vec2::new(90.0, 25.0)) < 1e-9);
        let total = 37.0 + 25.0 + 53.0;
        for p in &half {
            let arc_from_far = if (p.y - 25.0).abs() < 1e-9 {
                90.0 - p.x // top horizontal segment, walked backward from (90, 25)
            } else if (p.x - 37.0).abs() < 1e-9 {
                53.0 + (25.0 - p.y) // vertical segment
            } else {
                53.0 + 25.0 + (37.0 - p.x) // bottom horizontal (unreachable at frac 0.5)
            };
            assert!(arc_from_far <= total * 0.5 + 1e-6, "sample past the cut: {p:?}");
        }
    }

    /// A mid-block camera on a straight 2-vertex block is detected. Under the previous
    /// 3-vertex sampling this exact case was structurally invisible: the only samples were
    /// the two intersections, 100 m apart, and a 15 m camera at mid-block covered neither.
    #[test]
    fn mid_block_camera_is_detected() {
        let g = line_graph(100.0);
        let ws = g.walkshed(0, 200.0, 1.0); // whole edge reachable
        assert_eq!(ws.edges.len(), 1);
        let sensors = vec![cam15(50.0, 5.0, 0)];
        let s = walkshed_exposure_disc(&g, &ws, &sensors, &OccluderIndex::empty(), 1.0);
        assert_eq!(s.cameras_raw, 1, "mid-block camera missed by walkshed sampling");
    }

    /// Boundary edges count only the street a walker can actually reach: a camera on the
    /// near half of a cut edge is seen, one past the cut is not.
    #[test]
    fn partial_edge_sampled_only_to_the_cut() {
        let g = line_graph(100.0);
        let ws = g.walkshed(0, 50.0, 1.0); // 50 s at 1 m/s: half the edge, entered from node 0
        assert!(ws.edges.is_empty());
        assert_eq!(ws.partial.len(), 1);
        let (_, entry, frac) = ws.partial[0];
        assert_eq!(entry, 0);
        assert!((frac - 0.5).abs() < 1e-9);

        let sensors = vec![cam15(30.0, 5.0, 0), cam15(80.0, 5.0, 1)];
        let s = walkshed_exposure_disc(&g, &ws, &sensors, &OccluderIndex::empty(), 1.0);
        assert_eq!(s.cameras_raw, 1, "expected only the near-half camera");
        assert!(s.camera_points[0].distance(Vec2::new(30.0, 5.0)) < 1e-9);
    }

    /// **Occlusion is subtractive.** A building can only ever *remove* a camera from `R_i`, never add
    /// one — so `R_occluded ⊆ R_free`, always.
    ///
    /// This is the contract that makes the paper's headline *interpretable*. We report that occlusion
    /// changes `R_i` by −0.01%; a sign error, or a `host_poly` wired to the wrong index (excluding an
    /// arbitrary unrelated building from every camera's sightlines), would produce a number of the
    /// same tiny magnitude and read exactly the same in any summary statistic. The geometry tests in
    /// `occlusion.rs` prove `blocked()` is correct in isolation; this proves the *instrument* uses it
    /// in the only direction physics allows.
    #[test]
    fn occlusion_can_only_remove_cameras_never_add_them() {
        use crate::assets::{EdgeData, GraphAsset, NodePoint, Provenance};
        use crate::projection::GeoOrigin;

        // A 60 m street with cameras along it, and a building slab parked between the street and a
        // camera set back from it — so some, but not all, sightlines die.
        let asset = GraphAsset {
            origin: GeoOrigin::MANHATTAN,
            nodes: vec![NodePoint { x: 0.0, y: 0.0 }, NodePoint { x: 60.0, y: 0.0 }],
            edges: vec![EdgeData {
                from: 0,
                to: 1,
                length_m: 60.0,
                polyline: vec![[0.0, 0.0], [30.0, 0.0], [60.0, 0.0]],
                segment_id: None,
            }],
            provenance: Provenance {
                source: String::new(),
                url: String::new(),
                license: String::new(),
                as_of: String::new(),
                notes: String::new(),
            },
        };
        let graph = StreetGraph::from_asset(asset);
        let ws = graph.walkshed(0, 600.0, 1.34);
        let sensors = vec![
            sensor(0.0, 0.0, SourceKind::FixedCctv, 0),
            sensor(30.0, 6.0, SourceKind::FixedCctv, 1), // set back behind the slab below
            sensor(60.0, 0.0, SourceKind::Alpr, 2),
        ];

        // A wall spanning the street frontage at y = 3, between sensor 1 and the street.
        let rings = vec![vec![[20.0, 3.0], [40.0, 3.0], [40.0, 4.0], [20.0, 4.0]]];
        let occ = OccluderIndex::from_rings(rings, crate::occlusion::DEFAULT_CELL_M);

        let free = walkshed_exposure_disc(&graph, &ws, &sensors, &OccluderIndex::empty(), 1.0);
        let occl = walkshed_exposure_disc(&graph, &ws, &sensors, &occ, 1.0);

        assert!(
            occl.cameras_raw <= free.cameras_raw,
            "occlusion added cameras ({} > {}) — the sightline test is inverted",
            occl.cameras_raw,
            free.cameras_raw
        );
        // ...and the surviving set is a strict subset, not merely a smaller count.
        for p in &occl.camera_points {
            assert!(
                free.camera_points.contains(p),
                "occlusion produced a camera at {p:?} that free space never saw"
            );
        }
        // Non-vacuous: the slab must actually blind the set-back camera, or this test proves nothing.
        assert_eq!(free.cameras_raw, 3, "free space sees all three");
        assert_eq!(occl.cameras_raw, 2, "the slab blinds the set-back camera, and only that one");
    }

    #[test]
    fn group_sensors_merges_across_sources_only() {
        // s0+s1+s2 cluster at the origin (CCTV+DOT+CCTV → one node via the DOT bridge);
        // s3 and s4 are two same-source CCTV cameras 3 m apart far away (stay distinct).
        let mut s = vec![
            sensor(0.0, 0.0, SourceKind::FixedCctv, 0),
            sensor(5.0, 0.0, SourceKind::DotLiveView, 1),
            sensor(8.0, 0.0, SourceKind::FixedCctv, 2),
            sensor(100.0, 0.0, SourceKind::FixedCctv, 3),
            sensor(103.0, 0.0, SourceKind::FixedCctv, 4),
        ];
        let n = group_sensors(&mut s, 15.0);
        assert_eq!(n, 3, "{{cctv+dot+cctv}}, {{cctv}}, {{cctv}}");
        assert_eq!(s[0].group, s[1].group);
        assert_eq!(s[1].group, s[2].group);
        assert!(s[0].confirmed, "a surveyed (DOT) attestation confirms the group");
        assert_ne!(s[3].group, s[4].group, "two same-source CCTV stay distinct cameras");
        assert!(!s[3].confirmed && !s[4].confirmed);
    }

    #[test]
    fn walkshed_reports_camera_kinds_aligned_with_points() {
        use crate::assets::{EdgeData, GraphAsset, NodePoint, Provenance};
        use crate::projection::GeoOrigin;
        // A single 50 m street; one ALPR at one end, one DOT cam at the other.
        let asset = GraphAsset {
            origin: GeoOrigin::MANHATTAN,
            nodes: vec![NodePoint { x: 0.0, y: 0.0 }, NodePoint { x: 50.0, y: 0.0 }],
            edges: vec![EdgeData {
                from: 0,
                to: 1,
                length_m: 50.0,
                polyline: vec![[0.0, 0.0], [50.0, 0.0]],
                segment_id: None,
            }],
            provenance: Provenance {
                source: String::new(),
                url: String::new(),
                license: String::new(),
                as_of: String::new(),
                notes: String::new(),
            },
        };
        let graph = StreetGraph::from_asset(asset);
        let ws = graph.walkshed(0, 600.0, 1.34);
        let sensors = vec![
            sensor(0.0, 0.0, SourceKind::Alpr, 0),
            sensor(50.0, 0.0, SourceKind::DotLiveView, 1),
        ];
        let sum = walkshed_exposure_disc(&graph, &ws, &sensors, &OccluderIndex::empty(), 1.0);
        // The per-layer highlight depends on kinds staying aligned 1:1 with points.
        assert_eq!(sum.camera_points.len(), sum.camera_kinds.len());
        assert_eq!(sum.cameras_raw, 2, "both cameras cover the street");
        assert!(sum.camera_kinds.contains(&SourceKind::Alpr));
        assert!(sum.camera_kinds.contains(&SourceKind::DotLiveView));
    }

    fn one_edge_graph(len_m: f64) -> StreetGraph {
        use crate::assets::{EdgeData, GraphAsset, NodePoint, Provenance};
        use crate::projection::GeoOrigin;
        StreetGraph::from_asset(GraphAsset {
            origin: GeoOrigin::MANHATTAN,
            nodes: vec![NodePoint { x: 0.0, y: 0.0 }, NodePoint { x: len_m, y: 0.0 }],
            edges: vec![EdgeData {
                from: 0,
                to: 1,
                length_m: len_m,
                polyline: vec![[0.0, 0.0], [len_m, 0.0]],
                segment_id: None,
            }],
            provenance: Provenance {
                source: String::new(),
                url: String::new(),
                license: String::new(),
                as_of: String::new(),
                notes: String::new(),
            },
        })
    }

    #[test]
    fn walkshed_dedups_colocated_cross_source_camera() {
        // The #60 accuracy requirement: a DOT camera and a DeFlock ALPR at the same spot
        // are one physical device reported by two layers — the My-area headline must
        // count it ONCE, not once per source.
        let graph = one_edge_graph(20.0);
        let ws = graph.walkshed(0, 600.0, 1.34);
        let mut sensors = vec![
            sensor(0.0, 0.0, SourceKind::DotLiveView, 0),
            sensor(2.0, 0.0, SourceKind::Alpr, 1), // 2 m away → same physical camera
        ];
        group_sensors(&mut sensors, 15.0);
        assert_eq!(sensors[0].group, sensors[1].group, "co-located cross-source → one group");
        let sum = walkshed_exposure_disc(&graph, &ws, &sensors, &OccluderIndex::empty(), 1.5);
        assert_eq!(sum.cameras_raw, 1, "the physical camera counts once, not once per layer");
        // DOT + ALPR are both surveyed → confirmed group → no recall inflation.
        assert_eq!(sum.cameras_corrected, 1.0, "a confirmed group counts at face value");
    }

    #[test]
    fn walkshed_recall_inflates_only_unconfirmed_groups() {
        // A lone CCTV-census camera (unconfirmed by any survey) keeps the recall
        // inflation, so the My-area corrected count matches the A→B grouped semantics.
        let graph = one_edge_graph(20.0);
        let ws = graph.walkshed(0, 600.0, 1.34);
        let mut sensors = vec![sensor(0.0, 0.0, SourceKind::FixedCctv, 0)];
        group_sensors(&mut sensors, 15.0);
        let sum = walkshed_exposure_disc(&graph, &ws, &sensors, &OccluderIndex::empty(), 1.5);
        assert_eq!(sum.cameras_raw, 1);
        assert_eq!(sum.cameras_unconfirmed, 1.0);
        assert!(
            (sum.cameras_corrected - 1.5).abs() < 1e-9,
            "an unconfirmed (CCTV-only) camera keeps the recall factor"
        );
    }

    #[test]
    fn recall_correction_is_linear_and_reconstructable_from_one_bake() {
        // THE load-bearing property of the recall design: because the correction inflates
        // only the unconfirmed sub-population, `cameras_corrected` under ANY recall r is
        // recoverable from a single bake as
        //     cameras(r) = cameras_unconfirmed / r + (cameras_raw - cameras_unconfirmed)
        // This is what lets the undercount bootstrap draw hundreds of recall values without
        // re-running the batch. If this test fails, the sweep + the uncertainty band are wrong.
        // Edge endpoints (0,0) and (20,0) are the sampled walkshed points; `sensor()` is a
        // 10 m omnidirectional wedge, and `group_sensors` merges across sources within 15 m.
        let graph = one_edge_graph(20.0);
        let ws = graph.walkshed(0, 600.0, 1.34);
        let mut sensors = vec![
            sensor(0.0, 0.0, SourceKind::FixedCctv, 0),   // unconfirmed  (>15 m from the DOT cam)
            sensor(0.0, 4.0, SourceKind::FixedCctv, 1),   // unconfirmed  (same source as #0 ⇒ no merge)
            sensor(20.0, 0.0, SourceKind::FixedCctv, 2),  // confirmed    (co-located DOT survey)
            sensor(20.0, 0.0, SourceKind::DotLiveView, 3), // groups with #2 ⇒ face value, no inflation
        ];
        group_sensors(&mut sensors, 15.0);

        // Bake ONCE at r = 1 (raw): this is all a caller ever needs to persist.
        let base = walkshed_exposure_disc(&graph, &ws, &sensors, &OccluderIndex::empty(), 1.0);
        let raw = base.cameras_raw as f64;
        let unconf = base.cameras_unconfirmed;
        assert!(unconf > 0.0 && unconf < raw, "need a mix of confirmed and unconfirmed");

        for &r in &[0.458, 0.501, 0.544, 0.63, 1.0] {
            let direct = walkshed_exposure_disc(&graph, &ws, &sensors, &OccluderIndex::empty(), 1.0 / r).cameras_corrected;
            let reconstructed = unconf / r + (raw - unconf);
            assert!(
                (direct - reconstructed).abs() < 1e-9,
                "recall r={r}: re-running gave {direct}, reconstruction gave {reconstructed}"
            );
        }
    }

    #[test]
    fn arc_union_fraction_exact_cases() {
        use std::f64::consts::{PI, TAU};
        let fov = 70f64.to_radians();
        // One bearing: exactly the FOV window.
        assert!((arc_union_fraction(&[0.0], fov) - fov / TAU).abs() < 1e-12);
        // Two opposite bearings: disjoint windows, twice the coverage.
        assert!((arc_union_fraction(&[0.0, PI], fov) - 2.0 * fov / TAU).abs() < 1e-12);
        // Identical bearings collapse to one window.
        assert!((arc_union_fraction(&[1.0, 1.0, 1.0], fov) - fov / TAU).abs() < 1e-12);
        // Wraparound: a window straddling ±180° merges with one at +180°.
        assert!((arc_union_fraction(&[PI - 0.01, -PI + 0.01], fov) - (fov + 0.02) / TAU).abs() < 1e-9);
        // A wide-enough FOV covers the circle.
        assert!((arc_union_fraction(&[0.0], TAU) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn census_camera_weight_is_the_heading_mixture() {
        // Camera at the west end of a straight 20 m edge: every visible sample lies due
        // east, so the bearing set is a single direction and the census weight is
        // p_dome + p_bullet * fov/360 — the point-level mixture.
        let fov = FovModel::default();
        let graph = one_edge_graph(20.0);
        let ws = graph.walkshed(0, 600.0, 1.34);
        let mut sensors = vec![sensor(0.0, 0.0, SourceKind::FixedCctv, 0)];
        group_sensors(&mut sensors, 15.0);
        let sum = walkshed_exposure_with(&graph, &ws, &sensors, &OccluderIndex::empty(), 1.0, &fov, None);
        let want = (1.0 - fov.p_bullet) + fov.p_bullet * fov.bullet_fov_rad / std::f64::consts::TAU;
        assert!((sum.cameras_corrected - want).abs() < 1e-9, "got {}", sum.cameras_corrected);
        assert_eq!(sum.cameras_raw, 1, "raw stays a touch count");
        assert_eq!(sum.groups.len(), 1);
    }

    #[test]
    fn street_on_both_sides_widens_the_arc_toward_one() {
        // Mid-block camera: samples to its east AND west double the bearing arc, so the
        // path-based weight strictly exceeds the single-direction (point-like) weight —
        // the §10b point-vs-path structure.
        let fov = FovModel::default();
        let graph = one_edge_graph(20.0);
        let ws = graph.walkshed(0, 600.0, 1.34);
        let end = vec![sensor(0.0, 0.0, SourceKind::FixedCctv, 0)];
        let mid = vec![sensor(10.0, 0.0, SourceKind::FixedCctv, 0)];
        let w_end = walkshed_exposure_with(&graph, &ws, &end, &OccluderIndex::empty(), 1.0, &fov, None)
            .cameras_corrected;
        let w_mid = walkshed_exposure_with(&graph, &ws, &mid, &OccluderIndex::empty(), 1.0, &fov, None)
            .cameras_corrected;
        let two_sided =
            (1.0 - fov.p_bullet) + fov.p_bullet * 2.0 * fov.bullet_fov_rad / std::f64::consts::TAU;
        assert!((w_mid - two_sided).abs() < 1e-9, "got {w_mid}");
        assert!(w_mid > w_end + 0.02);
    }

    #[test]
    fn surveyed_member_makes_a_census_group_certain() {
        // Census camera + co-located DOT survey: the group is one physical camera with
        // KNOWN coverage, so the heading expectation must not discount it.
        let fov = FovModel::default();
        let graph = one_edge_graph(20.0);
        let ws = graph.walkshed(0, 600.0, 1.34);
        let mut sensors = vec![
            sensor(0.0, 0.0, SourceKind::FixedCctv, 0),
            sensor(1.0, 0.0, SourceKind::DotLiveView, 1),
        ];
        group_sensors(&mut sensors, 15.0);
        let sum = walkshed_exposure_with(&graph, &ws, &sensors, &OccluderIndex::empty(), 1.0, &fov, None);
        assert_eq!(sum.cameras_raw, 1);
        assert!((sum.cameras_corrected - 1.0).abs() < 1e-12, "certain group counts in full");
    }

    #[test]
    fn direct_capture_counts_only_cameras_framing_the_point() {
        // Heading convention (see geometry.rs): 0° = +Y/north, 90° = +X/east, 180° = south.
        let point = Vec2::ZERO;
        let cam = |x: f64, y: f64, heading_deg: f64, id: u64, group: u32| SensorInstance {
            wedge: FrustumWedge::from_degrees(Vec2::new(x, y), Some(heading_deg), 60.0, 30.0),
            frame_rate: 1.0,
            id,
            kind: SourceKind::FixedCctv,
            group,
            confirmed: false,
            host_poly: None,
        };
        // A: 5 m north of the point, aimed south (180°) → frames it.
        // B: 5 m east, aimed east (90°, away) → the point is 180° off-axis → misses.
        // C: co-located with A and in the same physical group → must not double-count.
        let sensors = vec![
            cam(0.0, 5.0, 180.0, 0, 0),
            cam(5.0, 0.0, 90.0, 1, 1),
            cam(0.2, 5.0, 180.0, 2, 0),
        ];
        let sum = direct_capture_exposure(&sensors, &OccluderIndex::empty(), point, 1.5);
        assert_eq!(sum.cameras_raw, 1, "one physical camera frames the point (group-deduped)");
        assert_eq!(sum.cameras.len(), 1);
        assert!(
            (sum.cameras_corrected - 1.5).abs() < 1e-9,
            "the lone unconfirmed group keeps the recall factor"
        );

        // A wall across the sightlines (y = 2.5) blocks the direct capture entirely.
        let wall = [crate::geometry::OccluderEdge {
            a: Vec2::new(-5.0, 2.5),
            b: Vec2::new(5.0, 2.5),
        }];
        let occ = OccluderIndex::from_edges(&wall, crate::occlusion::DEFAULT_CELL_M);
        let blocked = direct_capture_exposure(&sensors, &occ, point, 1.5);
        assert_eq!(blocked.cameras_raw, 0, "an occluding wall removes the direct capture");
        assert!(blocked.cameras.is_empty());
    }
}
