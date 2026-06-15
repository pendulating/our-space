//! Thin Bevy ECS layer over the pure core (feature `ecs`).
//!
//! The render-agnostic simulation systems operate on these components/resources;
//! the same set is inserted by both the interactive app and the headless batch.
//! Rendering crates add their own visual components on top.

use bevy_ecs::prelude::*;

use crate::exposure::ExposureTally;
use crate::math::Vec2;
use crate::projection::EnuProjection;

/// Position of any spatial entity, in local ENU meters (the common coordinate
/// all render-agnostic systems operate on).
#[derive(Component, Debug, Clone, Copy)]
pub struct WorldPos(pub Vec2);

/// The walker following a routed path.
#[derive(Component, Debug, Clone)]
pub struct PedestrianAgent {
    /// Distance already travelled along the route, meters.
    pub progress_m: f64,
    /// Desired walking speed, m/s.
    pub speed_mps: f64,
}

impl Default for PedestrianAgent {
    fn default() -> Self {
        PedestrianAgent {
            progress_m: 0.0,
            speed_mps: crate::graph::DEFAULT_WALK_SPEED_MPS,
        }
    }
}

/// A fixed directional sensor (CCTV / DOT cam) as an entity.
#[derive(Component, Debug, Clone, Copy)]
pub struct FixedSensor {
    pub kind: crate::exposure::SourceKind,
    pub heading_deg: Option<f64>,
    pub full_fov_deg: f64,
    pub range_m: f64,
    pub frame_rate: f64,
    /// Stable id for distinct-device counting.
    pub id: u64,
}

/// Global discrete simulation clock.
#[derive(Resource, Debug, Clone, Copy)]
pub struct SimClock {
    /// Elapsed simulated seconds since route start.
    pub t: f64,
    /// Fixed time step, seconds.
    pub dt: f64,
    /// Departure time of day in hours [0, 24); scales diurnal intensities.
    pub departure_hour: f64,
}

impl Default for SimClock {
    fn default() -> Self {
        SimClock {
            t: 0.0,
            dt: 1.0,
            departure_hour: 17.0, // 5pm default
        }
    }
}

/// The map projection used to convert app input (lat/lon) to ENU.
#[derive(Resource, Debug, Clone, Copy)]
pub struct MapProjection(pub EnuProjection);

impl Default for MapProjection {
    fn default() -> Self {
        MapProjection(EnuProjection::default())
    }
}

/// The accumulating exposure result for the current route.
#[derive(Resource, Debug, Clone, Default)]
pub struct ExposureLog(pub ExposureTally);
