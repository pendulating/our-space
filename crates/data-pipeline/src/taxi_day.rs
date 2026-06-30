//! Bake a single real day of NYC TLC rideshare (HVFHV) trips for schedule-driven
//! replay: a shared routed-O-D pool, the day's real trips (pickup minute + O-D +
//! real duration), and the full per-minute O-D aggregate (for the analytical flux).
//!
//! Inputs (fetched once via DuckDB — see README/fetch-taxi-day): the baked drive
//! graph, the taxi-zone GeoJSON, a per-minute O-D CSV
//! (`pu_min,PULocationID,DOLocationID,trips`) and a trip CSV
//! (`pu_min,PULocationID,DOLocationID,dur_min[,trip_miles,trip_time]`). The two
//! optional columns (HVFHV routed distance + duration) drive Stage-3 route
//! inference; without them each trip falls back to the fastest route.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use sim_core::assets::{
    GraphAsset, Provenance, SensingPower, TaxiDayLayer, TaxiOdMinute, TaxiRoute, TaxiTrip,
    VehicleRoute,
};
use sim_core::graph::{Route, StreetGraph};
use sim_core::math::Vec2;
use sim_core::projection::{EnuProjection, GeoOrigin};

use crate::dashcam::parse_zones;
use crate::vehicle_routes::{decimate, zone_centroid};

const DECIMATE_TOL_M: f64 = 6.0;
/// Endpoint representatives sampled per zone; trip O/D fan out across these
/// instead of all snapping to one centroid node (spreads coverage off the spines).
const K_ENDPOINTS: usize = 6;
/// Distinct O-D zone pairs that get a centroid route (breadth + per-trip fallback).
/// Citywide has ~35.3k distinct pairs; 40k routes every one (option-1 "all taxis"
/// experiment — was 5,000, which covered only ~71% of citywide trips).
const MAX_OD_PAIRS: usize = 40000;
/// Global cap on baked routes (centroid + per-trip route candidates). Effectively
/// uncapped for the option-1 experiment: every O-D pair + endpoint-combo candidate is
/// kept (max ~MAX_OD_PAIRS × (1 + 2·MAX_COMBOS_PER_PAIR) routes), so no trip falls back
/// to a generic base route. Grows the asset; does NOT affect runtime FPS (that's the
/// MAX_VEHICLES pool). Re-cap once option 2 lands a leaner route representation.
const MAX_ROUTES: usize = usize::MAX;
/// Distinct endpoint combos that get a candidate set, per zone pair.
const MAX_COMBOS_PER_PAIR: usize = 4;
/// Highway/ramp cost multiplier for the *surface* alternative candidate.
const HW_PENALTY: f64 = 8.0;
/// Distance-likelihood tolerance (relative): `Normal(L; D, (SIG_D·D)²)`.
const SIG_D: f64 = 0.35;
/// Surface free-flow speed (25 mph) — reference for calibrating the slowdown prior.
const SURF_SPEED_MPS: f64 = 11.176;

/// Shoelace area magnitude of a ring (to pick a zone's largest part).
fn ring_area2(ring: &[[f64; 2]]) -> f64 {
    let n = ring.len();
    if n < 3 {
        return 0.0;
    }
    let mut a = 0.0;
    let mut j = n - 1;
    for i in 0..n {
        a += (ring[j][0] + ring[i][0]) * (ring[j][1] - ring[i][1]);
        j = i;
    }
    a.abs()
}

/// Ray-cast point-in-polygon against a single ring (ENU).
fn point_in_ring(p: Vec2, ring: &[[f64; 2]]) -> bool {
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = (ring[i][0], ring[i][1]);
        let (xj, yj) = (ring[j][0], ring[j][1]);
        if (yi > p.y) != (yj > p.y) && p.x < (xj - xi) * (p.y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Deterministic 64-bit hash (splitmix64) for reproducible endpoint sampling
/// without an RNG dependency — keyed by trip index + a salt.
fn hash64(x: u64, salt: u64) -> u64 {
    let mut z = x
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(salt.wrapping_mul(0xD1B5_4A32_D192_ED03))
        .wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Route between two graph nodes by free-flow time → a decimated `VehicleRoute`
/// plus the edge indices it traverses (for the sensing-power metric).
fn route_vr(graph: &StreetGraph, a: u32, b: u32) -> Option<(VehicleRoute, Vec<u32>)> {
    if a == b {
        return None;
    }
    let (route, _t, edges) = graph.route_timed_pen(a, b, 1.0).ok()?;
    Some((route_to_vr(&route)?, edges))
}

/// Per-node building mass: sum of nearby footprint areas (m²). This is the
/// dasymetric weight — it separates *concentrated* zones (mass piled into a
/// dense cluster, e.g. a hub) from *dispersed* ones (even residential mass).
fn node_building_mass(
    graph: &StreetGraph,
    footprints_path: &str,
    proj: &EnuProjection,
) -> Result<Vec<f64>> {
    let bytes =
        std::fs::read(footprints_path).with_context(|| format!("reading {footprints_path}"))?;
    let fc: geojson::FeatureCollection =
        serde_json::from_slice(&bytes).context("parsing footprints GeoJSON")?;
    // (centroid xy, area) per building part.
    let mut bld: Vec<([f64; 2], f64)> = Vec::new();
    for f in fc.features {
        let Some(geom) = f.geometry else { continue };
        let rings: Vec<Vec<[f64; 2]>> = match geom.value {
            geojson::Value::Polygon(rings) => {
                rings.into_iter().take(1).map(|r| r.iter().map(|p| [p[0], p[1]]).collect()).collect()
            }
            geojson::Value::MultiPolygon(polys) => polys
                .into_iter()
                .filter_map(|poly| poly.into_iter().next())
                .map(|r| r.iter().map(|p| [p[0], p[1]]).collect())
                .collect(),
            _ => continue,
        };
        for ring in rings {
            if ring.len() < 4 {
                continue;
            }
            let enu: Vec<[f64; 2]> = ring
                .iter()
                .map(|p| {
                    let e = proj.to_enu(p[1], p[0]);
                    [e.x, e.y]
                })
                .collect();
            let area = crate::footprints::ring_area_m2(&enu);
            if area < 1.0 {
                continue;
            }
            let n = enu.len() as f64;
            let cx = enu.iter().map(|p| p[0]).sum::<f64>() / n;
            let cy = enu.iter().map(|p| p[1]).sum::<f64>() / n;
            bld.push(([cx, cy], area));
        }
    }
    // Grid-index building centroids → per-node nearby footprint area within R.
    const CELL: f64 = 100.0;
    const R: f64 = 120.0;
    let cell = |x: f64, y: f64| ((x / CELL).floor() as i64, (y / CELL).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, (c, _)) in bld.iter().enumerate() {
        grid.entry(cell(c[0], c[1])).or_default().push(i);
    }
    let r2 = R * R;
    let reach = (R / CELL).ceil() as i64;
    let n_nodes = graph.node_count();
    let mut mass = vec![0.0f64; n_nodes];
    for nid in 0..n_nodes {
        let p = graph.node_pos(nid as u32);
        let (cx, cy) = cell(p.x, p.y);
        let mut m = 0.0;
        for gx in (cx - reach)..=(cx + reach) {
            for gy in (cy - reach)..=(cy + reach) {
                let Some(ids) = grid.get(&(gx, gy)) else { continue };
                for &bi in ids {
                    let (c, a) = bld[bi];
                    let (dx, dy) = (c[0] - p.x, c[1] - p.y);
                    if dx * dx + dy * dy <= r2 {
                        m += a;
                    }
                }
            }
        }
        mass[nid] = m;
    }
    eprintln!("  building mass: {} footprint parts → {n_nodes} nodes weighted", bld.len());
    Ok(mass)
}

/// Pick an endpoint weighted by its stored mass (hashed uniform → cumulative).
fn weighted_pick(eps: &[(u32, f64)], seed: u64) -> u32 {
    let total: f64 = eps.iter().map(|e| e.1).sum();
    if total <= 0.0 {
        return eps[hash64(seed, 19) as usize % eps.len()].0;
    }
    let mut r = hash64(seed, 11) as f64 / u64::MAX as f64 * total;
    for &(nid, w) in eps {
        r -= w;
        if r <= 0.0 {
            return nid;
        }
    }
    eps[eps.len() - 1].0
}

/// Decimate an already-computed `Route` into a `VehicleRoute` (no re-routing).
fn route_to_vr(route: &Route) -> Option<VehicleRoute> {
    let pts = decimate(&route.points, DECIMATE_TOL_M);
    if pts.len() < 2 {
        return None;
    }
    Some(VehicleRoute {
        polyline: pts.iter().map(|p| [p.x as f32, p.y as f32]).collect(),
        length_m: route.total_m as f32,
        weight: 1.0,
    })
}

/// Route-inference score for a candidate of length `len_m` and free-flow time
/// `t_r` (s), given the trip's real distance `d_m` (m) and time `t_obs` (s):
/// `Normal(len; d, (SIG_D·d)²) · lognormal(t_obs/t_r; s0, τ)`. The slowdown term
/// is load-bearing — a trip too fast for a candidate's free-flow time (slowdown
/// well below the typical `s0`) scores near zero, so only the faster (highway)
/// candidate can reproduce a fast trip. `ln_s0`/`tau` are calibrated from data.
fn infer_score(len_m: f64, t_r: f64, d_m: f64, t_obs: f64, ln_s0: f64, tau: f64) -> f64 {
    if t_r <= 1.0 || d_m <= 0.0 || t_obs <= 0.0 {
        return 0.0;
    }
    let dl = (len_m - d_m) / d_m;
    let nrm = (-(dl * dl) / (2.0 * SIG_D * SIG_D)).exp();
    let z = ((t_obs / t_r).ln() - ln_s0) / tau;
    let g = (-0.5 * z * z).exp();
    nrm * g
}

pub fn bake(
    graph_path: &str,
    geojson_path: &str,
    perminute_csv: &str,
    trips_csv: &str,
    date: u32,
    out_path: &str,
    footprints_path: Option<&str>,
) -> Result<usize> {
    let proj = EnuProjection::default();
    let g_bytes = std::fs::read(graph_path).with_context(|| format!("reading {graph_path}"))?;
    let graph = StreetGraph::from_asset(GraphAsset::from_bytes(&g_bytes).context("decoding graph")?);
    // Optional dasymetric weight: per-node building mass (concentrated vs dispersed).
    let masses: Option<Vec<f64>> = footprints_path
        .map(|fp| node_building_mass(&graph, fp, &proj))
        .transpose()?;
    let zone_bytes = std::fs::read(geojson_path).with_context(|| format!("reading {geojson_path}"))?;
    let zones = parse_zones(&zone_bytes, &proj)?;
    let mut centroid: HashMap<i64, Vec2> = HashMap::new();
    for (loc, rings) in &zones {
        if let Some(c) = zone_centroid(rings) {
            centroid.insert(*loc, c);
        }
    }

    // 1. Read the day's trips: (pu_min, pu, do, dur_min, trip_miles, trip_time_s) +
    //    count O-D frequencies. The last two columns are optional (older CSVs lack
    //    them → 0 → fastest-route fallback per trip).
    let mut trips_raw: Vec<(f32, i64, i64, f32, f32, i32)> = Vec::new();
    let mut od_freq: HashMap<(i64, i64), u32> = HashMap::new();
    let mut rdr = csv::Reader::from_path(trips_csv).with_context(|| format!("opening {trips_csv}"))?;
    for rec in rdr.records() {
        let rec = rec?;
        let pu_min: f32 = rec.get(0).unwrap_or("").parse().unwrap_or(-1.0);
        let pu: i64 = rec.get(1).unwrap_or("").parse().unwrap_or(-1);
        let dolc: i64 = rec.get(2).unwrap_or("").parse().unwrap_or(-1);
        let dur: f32 = rec.get(3).unwrap_or("1").parse().unwrap_or(1.0);
        let miles: f32 = rec.get(4).unwrap_or("0").parse().unwrap_or(0.0);
        let time_s: i32 = rec.get(5).unwrap_or("0").parse().unwrap_or(0);
        if pu_min >= 0.0 && pu >= 0 && dolc >= 0 {
            trips_raw.push((pu_min, pu, dolc, dur.max(0.5), miles, time_s));
            *od_freq.entry((pu, dolc)).or_default() += 1;
        }
    }
    anyhow::ensure!(!trips_raw.is_empty(), "no taxi trips parsed");

    // 2. Per-zone endpoint sets: the drive-graph nodes inside each zone, reduced to
    //    K spatially-spread representatives. Sampling a trip's O/D among these (vs one
    //    centroid) fans routes out across the zone, spreading coverage off the spines.
    let n_nodes = graph.node_count();
    // Largest ring + bbox per zone for a point-in-zone prefilter.
    let mut zone_ring: HashMap<i64, &[[f64; 2]]> = HashMap::new();
    let mut zone_bbox: HashMap<i64, [f64; 4]> = HashMap::new();
    for (loc, rings) in &zones {
        let Some(ring) = rings.iter().max_by(|a, b| ring_area2(a).total_cmp(&ring_area2(b))) else {
            continue;
        };
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in ring {
            x0 = x0.min(p[0]);
            y0 = y0.min(p[1]);
            x1 = x1.max(p[0]);
            y1 = y1.max(p[1]);
        }
        zone_ring.insert(*loc, ring.as_slice());
        zone_bbox.insert(*loc, [x0, y0, x1, y1]);
    }
    // Assign each node to the (non-overlapping) zone whose largest ring contains it.
    let mut zone_node_list: HashMap<i64, Vec<u32>> = HashMap::new();
    for nid in 0..n_nodes as u32 {
        let p = graph.node_pos(nid);
        for (loc, bb) in &zone_bbox {
            if p.x < bb[0] || p.x > bb[2] || p.y < bb[1] || p.y > bb[3] {
                continue;
            }
            if point_in_ring(p, zone_ring[loc]) {
                zone_node_list.entry(*loc).or_default().push(nid);
                break;
            }
        }
    }
    // K endpoints per zone. With building mass (dasymetric), select by weighted
    // reservoir (Efraimidis–Spirakis: top-K by key = u^(1/w)) so a concentrated
    // zone picks clustered high-mass nodes and a dispersed one spreads; without
    // mass, sample evenly along the zone (uniform). Per-trip sampling then weights
    // by the same mass. Endpoints carry their weight for that.
    let mut endpoints: HashMap<i64, Vec<(u32, f64)>> = HashMap::new();
    for (loc, nodes) in &mut zone_node_list {
        let k = K_ENDPOINTS.min(nodes.len());
        if k == 0 {
            continue;
        }
        let eps: Vec<(u32, f64)> = match &masses {
            Some(m) => {
                let mut keyed: Vec<(f64, u32, f64)> = nodes
                    .iter()
                    .map(|&nid| {
                        let w = m[nid as usize].sqrt() + 1.0; // sqrt damps extreme contrast
                        let u = hash64(nid as u64, 7) as f64 / u64::MAX as f64;
                        (u.powf(1.0 / w), nid, w)
                    })
                    .collect();
                keyed.sort_by(|a, b| b.0.total_cmp(&a.0));
                keyed.into_iter().take(k).map(|(_, nid, w)| (nid, w)).collect()
            }
            None => {
                nodes.sort_by(|&a, &b| {
                    let (pa, pb) = (graph.node_pos(a), graph.node_pos(b));
                    (pa.x + pa.y).total_cmp(&(pb.x + pb.y))
                });
                (0..k).map(|j| (nodes[j * nodes.len() / k], 1.0)).collect()
            }
        };
        endpoints.insert(*loc, eps);
    }
    // Centroid fallback node per zone.
    let mut centroid_node: HashMap<i64, u32> = HashMap::new();
    for (loc, c) in &centroid {
        if let Some(n) = graph.snap_nearest(*c) {
            centroid_node.insert(*loc, n);
        }
    }

    // 3. Route pool. Phase A: one fastest centroid route per top O-D pair (breadth
    //    + a per-trip fallback). Phase B (trip assignment) builds per-combo
    //    candidate sets and infers each trip's route from its time + distance.
    let mut pairs: Vec<((i64, i64), u32)> = od_freq.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1));
    let mut routes: Vec<VehicleRoute> = Vec::new();
    let mut route_edges: Vec<Vec<u32>> = Vec::new(); // edges per route, for sensing power
    let mut pair_centroid: HashMap<(i64, i64), u32> = HashMap::new();
    let (mut tried, mut no_path) = (0usize, 0usize);
    for ((pu, dolc), _freq) in pairs.iter().take(MAX_OD_PAIRS) {
        let (Some(&from), Some(&to)) = (centroid_node.get(pu), centroid_node.get(dolc)) else {
            continue;
        };
        tried += 1;
        let Some((vr, edges)) = route_vr(&graph, from, to) else {
            no_path += 1;
            continue;
        };
        let idx = routes.len() as u32;
        routes.push(vr);
        route_edges.push(edges);
        pair_centroid.insert((*pu, *dolc), idx);
    }
    anyhow::ensure!(!routes.is_empty(), "no taxi routes baked");

    // Calibrate the slowdown prior g(s): s0 = the typical real slowdown relative to
    // the 25 mph surface free-flow time implied by each trip's own distance. Grounds
    // the time likelihood in the data rather than a guessed constant. Restricted to
    // **long trips (> 2 mi)** — the only regime with a real highway alternative;
    // short trips are stop-start-dominated and would bias s0 far too high, pushing
    // the highway-vs-surface boundary out of the relevant range.
    let mut logs: Vec<f64> = Vec::new();
    for (_, _, _, _, miles, time_s) in &trips_raw {
        let (d, t) = (*miles as f64 * 1609.344, *time_s as f64);
        if d > 3200.0 && t > 120.0 {
            logs.push((t / (d / SURF_SPEED_MPS)).clamp(0.3, 8.0).ln());
        }
    }
    let (ln_s0, tau) = if logs.len() > 100 {
        let n = logs.len() as f64;
        let mean = logs.iter().sum::<f64>() / n;
        let var = logs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
        (mean, var.sqrt().clamp(0.35, 0.8))
    } else {
        ((1.8f64).ln(), 0.55) // fallback prior when the CSV lacks miles/time
    };
    eprintln!(
        "  congestion prior: s0={:.2}× typical slowdown, tau={:.2} (from {} trips)",
        ln_s0.exp(),
        tau,
        logs.len()
    );

    // 4. Assign each trip → a route. Sample within-zone endpoints, lazily build the
    //    combo's candidate set (fastest + a surface alternative, globally capped),
    //    and pick the MAP candidate under the trip's observed distance + time.
    struct Cand {
        route_idx: u32,
        length_m: f64,
        time_s: f64,
    }
    let mut combo_cands: HashMap<(u32, u32), Vec<Cand>> = HashMap::new();
    let mut pair_combos: HashMap<(i64, i64), usize> = HashMap::new();
    let mut alt_routes: HashSet<u32> = HashSet::new(); // non-fastest candidates (diag)
    // Validation: mean observed speed of trips that chose the fastest (usually
    // highway) vs the surface candidate, among 2-candidate combos. Inference is
    // working iff highway-choosers are the faster trips.
    let (mut hw_n, mut hw_mph, mut surf_n, mut surf_mph) = (0usize, 0.0f64, 0usize, 0.0f64);
    let mut trips: Vec<TaxiTrip> = Vec::new();
    for (i, (pu_min, pu, dolc, dur, miles, time_s)) in trips_raw.into_iter().enumerate() {
        let Some(&centroid_ri) = pair_centroid.get(&(pu, dolc)) else {
            continue; // pair beyond MAX_OD_PAIRS / no path — drop (as before)
        };
        let (d_m, t_obs) = (miles as f64 * 1609.344, time_s as f64);
        let ri = match (endpoints.get(&pu), endpoints.get(&dolc)) {
            (Some(ep), Some(ed)) if !ep.is_empty() && !ed.is_empty() => {
                let a = weighted_pick(ep, i as u64 * 2);
                let b = weighted_pick(ed, i as u64 * 2 + 1);
                // Build this combo's candidate set once (fastest + surface), if budget allows.
                if !combo_cands.contains_key(&(a, b)) {
                    let used = pair_combos.entry((pu, dolc)).or_default();
                    if routes.len() + 2 <= MAX_ROUTES && *used < MAX_COMBOS_PER_PAIR {
                        let mut cands: Vec<Cand> = Vec::new();
                        for pen in [1.0, HW_PENALTY] {
                            let Ok((r, t_r, edges)) = graph.route_timed_pen(a, b, pen) else {
                                continue;
                            };
                            // Dedup near-identical candidates (same length ⇒ same path).
                            if cands.iter().any(|c| (c.length_m - r.total_m).abs() < 30.0) {
                                continue;
                            }
                            if let Some(vr) = route_to_vr(&r) {
                                let idx = routes.len() as u32;
                                if !cands.is_empty() {
                                    alt_routes.insert(idx);
                                }
                                routes.push(vr);
                                route_edges.push(edges);
                                cands.push(Cand { route_idx: idx, length_m: r.total_m, time_s: t_r });
                            }
                        }
                        if !cands.is_empty() {
                            *used += 1;
                            combo_cands.insert((a, b), cands);
                        }
                    }
                }
                // Infer: MAP candidate under (distance, time); else fastest; else centroid.
                match combo_cands.get(&(a, b)) {
                    Some(cands) if !cands.is_empty() => {
                        if d_m > 0.0 && t_obs > 0.0 {
                            let best = cands.iter().max_by(|x, y| {
                                infer_score(x.length_m, x.time_s, d_m, t_obs, ln_s0, tau).total_cmp(
                                    &infer_score(y.length_m, y.time_s, d_m, t_obs, ln_s0, tau),
                                )
                            });
                            match best {
                                Some(c) => {
                                    if cands.len() >= 2 {
                                        let mph = d_m / t_obs * 2.2369;
                                        if c.route_idx == cands[0].route_idx {
                                            hw_n += 1;
                                            hw_mph += mph;
                                        } else {
                                            surf_n += 1;
                                            surf_mph += mph;
                                        }
                                    }
                                    c.route_idx
                                }
                                None => centroid_ri,
                            }
                        } else {
                            cands[0].route_idx // no inference data → fastest
                        }
                    }
                    _ => centroid_ri,
                }
            }
            _ => centroid_ri,
        };
        trips.push(TaxiTrip { pu_min, route_idx: ri, dur_min: dur });
    }
    let chose_alt = trips.iter().filter(|t| alt_routes.contains(&t.route_idx)).count();
    eprintln!(
        "  route inference: {} surface-alt routes; on 2-candidate trips, fastest/highway picks \
         avg {:.1} mph ({hw_n}) vs surface picks {:.1} mph ({surf_n}) — {chose_alt} trips non-fastest",
        alt_routes.len(),
        if hw_n > 0 { hw_mph / hw_n as f64 } else { 0.0 },
        if surf_n > 0 { surf_mph / surf_n as f64 } else { 0.0 },
    );

    // Stage 4 — analytic sensing power (O'Keeffe et al., PNAS 2019). Per segment i,
    // q_i = probability a single random trip covers it = (trips traversing i)/M.
    // Expected coverage by N independent random trips: C(N) = (1/S)·Σ_i[1−(1−q_i)^N].
    // The heavy tail of q_i (a few avenues carry most trips) makes coverage saturate
    // fast — the paper's "~10 random taxis sense ⅓ of Manhattan's streets per day".
    let mut route_trips = vec![0u64; routes.len()];
    for t in &trips {
        route_trips[t.route_idx as usize] += 1;
    }
    let mut edge_trips = vec![0u64; graph.edge_count()];
    for (ri, edges) in route_edges.iter().enumerate() {
        let w = route_trips[ri];
        if w == 0 {
            continue;
        }
        for &e in edges {
            edge_trips[e as usize] += w;
        }
    }
    let (m, s_total) = (trips.len() as f64, graph.edge_count() as f64);
    let cov = |n: f64| -> f64 {
        edge_trips
            .iter()
            .filter(|&&c| c > 0)
            .map(|&c| 1.0 - (1.0 - c as f64 / m).powf(n))
            .sum::<f64>()
            / s_total
    };
    let n_for = |target: f64| -> f64 {
        let (mut lo, mut hi) = (1.0f64, 2.0f64);
        while cov(hi) < target && hi < 1e7 {
            hi *= 2.0;
        }
        for _ in 0..40 {
            let mid = (lo + hi) / 2.0;
            if cov(mid) < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        (lo + hi) / 2.0
    };
    let covered = edge_trips.iter().filter(|&&c| c > 0).count();
    eprintln!(
        "  sensing power (O'Keeffe) over {} drive segments (full fleet of {} trips senses \
         {covered} = {:.0}% — the asymptotic ceiling):",
        graph.edge_count(),
        trips.len(),
        100.0 * covered as f64 / s_total
    );
    for n in [1u32, 5, 10, 30, 100, 300, 1000, 3000] {
        eprintln!("    N={n:>4} random trips → {:.1}% of streets sensed/day", cov(n as f64) * 100.0);
    }
    let (n13, n12) = (n_for(1.0 / 3.0), n_for(0.5));
    eprintln!(
        "    ⅓ at ~{n13:.0} trips (~{:.0} FHV-days @27 trips/day); ½ at ~{n12:.0} trips \
         [O'Keeffe anchor: ~10 random taxis ≈ ⅓]",
        n13 / 27.0
    );

    trips.sort_by(|a, b| a.pu_min.total_cmp(&b.pu_min));

    // 5. Full per-minute O-D aggregate (for the analytical flux).
    let mut od_per_minute: Vec<TaxiOdMinute> = Vec::new();
    let mut rdr =
        csv::Reader::from_path(perminute_csv).with_context(|| format!("opening {perminute_csv}"))?;
    for rec in rdr.records() {
        let rec = rec?;
        let pu_min: u16 = rec.get(0).unwrap_or("0").parse().unwrap_or(0);
        let pu: u16 = rec.get(1).unwrap_or("0").parse().unwrap_or(0);
        let dolc: u16 = rec.get(2).unwrap_or("0").parse().unwrap_or(0);
        let t: u32 = rec.get(3).unwrap_or("0").parse().unwrap_or(0);
        if t > 0 {
            od_per_minute.push(TaxiOdMinute { pu_min, pu_zone: pu, do_zone: dolc, trips: t });
        }
    }

    let layer = TaxiDayLayer {
        origin: GeoOrigin::MANHATTAN,
        service_date: date,
        // Compress the route polylines for the wire/asset (delta-quantized; see `TaxiRoute`).
        routes: routes.into_iter().map(TaxiRoute::from).collect(),
        trips,
        od_per_minute,
        provenance: Provenance {
            source: "NYC TLC High-Volume FHV (Uber/Lyft) trip records, one service day".into(),
            url: "https://www.nyc.gov/site/tlc/about/tlc-trip-record-data.page".into(),
            license: "NYC OpenData / TLC terms".into(),
            as_of: format!("TLC HVFHV {date}"),
            notes: "Real per-trip replay: pickup minute + real duration. Endpoints sampled \
                    within each O/D zone (dasymetric, building-mass-weighted), then the route \
                    is *inferred* from the trip's own distance + time — a fastest vs surface \
                    candidate scored by a distance likelihood × a data-calibrated slowdown \
                    prior, so trips too fast for surface streets are attributed to the \
                    highways (FDR / West Side Hwy)."
                .into(),
        },
        sensing: SensingPower {
            segments_total: graph.edge_count() as u32,
            segments_sensed: covered as u32,
            trips_total: m as u32,
            n_third: n13.round() as u32,
            n_half: n12.round() as u32,
            trips_per_vehicle_day: 27,
        },
    };
    let (nt, nr, no) = (layer.trips.len(), layer.routes.len(), layer.od_per_minute.len());
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!(
        "taxi day {date}: {nt} trips, {nr} routes ({no_path}/{tried} no-path), {no} od-minute rows -> {out_path}"
    );
    Ok(nt)
}
