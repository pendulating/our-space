//! Headway-based subway router, baked from GTFS static.
//!
//! The exposure instrument models a *representative* AM-peak commute, not a clock-timed
//! departure, so the right transit model is an expectation over the service frequency,
//! not a timetable query. That choice is what keeps this module small and the OD bake
//! fast: the whole network reduces to an all-pairs station matrix computed once at bake
//! time, and the 5.9M-pair OD bake pays O(1) per lookup.
//!
//! Graph model (per parsed weekday-AM-peak feed): station nodes only, joined by
//! **direct-connection bundle edges** — for every ordered station pair (a, b) served
//! without a transfer, the rider waits once and boards the first arrival among the
//! *attractive* set of lines (the classic common-lines solution, Chriqui & Robillard
//! 1975): lines sorted by run time join the bundle while their ride beats the bundle's
//! expected total,
//!   E[T] = (alpha + sum f_l t_l) / (sum f_l),   alpha = 0.5,
//! giving expected wait alpha/F over the POOLED frequency F. A rider at a 1/2/3 trunk
//! station is credited the combined headway, not one line's — at the first boarding and
//! at every transfer. Local-only riders never see an express bundle: a line enters a
//! pair's bundle only if its trips actually serve a then b. One bundle edge = one
//! boarding = one train ridden (the camera complement stays integral); expected
//! line-haul is the frequency-weighted mean over the bundle's lines.
//!
//! Edge costs: board overhead (fare gate + platform access; the feed's same-station
//! `min_transfer_time` when present, else a default) + capped expected wait + expected
//! ride + alight dwell. Cross-parent transfer walks come from `transfers.txt`.
//!
//! Remaining approximation, disclosed where the paper uses this: the bundle commits to
//! one alighting station (a restriction of full hyperpath strategies, where a rider
//! might alight where the boarded line diverges), and pooling is parent-station level
//! (NYC platforms are mostly shared per direction, but a bundle across separate
//! platforms slightly overstates pooling).

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

use crate::assets::Provenance;
use crate::projection::GeoOrigin;

/// Expected-wait factor over the pooled frequency: E[wait] = ALPHA / F. 0.5 is the
/// standard frequency-based-assignment value (uniform arrivals over the headway).
const WAIT_ALPHA: f64 = 0.5;

/// One GTFS parent station (`location_type=1`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubwayStation {
    pub id: String,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

/// A service-day trip, normalized to parent-station indices with absolute departure
/// seconds (GTFS clock: may exceed 24h). Consecutive duplicate stations pre-deduped.
#[derive(Debug, Clone)]
pub struct FeedTrip {
    pub route: String,
    pub direction: u8,
    /// (station index, departure seconds since service-day midnight), stop order.
    pub stops: Vec<(u32, f64)>,
}

/// Parsed + normalized GTFS input for [`build_subway_matrix`]. The data-pipeline crate
/// does the file parsing; this type keeps the math testable with synthetic feeds.
#[derive(Debug, Default)]
pub struct SubwayFeed {
    pub stations: Vec<SubwayStation>,
    pub trips: Vec<FeedTrip>,
    /// (from, to, seconds). Same-station rows become that station's boarding overhead
    /// (`min_transfer_time - alight_s`, floored at the default); distinct-station rows
    /// become walk edges (symmetrized).
    pub transfers: Vec<(u32, u32, f64)>,
}

/// Build knobs. Defaults model a weekday AM peak (07:00-10:00).
#[derive(Debug, Clone, Copy)]
pub struct SubwayBuildParams {
    /// Service window, seconds since service-day midnight. Departures outside it don't
    /// count toward frequencies (run times still inform the medians).
    pub window_start_s: f64,
    pub window_end_s: f64,
    /// Street/mezzanine -> platform access charged on every boarding, when the feed has
    /// no same-station transfer row for the station.
    pub board_overhead_s: f64,
    /// Platform egress charged on every alighting.
    pub alight_s: f64,
    /// Cap on the expected wait: rare in-window services (rush-hour specials) are
    /// planned around, not waited for at random.
    pub max_wait_s: f64,
}

impl Default for SubwayBuildParams {
    fn default() -> Self {
        SubwayBuildParams {
            window_start_s: 7.0 * 3600.0,
            window_end_s: 10.0 * 3600.0,
            board_overhead_s: 60.0,
            alight_s: 15.0,
            max_wait_s: 900.0,
        }
    }
}

/// One all-pairs itinerary summary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Itinerary {
    /// Door(station)-to-door(station) seconds: first wait + overheads + rides + transfers.
    pub time_s: f64,
    /// Trains ridden (>= 1 for any real ride; transfers = boardings - 1).
    pub boardings: u32,
    /// Expected ridden distance, meters (frequency-weighted over each bundle).
    pub line_haul_m: f64,
}

/// The baked asset: stations + dense all-pairs (time, boardings, line-haul).
/// ~500 stations -> ~2.5 MB. Unreachable pairs store `f32::INFINITY`.
#[derive(Serialize, Deserialize)]
pub struct SubwayMatrix {
    pub origin: GeoOrigin,
    pub stations: Vec<SubwayStation>,
    pub time_s: Vec<f32>,
    pub boardings: Vec<u8>,
    pub line_haul_m: Vec<f32>,
    pub provenance: Provenance,
}

impl SubwayMatrix {
    pub fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(b)
    }
    pub fn len(&self) -> usize {
        self.stations.len()
    }
    pub fn is_empty(&self) -> bool {
        self.stations.is_empty()
    }
    /// `None` when the pair is unreachable (or out of range).
    pub fn itinerary(&self, from: usize, to: usize) -> Option<Itinerary> {
        let n = self.stations.len();
        if from >= n || to >= n {
            return None;
        }
        let t = self.time_s[from * n + to];
        if !t.is_finite() {
            return None;
        }
        Some(Itinerary {
            time_s: t as f64,
            boardings: self.boardings[from * n + to] as u32,
            line_haul_m: self.line_haul_m[from * n + to] as f64,
        })
    }
}

/// Great-circle meters between two stations (haversine; exact enough for chord hops).
fn chord_m(a: &SubwayStation, b: &SubwayStation) -> f64 {
    const R: f64 = 6_371_008.8;
    let (la, lb) = (a.lat.to_radians(), b.lat.to_radians());
    let dlat = lb - la;
    let dlon = (b.lon - a.lon).to_radians();
    let h = (dlat / 2.0).sin().powi(2) + la.cos() * lb.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * R * h.sqrt().min(1.0).asin()
}

/// Per-line service between one ordered station pair.
struct PairLine {
    n_dep: u32,       // in-window departures at the origin that reach the destination
    runs: Vec<f64>,   // observed run times (median taken at bundle time)
    haul_sum: f64,    // ridden chord distance summed over observations
    haul_n: u32,
}

#[derive(Clone, Copy)]
struct Edge {
    to: u32,
    cost: f64,
    board: bool,
    haul_m: f64,
}

/// Build the all-pairs matrix from a normalized feed. Deterministic: pair keys and edge
/// order derive from sorted keys, so identical feeds bake byte-identical assets.
pub fn build_subway_matrix(
    feed: &SubwayFeed,
    p: &SubwayBuildParams,
    origin: GeoOrigin,
    provenance: Provenance,
) -> SubwayMatrix {
    let n_st = feed.stations.len();
    let window_len = (p.window_end_s - p.window_start_s).max(1.0);

    // Intern (route, direction) as the line identity, first-seen order (stable).
    let mut line_ix: HashMap<(&str, u8), u32> = HashMap::new();
    let mut n_lines = 0u32;
    for t in &feed.trips {
        line_ix.entry((t.route.as_str(), t.direction)).or_insert_with(|| {
            let i = n_lines;
            n_lines += 1;
            i
        });
    }

    // Direct-connection observations: for every trip, every ordered stop pair (i < j)
    // is a one-boarding connection on that line. Cumulative chords give the ridden
    // distance a->b along the trip's own stop pattern (an express's A->D is shorter
    // than the local's A->B->C->D).
    let mut pairs: BTreeMap<(u32, u32, u32), PairLine> = BTreeMap::new(); // (a, b, line)
    for t in &feed.trips {
        let line = line_ix[&(t.route.as_str(), t.direction)];
        let mut cum = Vec::with_capacity(t.stops.len());
        let mut acc = 0.0;
        cum.push(0.0);
        for w in t.stops.windows(2) {
            acc += chord_m(&feed.stations[w[0].0 as usize], &feed.stations[w[1].0 as usize]);
            cum.push(acc);
        }
        for i in 0..t.stops.len() {
            let (a, ta) = t.stops[i];
            let in_window = ta >= p.window_start_s && ta < p.window_end_s;
            for j in (i + 1)..t.stops.len() {
                let (b, tb) = t.stops[j];
                if tb <= ta {
                    continue;
                }
                let e = pairs.entry((a, b, line)).or_insert(PairLine {
                    n_dep: 0,
                    runs: Vec::new(),
                    haul_sum: 0.0,
                    haul_n: 0,
                });
                e.runs.push(tb - ta);
                e.haul_sum += cum[j] - cum[i];
                e.haul_n += 1;
                if in_window {
                    e.n_dep += 1;
                }
            }
        }
    }

    // Same-station transfer rows -> per-station boarding overhead.
    let mut overhead: Vec<f64> = vec![p.board_overhead_s; n_st];
    for &(a, b, secs) in &feed.transfers {
        if a == b && (a as usize) < n_st {
            overhead[a as usize] = (secs - p.alight_s).max(p.board_overhead_s);
        }
    }

    // Fold each pair's lines into its attractive bundle (Chriqui-Robillard greedy):
    // sort candidate lines by run time; a line joins while its ride beats the bundle's
    // expected total E = (alpha + sum f t) / (sum f).
    let mut adj: Vec<Vec<Edge>> = vec![Vec::new(); n_st];
    let mut bundle: Vec<(f64, f64, f64)> = Vec::new(); // (f_l, t_l, haul_l) candidates
    let mut cur: Option<(u32, u32)> = None;
    let mut flush = |key: Option<(u32, u32)>, cand: &mut Vec<(f64, f64, f64)>,
                     adj: &mut Vec<Vec<Edge>>| {
        let Some((a, b)) = key else { return };
        if cand.is_empty() {
            return;
        }
        cand.sort_by(|x, y| x.1.total_cmp(&y.1));
        let (mut f_sum, mut ft_sum, mut fh_sum) = (0.0f64, 0.0f64, 0.0f64);
        for &(f, t, h) in cand.iter() {
            if f_sum > 0.0 && t >= (WAIT_ALPHA + ft_sum) / f_sum {
                break; // slower lines would raise the bundle's expected total
            }
            f_sum += f;
            ft_sum += f * t;
            fh_sum += f * h;
        }
        cand.clear();
        let wait = (WAIT_ALPHA / f_sum).min(p.max_wait_s);
        let ride = ft_sum / f_sum;
        adj[a as usize].push(Edge {
            to: b,
            cost: overhead[a as usize] + wait + ride + p.alight_s,
            board: true,
            haul_m: fh_sum / f_sum,
        });
    };
    for (&(a, b, _line), pl) in pairs.iter_mut() {
        if pl.n_dep == 0 {
            continue; // outside the window: no boarding opportunity
        }
        if cur != Some((a, b)) {
            flush(cur, &mut bundle, &mut adj);
            cur = Some((a, b));
        }
        pl.runs.sort_by(f64::total_cmp);
        let t_med = pl.runs[pl.runs.len() / 2];
        let f = pl.n_dep as f64 / window_len; // departures per second
        bundle.push((f, t_med, pl.haul_sum / pl.haul_n as f64));
    }
    flush(cur, &mut bundle, &mut adj);

    // Cross-station transfer walks, symmetrized.
    let mut walks: BTreeMap<(u32, u32), f64> = BTreeMap::new();
    for &(a, b, secs) in &feed.transfers {
        if a != b && (a as usize) < n_st && (b as usize) < n_st {
            let e = walks.entry((a, b)).or_insert(secs);
            *e = e.min(secs);
            let e = walks.entry((b, a)).or_insert(secs);
            *e = e.min(secs);
        }
    }
    for (&(a, b), &secs) in &walks {
        adj[a as usize].push(Edge { to: b, cost: secs, board: false, haul_m: 0.0 });
    }
    for edges in adj.iter_mut() {
        edges.sort_by(|x, y| (x.to, x.cost.to_bits()).cmp(&(y.to, y.cost.to_bits())));
    }

    // All-pairs Dijkstra from every station node, carrying (boardings, line-haul).
    let mut time_s = vec![f32::INFINITY; n_st * n_st];
    let mut boardings = vec![0u8; n_st * n_st];
    let mut line_haul_m = vec![0f32; n_st * n_st];
    let mut dist = vec![f64::INFINITY; n_st];
    let mut brd = vec![0u32; n_st];
    let mut haul = vec![0f64; n_st];
    for src in 0..n_st {
        dist.fill(f64::INFINITY);
        brd.fill(0);
        haul.fill(0.0);
        dist[src] = 0.0;
        let mut heap: std::collections::BinaryHeap<QItem> = std::collections::BinaryHeap::new();
        heap.push(QItem { cost: 0.0, node: src as u32 });
        while let Some(QItem { cost, node }) = heap.pop() {
            if cost > dist[node as usize] {
                continue;
            }
            for e in &adj[node as usize] {
                let nd = cost + e.cost;
                if nd < dist[e.to as usize] {
                    dist[e.to as usize] = nd;
                    brd[e.to as usize] = brd[node as usize] + e.board as u32;
                    haul[e.to as usize] = haul[node as usize] + e.haul_m;
                    heap.push(QItem { cost: nd, node: e.to });
                }
            }
        }
        for dst in 0..n_st {
            if dist[dst].is_finite() {
                time_s[src * n_st + dst] = dist[dst] as f32;
                boardings[src * n_st + dst] = brd[dst].min(u8::MAX as u32) as u8;
                line_haul_m[src * n_st + dst] = haul[dst] as f32;
            }
        }
    }

    SubwayMatrix {
        origin,
        stations: feed.stations.clone(),
        time_s,
        boardings,
        line_haul_m,
        provenance,
    }
}

/// Min-heap item ordered by cost (ties by node id for determinism).
#[derive(PartialEq)]
struct QItem {
    cost: f64,
    node: u32,
}
impl Eq for QItem {}
impl Ord for QItem {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other
            .cost
            .total_cmp(&self.cost)
            .then_with(|| other.node.cmp(&self.node))
    }
}
impl PartialOrd for QItem {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(id: &str, lat: f64, lon: f64) -> SubwayStation {
        SubwayStation { id: id.into(), name: id.into(), lat, lon }
    }
    fn prov() -> Provenance {
        Provenance {
            source: "test".into(),
            url: String::new(),
            license: String::new(),
            as_of: String::new(),
            notes: String::new(),
        }
    }
    /// `k` identical trips on `route`/`dir` over `stops`, departures spread over the
    /// window start..start+3h, legs `run_s` apart.
    fn trips(route: &str, dir: u8, stops: &[u32], k: usize, run_s: f64) -> Vec<FeedTrip> {
        let window = 3.0 * 3600.0;
        (0..k)
            .map(|i| FeedTrip {
                route: route.into(),
                direction: dir,
                stops: stops
                    .iter()
                    .enumerate()
                    .map(|(j, &s)| {
                        (s, 7.0 * 3600.0 + i as f64 * window / k as f64 + j as f64 * run_s)
                    })
                    .collect(),
            })
            .collect()
    }
    fn params() -> SubwayBuildParams {
        SubwayBuildParams::default() // window 07:00-10:00, overhead 60, alight 15, cap 900
    }
    fn build(feed: &SubwayFeed) -> SubwayMatrix {
        build_subway_matrix(feed, &params(), GeoOrigin::MANHATTAN, prov())
    }

    #[test]
    fn single_ride_time_wait_and_boardings() {
        // 6 departures over 3 h -> pooled F = one line -> wait alpha/F = 900 (at the cap).
        let feed = SubwayFeed {
            stations: vec![st("A", 40.70, -74.00), st("B", 40.71, -74.00)],
            trips: trips("X", 0, &[0, 1], 6, 240.0),
            transfers: vec![],
        };
        let m = build(&feed);
        let it = m.itinerary(0, 1).expect("A->B routable");
        // wait 900 + overhead 60 + run 240 + alight 15
        assert!((it.time_s - 1215.0).abs() < 1e-6, "time {}", it.time_s);
        assert_eq!(it.boardings, 1);
        // 0.01 deg latitude ~= 1112 m.
        assert!((it.line_haul_m - 1111.95).abs() < 1.0, "haul {}", it.line_haul_m);
        // Terminals don't board: B->A has no reverse service.
        assert!(m.itinerary(1, 0).is_none());
        assert_eq!(m.itinerary(0, 0).unwrap().time_s, 0.0);
    }

    #[test]
    fn express_bypasses_local_but_only_where_it_stops() {
        // Local L: A-B-C-D at 300 s/leg; express E: A-D direct in 400 s. Both frequent.
        let stations =
            vec![st("A", 40.70, -74.00), st("B", 40.71, -74.00), st("C", 40.72, -74.00), st("D", 40.73, -74.00)];
        let mut all = trips("L", 0, &[0, 1, 2, 3], 18, 300.0);
        all.extend(trips("E", 0, &[0, 3], 18, 400.0));
        let feed = SubwayFeed { stations, trips: all, transfers: vec![] };
        let m = build(&feed);
        // From A: the local (900 s ride) is NOT attractive next to the express
        // (E = 300 + 400 = 700 < 900), so the bundle is the express alone: 60+300+400+15.
        let ad = m.itinerary(0, 3).unwrap();
        assert!((ad.time_s - 775.0).abs() < 1e-6, "time {}", ad.time_s);
        assert_eq!(ad.boardings, 1);
        // From B the express is invisible; the local's 600 s ride stands.
        let bd = m.itinerary(1, 3).unwrap();
        assert!((bd.time_s - 975.0).abs() < 1e-6, "time {}", bd.time_s);
    }

    #[test]
    fn transfer_counts_two_boardings_and_both_waits() {
        // X: A->B, Y: B->C, both 18 deps (wait 300). Same-station transfer at B costs
        // alight 15 + overhead 60 + wait 300.
        let stations = vec![st("A", 40.70, -74.00), st("B", 40.71, -74.00), st("C", 40.71, -73.99)];
        let mut all = trips("X", 0, &[0, 1], 18, 300.0);
        all.extend(trips("Y", 0, &[1, 2], 18, 200.0));
        let feed = SubwayFeed { stations, trips: all, transfers: vec![] };
        let m = build(&feed);
        let it = m.itinerary(0, 2).unwrap();
        // 60+300+300+15 (X) + 60+300+200+15 (Y at B)
        assert!((it.time_s - 1250.0).abs() < 1e-6, "time {}", it.time_s);
        assert_eq!(it.boardings, 2);
    }

    #[test]
    fn cross_station_transfer_walk_edge() {
        // X: A->B1; Y: B2->C; B1<->B2 walk 120 s (distinct parents, e.g. Times Sq complex).
        let stations = vec![
            st("A", 40.70, -74.00),
            st("B1", 40.71, -74.00),
            st("B2", 40.7105, -74.0005),
            st("C", 40.72, -74.00),
        ];
        let mut all = trips("X", 0, &[0, 1], 18, 300.0);
        all.extend(trips("Y", 0, &[2, 3], 18, 200.0));
        let feed = SubwayFeed { stations, trips: all, transfers: vec![(1, 2, 120.0)] };
        let m = build(&feed);
        let it = m.itinerary(0, 3).unwrap();
        // X leg 675 + walk 120 + board at B2 (60+300) + ride 200 + alight 15
        assert!((it.time_s - 1370.0).abs() < 1e-6, "time {}", it.time_s);
        assert_eq!(it.boardings, 2);
        // Symmetrized even though the feed listed one direction only.
        assert!(m.itinerary(3, 0).is_none()); // still no reverse *service*, though
    }

    #[test]
    fn same_station_transfer_row_sets_boarding_overhead() {
        let stations = vec![st("A", 40.70, -74.00), st("B", 40.71, -74.00)];
        let feed = SubwayFeed {
            stations,
            trips: trips("X", 0, &[0, 1], 18, 240.0),
            transfers: vec![(0, 0, 300.0)], // MTA-style self row: 300 s to reach a platform at A
        };
        let m = build(&feed);
        let it = m.itinerary(0, 1).unwrap();
        // wait 300 + overhead (300-15) + run 240 + alight 15
        assert!((it.time_s - 840.0).abs() < 1e-6, "time {}", it.time_s);
    }

    #[test]
    fn rare_service_wait_is_capped() {
        // One departure in the window: uncapped wait would be 5400 s.
        let feed = SubwayFeed {
            stations: vec![st("A", 40.70, -74.00), st("B", 40.71, -74.00)],
            trips: trips("X", 0, &[0, 1], 1, 240.0),
            transfers: vec![],
        };
        let m = build(&feed);
        let it = m.itinerary(0, 1).unwrap();
        assert!((it.time_s - (900.0 + 60.0 + 240.0 + 15.0)).abs() < 1e-6);
    }

    #[test]
    fn out_of_window_departures_do_not_board() {
        // Departures at 05:00: legs exist (run medians) but no in-window boarding.
        let feed = SubwayFeed {
            stations: vec![st("A", 40.70, -74.00), st("B", 40.71, -74.00)],
            trips: vec![FeedTrip {
                route: "X".into(),
                direction: 0,
                stops: vec![(0, 5.0 * 3600.0), (1, 5.0 * 3600.0 + 240.0)],
            }],
            transfers: vec![],
        };
        let m = build(&feed);
        assert!(m.itinerary(0, 1).is_none());
    }

    #[test]
    fn matrix_roundtrips_through_postcard() {
        let feed = SubwayFeed {
            stations: vec![st("A", 40.70, -74.00), st("B", 40.71, -74.00)],
            trips: trips("X", 0, &[0, 1], 6, 240.0),
            transfers: vec![],
        };
        let m = build(&feed);
        let m2 = SubwayMatrix::from_bytes(&m.to_bytes().unwrap()).unwrap();
        assert_eq!(m2.stations.len(), 2);
        assert_eq!(m2.itinerary(0, 1), m.itinerary(0, 1));
    }

    // ---- common-lines pooling (new with the bundle model) --------------------------

    #[test]
    fn parallel_lines_pool_the_wait() {
        // Two identical lines, 18 deps each (per-line wait 300): pooled wait is 150.
        let stations = vec![st("A", 40.70, -74.00), st("B", 40.71, -74.00)];
        let mut all = trips("X", 0, &[0, 1], 18, 240.0);
        all.extend(trips("Y", 0, &[0, 1], 18, 240.0));
        let feed = SubwayFeed { stations, trips: all, transfers: vec![] };
        let m = build(&feed);
        let it = m.itinerary(0, 1).unwrap();
        // overhead 60 + pooled wait 150 + ride 240 + alight 15
        assert!((it.time_s - 465.0).abs() < 1e-6, "time {}", it.time_s);
        assert_eq!(it.boardings, 1);
    }

    #[test]
    fn unattractive_slow_line_is_excluded_marginal_line_joins() {
        let stations = vec![st("A", 40.70, -74.00), st("B", 40.71, -74.00)];
        // Fast line X: t=240 (E alone = 300 + 240 = 540).
        // Slow line Z: t=2000 > 540 -> excluded; time is X's alone.
        let mut all = trips("X", 0, &[0, 1], 18, 240.0);
        all.extend(trips("Z", 0, &[0, 1], 18, 2000.0));
        let m = build(&SubwayFeed { stations: stations.clone(), trips: all, transfers: vec![] });
        let it = m.itinerary(0, 1).unwrap();
        assert!((it.time_s - (60.0 + 300.0 + 240.0 + 15.0)).abs() < 1e-6, "time {}", it.time_s);

        // Marginal line Y: t=500 < 540 -> joins; E = (0.5 + f*240 + f*500)/(2f) = 520
        // (wait 150 + weighted ride 370), an improvement over 540.
        let mut all = trips("X", 0, &[0, 1], 18, 240.0);
        all.extend(trips("Y", 0, &[0, 1], 18, 500.0));
        let m = build(&SubwayFeed { stations, trips: all, transfers: vec![] });
        let it = m.itinerary(0, 1).unwrap();
        assert!((it.time_s - (60.0 + 150.0 + 370.0 + 15.0)).abs() < 1e-6, "time {}", it.time_s);
        assert_eq!(it.boardings, 1);
    }

    #[test]
    fn bundle_line_haul_is_frequency_weighted() {
        // Express A->C direct vs local A->B->C (longer ridden path via the dogleg at B).
        // Equal frequencies and close run times -> both attractive -> haul is the mean.
        let stations =
            vec![st("A", 40.70, -74.00), st("B", 40.705, -73.99), st("C", 40.71, -74.00)];
        let mut all = trips("E", 0, &[0, 2], 18, 300.0);
        all.extend(trips("L", 0, &[0, 1, 2], 18, 160.0)); // 320 s total, joins the bundle
        let feed = SubwayFeed { stations: stations.clone(), trips: all, transfers: vec![] };
        let m = build(&feed);
        let it = m.itinerary(0, 2).unwrap();
        let direct = chord_m(&stations[0], &stations[2]);
        let dogleg = chord_m(&stations[0], &stations[1]) + chord_m(&stations[1], &stations[2]);
        let want = (direct + dogleg) / 2.0;
        assert!((it.line_haul_m - want).abs() < 1.0, "haul {} want {want}", it.line_haul_m);
        assert_eq!(it.boardings, 1);
    }
}
