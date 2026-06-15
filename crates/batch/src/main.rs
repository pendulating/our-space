//! `batch` — native headless host for citywide exposure computation.
//!
//! Runs the same render-agnostic sim-core systems under MinimalPlugins +
//! ScheduleRunnerPlugin (no winit/render), exploiting native multithreading for
//! the coverage-aggregation heatmap and fast_paths O-D sampling. Built out in
//! Phase 3.

fn main() -> anyhow::Result<()> {
    eprintln!("batch: citywide heatmap + O-D sampling not yet implemented (Phase 3)");
    Ok(())
}
