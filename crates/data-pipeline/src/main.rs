//! `data-pipeline` — turns raw NYC open datasets into compact, client-loadable
//! static assets (the routable graph + per-class sensor layers).
//!
//! Usage:
//!   data-pipeline bake-graph --synthetic <rows> <cols> <spacing_m> <out.postcard>
//!   data-pipeline bake-graph --geojson <walk.geojson> <out.postcard>   (TODO)
//!   data-pipeline bake-cameras <map_data.csv> <out.postcard>

mod ace;
mod alpr;
mod amnesty;
mod borough;
mod boundary;
mod bus_day;
mod cameras_dahir;
mod dashcam;
mod enforcement;
mod dot;
mod equity;
mod extent;
mod facilities;
mod footprints;
mod parks;
mod plazas;
mod graph_osm;
mod landmarks;
mod linknyc;
mod graph_synth;
mod neighborhoods;
mod robotability;
mod taxi_day;
mod tesla;
mod vehicle_routes;

use std::path::Path;
use std::process::ExitCode;

use anyhow::{bail, Context};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("bake-graph") => bake_graph(&args[2..]),
        Some("bake-cameras") => {
            let csv = args.get(2).context(USAGE)?;
            let out = args.get(3).context(USAGE)?;
            let ext = extent::Extent::parse(args.get(4).map(String::as_str));
            ensure_parent(out)?;
            cameras_dahir::bake(csv, out, ext)?;
            Ok(())
        }
        Some("bake-cctv") => {
            let amnesty = args.get(2).context(USAGE)?;
            let dahir = args.get(3).context(USAGE)?;
            let out = args.get(4).context(USAGE)?;
            let ext = extent::Extent::parse(args.get(5).map(String::as_str));
            ensure_parent(out)?;
            amnesty::bake(amnesty, dahir, out, ext)?;
            Ok(())
        }
        Some("bake-ace") => {
            let gtfs_dir = args.get(2).context(USAGE)?;
            let ace_json = args.get(3).context(USAGE)?;
            let out = args.get(4).context(USAGE)?;
            let ext = extent::Extent::parse(args.get(5).map(String::as_str));
            let boundary = args.get(6).map(String::as_str); // optional borough-clip GeoJSON
            ensure_parent(out)?;
            ace::bake(gtfs_dir, ace_json, out, ext, boundary)?;
            Ok(())
        }
        Some("bake-equity") => {
            let geojson = args.get(2).context(USAGE)?;
            let acs = args.get(3).context(USAGE)?;
            let csv = args.get(4).context(USAGE)?;
            let out = args.get(5).context(USAGE)?;
            ensure_parent(out)?;
            equity::bake(geojson, acs, csv, out)?;
            Ok(())
        }
        Some("bake-bus-day") => {
            let gtfs = args.get(2).context(USAGE)?;
            let ace_json = args.get(3).context(USAGE)?;
            let date: u32 = args.get(4).context(USAGE)?.parse().context("date YYYYMMDD")?;
            let out = args.get(5).context(USAGE)?;
            let ext = extent::Extent::parse(args.get(6).map(String::as_str));
            let boundary = args.get(7).map(String::as_str); // optional borough-clip GeoJSON
            ensure_parent(out)?;
            bus_day::bake(gtfs, ace_json, date, out, ext, boundary)?;
            Ok(())
        }
        Some("bake-taxi-day") => {
            let graph = args.get(2).context(USAGE)?;
            let geojson = args.get(3).context(USAGE)?;
            let perminute = args.get(4).context(USAGE)?;
            let trips = args.get(5).context(USAGE)?;
            let date: u32 = args.get(6).context(USAGE)?.parse().context("date YYYYMMDD")?;
            let out = args.get(7).context(USAGE)?;
            // [footprints.geojson] optional: dasymetric within-zone endpoint weighting.
            let footprints = args.get(8).map(String::as_str);
            ensure_parent(out)?;
            taxi_day::bake(graph, geojson, perminute, trips, date, out, footprints)?;
            Ok(())
        }
        Some("bake-robotability") => {
            let graph = args.get(2).context(USAGE)?;
            let geojson = args.get(3).context(USAGE)?;
            let out = args.get(4).context(USAGE)?;
            ensure_parent(out)?;
            robotability::bake(graph, geojson, out)?;
            Ok(())
        }
        Some("bake-enforcement") => {
            let csv = args.get(2).context(USAGE)?;
            let out = args.get(3).context(USAGE)?;
            ensure_parent(out)?;
            enforcement::bake(csv, out)?;
            Ok(())
        }
        Some("bake-teslas") => {
            let geojson = args.get(2).context(USAGE)?;
            let counts = args.get(3).context(USAGE)?;
            let out = args.get(4).context(USAGE)?;
            ensure_parent(out)?;
            tesla::bake(geojson, counts, out)?;
            Ok(())
        }
        Some("bake-neighborhoods") => {
            let geojson = args.get(2).context(USAGE)?;
            let out = args.get(3).context(USAGE)?;
            ensure_parent(out)?;
            neighborhoods::bake(geojson, out)?;
            Ok(())
        }
        Some("bake-borough") => {
            let geojson = args.get(2).context(USAGE)?;
            let name = args.get(3).context(USAGE)?;
            let out = args.get(4).context(USAGE)?;
            ensure_parent(out)?;
            borough::bake(geojson, name, out)?;
            Ok(())
        }
        Some("bake-footprints") => {
            let geojson = args.get(2).context(USAGE)?;
            let out = args.get(3).context(USAGE)?;
            // [borough] selects one borough from a citywide GeoJSON by BIN digit
            // ("all"/omitted = keep every feature, for pre-filtered inputs).
            let borough = args.get(4).map(String::as_str).filter(|s| *s != "all");
            let boundary = args.get(5).map(String::as_str); // optional borough-clip GeoJSON
            ensure_parent(out)?;
            footprints::bake(geojson, out, borough, boundary)?;
            Ok(())
        }
        Some("bake-parks") => {
            let geojson = args.get(2).context(USAGE)?;
            let out = args.get(3).context(USAGE)?;
            // [borough] selects one borough by the Parks `borough` code (M/B/Q/X/R);
            // "all"/omitted keeps every borough (citywide).
            let borough = args.get(4).map(String::as_str).filter(|s| *s != "all");
            // [boundary] optional shoreline GeoJSON: clip parks to land so none spills
            // into open water (e.g. Randall's/Ward's Island overshoot).
            let boundary = args.get(5).map(String::as_str);
            ensure_parent(out)?;
            parks::bake(geojson, out, borough, boundary)?;
            Ok(())
        }
        Some("bake-plazas") => {
            let geojson = args.get(2).context(USAGE)?;
            let out = args.get(3).context(USAGE)?;
            ensure_parent(out)?;
            plazas::bake(geojson, out)?;
            Ok(())
        }
        Some("bake-landmarks") => {
            let json = args.get(2).context(USAGE)?;
            let out = args.get(3).context(USAGE)?;
            ensure_parent(out)?;
            landmarks::bake(json, out)?;
            Ok(())
        }
        Some("bake-linknyc") => {
            let json = args.get(2).context(USAGE)?;
            let out = args.get(3).context(USAGE)?;
            let boundary = args.get(4).map(String::as_str); // optional Manhattan-clip GeoJSON
            ensure_parent(out)?;
            linknyc::bake(json, out, boundary)?;
            Ok(())
        }
        Some("bake-facilities") => {
            let json = args.get(2).context(USAGE)?;
            let out = args.get(3).context(USAGE)?;
            let boro = args.get(4).map(String::as_str); // optional borough filter (e.g. MANHATTAN)
            ensure_parent(out)?;
            facilities::bake(json, out, boro)?;
            Ok(())
        }
        Some("bake-dashcam-field") => {
            let geojson = args.get(2).context(USAGE)?;
            let trips = args.get(3).context(USAGE)?;
            let out = args.get(4).context(USAGE)?;
            ensure_parent(out)?;
            dashcam::bake(geojson, trips, out)?;
            Ok(())
        }
        Some("bake-alpr") => {
            let json = args.get(2).context(USAGE)?;
            let out = args.get(3).context(USAGE)?;
            ensure_parent(out)?;
            alpr::bake(json, out)?;
            Ok(())
        }
        Some("bake-dot") => {
            let json = args.get(2).context(USAGE)?;
            let out = args.get(3).context(USAGE)?;
            let ext = extent::Extent::parse(args.get(4).map(String::as_str));
            ensure_parent(out)?;
            dot::bake(json, out, ext)?;
            Ok(())
        }
        Some("bake-vehicle-routes") => {
            let graph = args.get(2).context(USAGE)?;
            let geojson = args.get(3).context(USAGE)?;
            let od = args.get(4).context(USAGE)?;
            let out = args.get(5).context(USAGE)?;
            let max_routes: usize = args.get(6).and_then(|s| s.parse().ok()).unwrap_or(1000);
            ensure_parent(out)?;
            vehicle_routes::bake(graph, geojson, od, out, max_routes)?;
            Ok(())
        }
        _ => bail!(USAGE),
    }
}

fn bake_graph(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("--synthetic") => {
            let rows: u32 = parse(args.get(1), "rows")?;
            let cols: u32 = parse(args.get(2), "cols")?;
            let spacing: f64 = parse(args.get(3), "spacing_m")?;
            let out = args.get(4).context(USAGE)?;
            let g = graph_synth::synthetic_grid(rows, cols, spacing);
            let (n, e) = (g.nodes.len(), g.edges.len());
            ensure_parent(out)?;
            std::fs::write(out, g.to_bytes()?).with_context(|| format!("writing {out}"))?;
            eprintln!("synthetic graph: {n} nodes, {e} edges -> {out}");
            Ok(())
        }
        Some("--overpass-json") => {
            let json = args.get(1).context(USAGE)?;
            let out = args.get(2).context(USAGE)?;
            let boundary = args.get(3).map(String::as_str); // optional Manhattan-clip GeoJSON
            ensure_parent(out)?;
            graph_osm::bake(json, out, boundary, false)?; // pedestrian walk network
            Ok(())
        }
        Some("--overpass-drive") => {
            // Drive network for vehicle/taxi routing: excludes pedestrian plazas +
            // access-restricted ways so cars don't route over pedestrianized streets.
            let json = args.get(1).context(USAGE)?;
            let out = args.get(2).context(USAGE)?;
            let boundary = args.get(3).map(String::as_str);
            ensure_parent(out)?;
            graph_osm::bake(json, out, boundary, true)?;
            Ok(())
        }
        Some("--cscl") => {
            // Five-borough street network from NYC's CSCL centerline GeoJSON (Socrata
            // inkn-q76z) — the Overpass route can't pull all of NYC at once. Optional
            // 3rd arg clips to a borough boundary (e.g. a Manhattan drive graph whose
            // rw_type, carried in segment_id, drives time-based taxi routing); optional
            // 4th arg is a parks GeoJSON whose interiors are dropped from the drivable
            // surface network (CSCL codes car-free park drives/paths as "Street").
            let geojson = args.get(1).context(USAGE)?;
            let out = args.get(2).context(USAGE)?;
            // `-` (or empty) skips an optional path arg, so parks can be passed without
            // a borough clip (the citywide graph keeps all five boroughs).
            let opt = |i: usize| args.get(i).map(String::as_str).filter(|s| !s.is_empty() && *s != "-");
            let boundary = opt(3);
            let parks = opt(4);
            let open_streets = opt(5); // NYC DOT Open Streets (car-free) mask
            ensure_parent(out)?;
            graph_osm::bake_cscl(geojson, out, boundary, parks, open_streets)?;
            Ok(())
        }
        _ => bail!(USAGE),
    }
}

fn parse<T: std::str::FromStr>(v: Option<&String>, name: &str) -> anyhow::Result<T> {
    v.context(USAGE)?
        .parse::<T>()
        .map_err(|_| anyhow::anyhow!("could not parse {name}"))
}

fn ensure_parent(path: &str) -> anyhow::Result<()> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
    }
    Ok(())
}

const USAGE: &str = "usage:\n  \
    data-pipeline bake-graph --synthetic <rows> <cols> <spacing_m> <out.postcard>\n  \
    data-pipeline bake-graph --overpass-json <walk.json> <out.postcard> [manhattan.geojson]\n  \
    data-pipeline bake-graph --overpass-drive <walk.json> <out.osgraph> [manhattan.geojson]\n  \
    data-pipeline bake-graph --cscl <cscl.geojson> <out.osgraph> [boundary.geojson|-] [parks.geojson|-] [open_streets.geojson]\n  \
    data-pipeline bake-cameras <map_data.csv> <out.postcard> [manhattan|nyc]\n  \
    data-pipeline bake-cctv <amnesty_counts_per_intersections.csv> <dahir_map_data.csv> <out.postcard> [manhattan|nyc]\n  \
    data-pipeline bake-ace <gtfs_dir> <ace_routes.json> <out.postcard> [manhattan|nyc] [boundary.geojson]\n  \
    data-pipeline bake-equity <bg.geojson> <acs.json> <map_data.csv> <out.postcard>\n  \
    data-pipeline bake-neighborhoods <neighborhoods.geojson> <out.osneigh>\n  \
    data-pipeline bake-borough <borough_boundaries.geojson> <BoroughName> <out.osboro>\n  \
    data-pipeline bake-footprints <building_footprints.geojson> <out.osbldg> [borough|all] [boundary.geojson]\n  \
    data-pipeline bake-parks <parks_properties.geojson> <out.ospark> [borough|all]\n  \
    data-pipeline bake-plazas <pedestrian_plazas.geojson> <out.osplaza>\n  \
    data-pipeline bake-landmarks <landmarks_lod2.json> <out.oslmk>\n  \
    data-pipeline bake-linknyc <kiosks.json> <out.oslink> [manhattan.geojson]\n  \
    data-pipeline bake-robotability <graph.osgraph> <sidewalks.geojson> <out.osrobot>\n  \
    data-pipeline bake-teslas <nyc_zips.geojson> <tesla_by_zip.csv> <out.osteslas>\n  \
    data-pipeline bake-enforcement <enforcement_signs.csv> <out.oscam>\n  \
    data-pipeline bake-bus-day <gtfs_dir> <ace_routes.json> <YYYYMMDD> <out.osbusday> [manhattan|nyc] [boundary.geojson]\n  \
    data-pipeline bake-taxi-day <graph.osgraph> <taxi_zones.geojson> <perminute_od.csv> <trips_all.csv> <YYYYMMDD> <out.ostaxiday>\n  \
    data-pipeline bake-dashcam-field <taxi_zones.geojson> <zone_trips.csv> <out.postcard>\n  \
    data-pipeline bake-alpr <alpr_overpass.json> <out.postcard>\n  \
    data-pipeline bake-dot <nyctmc_cameras.json> <out.postcard> [manhattan|nyc]\n  \
    data-pipeline bake-vehicle-routes <graph.osgraph> <taxi_zones.geojson> <zone_od.csv> <out.osroutes> [max_routes]";
