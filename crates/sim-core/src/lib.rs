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
pub mod projection;
pub mod rng;
pub mod scenario;
pub mod simulation;

#[cfg(feature = "ecs")]
pub mod ecs;

// Convenience re-exports.
pub use exposure::{ConfidenceTier, ExposureTally, SourceKind, DAHIR_RECALL};
pub use geometry::{captures, FrustumWedge, OccluderEdge};
pub use graph::{Route, RouteError, StreetGraph, Walkshed, DEFAULT_WALK_SPEED_MPS};
pub use math::Vec2;
pub use mobile::{AceConfig, DashcamConfig, GlassesConfig, MobileScenario};
pub use projection::{EnuProjection, GeoOrigin};
pub use rng::{RngLike, WyRand};
pub use scenario::{
    run_route, sensors_from_layer, summarize, walkshed_exposure, FixedCameraDefaults, RouteSummary,
    SourceBreakdown, WalkshedSummary,
};
pub use simulation::{
    exposure_rates_per_minute, simulate_fixed, simulate_full, ExposureRates, SensorInstance,
    SimParams,
};
