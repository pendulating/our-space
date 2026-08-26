#!/usr/bin/env bash
#SBATCH --job-name=ourspace-drive
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=16
#SBATCH --mem=32G
#SBATCH --time=02:00:00
#SBATCH --output=data/derived/logs/slurm-drive-%j.out
#SBATCH --partition=default_partition
#SBATCH --nodelist=lisbeth
set -euo pipefail
cd "${SLURM_SUBMIT_DIR:-$(dirname "$0")/..}"
export RAYON_NUM_THREADS="${SLURM_CPUS_PER_TASK:-16}"
exec target/release/batch od-exposure \
  crates/app-interactive/assets/processed/graph_nyc.osgraph \
  crates/app-interactive/assets/processed/graph_nyc_walk.osgraph \
  data/snapshots/census/bg_centroids_nyc.csv \
  data/snapshots/lodes/bg_od_nyc.csv \
  data/derived/exposure/A_i_drive_bg_nyc.csv 10 900
