//! Bake a single real day of NYC TLC rideshare (HVFHV) trips for schedule-driven
//! replay: a shared routed-O-D pool, the day's real trips (pickup minute + O-D +
//! real duration), and the full per-minute O-D aggregate (for the analytical flux).
//!
//! Inputs (fetched once via DuckDB — see README/fetch-taxi-day): the baked walk
//! graph, the taxi-zone GeoJSON, a per-minute O-D CSV
//! (`pu_min,PULocationID,DOLocationID,trips`) and a trip CSV
//! (`pu_min,PULocationID,DOLocationID,dur_min`).

use std::collections::HashMap;

use anyhow::{Context, Result};
use sim_core::assets::{GraphAsset, Provenance, TaxiDayLayer, TaxiOdMinute, TaxiTrip, VehicleRoute};
use sim_core::graph::StreetGraph;
use sim_core::math::Vec2;
use sim_core::projection::{EnuProjection, GeoOrigin};

use crate::dashcam::parse_zones;
use crate::vehicle_routes::{decimate, zone_centroid};

const DECIMATE_TOL_M: f64 = 5.0;
/// Cap the routed O-D pool (distinct pairs, by trip frequency).
const MAX_OD_ROUTES: usize = 5000;

pub fn bake(
    graph_path: &str,
    geojson_path: &str,
    perminute_csv: &str,
    trips_csv: &str,
    date: u32,
    out_path: &str,
) -> Result<usize> {
    let proj = EnuProjection::default();
    let g_bytes = std::fs::read(graph_path).with_context(|| format!("reading {graph_path}"))?;
    let graph = StreetGraph::from_asset(GraphAsset::from_bytes(&g_bytes).context("decoding graph")?);
    let zone_bytes = std::fs::read(geojson_path).with_context(|| format!("reading {geojson_path}"))?;
    let zones = parse_zones(&zone_bytes, &proj)?;
    let mut centroid: HashMap<i64, Vec2> = HashMap::new();
    for (loc, rings) in &zones {
        if let Some(c) = zone_centroid(rings) {
            centroid.insert(*loc, c);
        }
    }

    // 1. Read the day's trips: (pu_min, pu, do, dur_min) + count O-D frequencies.
    let mut trips_raw: Vec<(f32, i64, i64, f32)> = Vec::new();
    let mut od_freq: HashMap<(i64, i64), u32> = HashMap::new();
    let mut rdr = csv::Reader::from_path(trips_csv).with_context(|| format!("opening {trips_csv}"))?;
    for rec in rdr.records() {
        let rec = rec?;
        let pu_min: f32 = rec.get(0).unwrap_or("").parse().unwrap_or(-1.0);
        let pu: i64 = rec.get(1).unwrap_or("").parse().unwrap_or(-1);
        let dolc: i64 = rec.get(2).unwrap_or("").parse().unwrap_or(-1);
        let dur: f32 = rec.get(3).unwrap_or("1").parse().unwrap_or(1.0);
        if pu_min >= 0.0 && pu >= 0 && dolc >= 0 {
            trips_raw.push((pu_min, pu, dolc, dur.max(0.5)));
            *od_freq.entry((pu, dolc)).or_default() += 1;
        }
    }
    anyhow::ensure!(!trips_raw.is_empty(), "no taxi trips parsed");

    // 2. Route the top distinct O-D pairs (by frequency) over the walk graph.
    let mut pairs: Vec<((i64, i64), u32)> = od_freq.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1));
    let mut route_idx: HashMap<(i64, i64), u32> = HashMap::new();
    let mut routes: Vec<VehicleRoute> = Vec::new();
    let (mut tried, mut no_path) = (0usize, 0usize);
    for ((pu, dolc), freq) in pairs.into_iter().take(MAX_OD_ROUTES) {
        let (Some(&from), Some(&to)) = (centroid.get(&pu), centroid.get(&dolc)) else { continue };
        tried += 1;
        let Ok(route) = graph.route_points(from, to) else {
            no_path += 1;
            continue;
        };
        let pts = decimate(&route.points, DECIMATE_TOL_M);
        if pts.len() < 2 {
            continue;
        }
        route_idx.insert((pu, dolc), routes.len() as u32);
        routes.push(VehicleRoute {
            polyline: pts.iter().map(|p| [p.x as f32, p.y as f32]).collect(),
            length_m: route.total_m as f32,
            weight: freq as f32,
        });
    }
    anyhow::ensure!(!routes.is_empty(), "no taxi routes baked");

    // 3. Map each trip → route_idx (drop those whose O-D wasn't routed).
    let mut trips: Vec<TaxiTrip> = Vec::new();
    for (pu_min, pu, dolc, dur) in trips_raw {
        if let Some(&ri) = route_idx.get(&(pu, dolc)) {
            trips.push(TaxiTrip { pu_min, route_idx: ri, dur_min: dur });
        }
    }
    trips.sort_by(|a, b| a.pu_min.total_cmp(&b.pu_min));

    // 4. Full per-minute O-D aggregate (for the analytical flux).
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
        routes,
        trips,
        od_per_minute,
        provenance: Provenance {
            source: "NYC TLC High-Volume FHV (Uber/Lyft) trip records, one service day".into(),
            url: "https://www.nyc.gov/site/tlc/about/tlc-trip-record-data.page".into(),
            license: "NYC OpenData / TLC terms".into(),
            as_of: format!("TLC HVFHV {date}"),
            notes: "Real per-trip replay: pickup minute + O-D zone-centroid route over the \
                    walk graph + real trip duration."
                .into(),
        },
    };
    let (nt, nr, no) = (layer.trips.len(), layer.routes.len(), layer.od_per_minute.len());
    std::fs::write(out_path, layer.to_bytes()?).with_context(|| format!("writing {out_path}"))?;
    eprintln!(
        "taxi day {date}: {nt} trips, {nr} routes ({no_path}/{tried} no-path), {no} od-minute rows -> {out_path}"
    );
    Ok(nt)
}
