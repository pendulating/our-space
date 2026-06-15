//! `data-pipeline` — turns raw NYC open datasets into compact, client-loadable
//! static assets (the routable graph + per-class sensor layers).
//!
//! Usage:
//!   data-pipeline bake-graph --synthetic <rows> <cols> <spacing_m> <out.postcard>
//!   data-pipeline bake-graph --geojson <walk.geojson> <out.postcard>   (TODO)
//!   data-pipeline bake-cameras <map_data.csv> <out.postcard>

mod ace;
mod cameras_dahir;
mod equity;
mod graph_osm;
mod graph_synth;

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
            ensure_parent(out)?;
            cameras_dahir::bake(csv, out)?;
            Ok(())
        }
        Some("bake-ace") => {
            let gtfs_dir = args.get(2).context(USAGE)?;
            let ace_json = args.get(3).context(USAGE)?;
            let out = args.get(4).context(USAGE)?;
            ensure_parent(out)?;
            ace::bake(gtfs_dir, ace_json, out)?;
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
            ensure_parent(out)?;
            graph_osm::bake(json, out)?;
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
    data-pipeline bake-graph --overpass-json <walk.json> <out.postcard>\n  \
    data-pipeline bake-cameras <map_data.csv> <out.postcard>\n  \
    data-pipeline bake-ace <gtfs_dir> <ace_routes.json> <out.postcard>\n  \
    data-pipeline bake-equity <bg.geojson> <acs.json> <map_data.csv> <out.postcard>";
