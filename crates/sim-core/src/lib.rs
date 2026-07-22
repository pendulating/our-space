//! `sim-core` — the render-agnostic simulation core for **our-space**.
//!
//! Everything here is free of Bevy/render dependencies (the `ecs` feature adds a
//! thin Bevy component/resource layer) so the same movement, capture-detection,
//! and exposure logic runs in the interactive web build and the native headless
//! batch, and so the analytical core compiles and tests in milliseconds.
//!
//! Module map:
//! - [`math`] — 2D vectors in local ENU meters.
//! - [`projection`] — closed-form WGS84 <-> ENU (no native PROJ).
//! - [`geometry`] — FOV wedges + 2D line-of-sight occlusion.
//! - [`exposure`] — the exposure model (headline "cameras that saw you" + E[C]).
//! - [`assets`] — baked static asset structs (postcard).
//! - [`graph`] — routable pedestrian graph, A*, and position-over-time.

pub mod assets;
pub mod exposure;
pub mod geometry;
pub mod graph;
pub mod math;
pub mod mobile;
pub mod occlusion;
pub mod projection;
pub mod rng;
pub mod scenario;
pub mod simulation;
pub mod spatial;
pub mod subway;

#[cfg(feature = "ecs")]
pub mod ecs;

// Convenience re-exports.
pub use exposure::{ConfidenceTier, ExposureTally, SourceKind, CENSUS_RECALL, DAHIR_RECALL};
pub use occlusion::{OccluderIndex, DEFAULT_CELL_M};
pub use geometry::{captures, FrustumWedge, OccluderEdge};
pub use graph::{PaceProfile, Route, RouteError, StreetGraph, Walkshed, DEFAULT_WALK_SPEED_MPS};
pub use math::Vec2;
pub use mobile::{AceConfig, DashcamConfig, GlassesConfig, MobileScenario, RobotConfig, TeslaConfig};
pub use projection::{EnuProjection, GeoOrigin};
pub use rng::{RngLike, WyRand};
pub use scenario::{
    arc_union_fraction, direct_capture_exposure, group_sensors, run_route, sample_polyline,
    sample_polyline_into, sensors_from_layer, summarize, walkshed_exposure, walkshed_exposure_with,
    CapturingCamera, DirectCaptureSummary, FixedCameraDefaults, FovModel, RouteSummary,
    SourceBreakdown, WalkshedSummary, EXPOSURE_SAMPLE_STRIDE_M,
};
pub use simulation::{
    exposure_rates_per_minute, simulate_fixed, simulate_full, ExposureRates, SensorInstance,
    SimParams,
};
pub use spatial::{AceGrid, SensorIndex, ZoneGrid};
pub use subway::{
    build_subway_matrix, FeedTrip, Itinerary, SubwayBuildParams, SubwayFeed, SubwayMatrix,
    SubwayStation,
};
