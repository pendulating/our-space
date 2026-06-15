//! Closed-form WGS84 <-> local ENU-meters projection centered on Manhattan.
//!
//! At neighborhood/borough scale an equirectangular tangent-plane approximation
//! is accurate to well under a meter and keeps FOV geometry and movement
//! Euclidean. The native `proj` crate (libproj C) does not build for
//! `wasm32-unknown-unknown` (georust/proj#115); this avoids it entirely.

use crate::math::Vec2;
use serde::{Deserialize, Serialize};

/// WGS84 mean Earth radius (meters).
const EARTH_RADIUS_M: f64 = 6_371_008.8;

/// Geographic origin of the local ENU frame (degrees).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GeoOrigin {
    pub lat_deg: f64,
    pub lon_deg: f64,
}

impl GeoOrigin {
    /// Roughly the center of Manhattan (Midtown). Good default tangent point.
    pub const MANHATTAN: GeoOrigin = GeoOrigin {
        lat_deg: 40.7831,
        lon_deg: -73.9712,
    };
}

/// Projects WGS84 lat/lon to a local east-north meters frame about an origin.
#[derive(Debug, Clone, Copy)]
pub struct EnuProjection {
    origin: GeoOrigin,
    cos_lat0: f64,
}

impl EnuProjection {
    pub fn new(origin: GeoOrigin) -> Self {
        EnuProjection {
            origin,
            cos_lat0: origin.lat_deg.to_radians().cos(),
        }
    }

    pub fn origin(&self) -> GeoOrigin {
        self.origin
    }

    /// lat/lon (degrees) -> local ENU meters.
    pub fn to_enu(&self, lat_deg: f64, lon_deg: f64) -> Vec2 {
        let d_lat = (lat_deg - self.origin.lat_deg).to_radians();
        let d_lon = (lon_deg - self.origin.lon_deg).to_radians();
        Vec2::new(
            EARTH_RADIUS_M * d_lon * self.cos_lat0, // east
            EARTH_RADIUS_M * d_lat,                 // north
        )
    }

    /// local ENU meters -> lat/lon (degrees).
    pub fn to_wgs84(&self, p: Vec2) -> (f64, f64) {
        let d_lat = p.y / EARTH_RADIUS_M;
        let d_lon = p.x / (EARTH_RADIUS_M * self.cos_lat0);
        (
            self.origin.lat_deg + d_lat.to_degrees(),
            self.origin.lon_deg + d_lon.to_degrees(),
        )
    }
}

impl Default for EnuProjection {
    fn default() -> Self {
        EnuProjection::new(GeoOrigin::MANHATTAN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_maps_to_zero() {
        let p = EnuProjection::default();
        let o = p.to_enu(GeoOrigin::MANHATTAN.lat_deg, GeoOrigin::MANHATTAN.lon_deg);
        assert!(o.length() < 1e-6);
    }

    #[test]
    fn round_trip_within_manhattan() {
        let p = EnuProjection::default();
        // A point in Lower Manhattan (~City Hall).
        let (lat, lon) = (40.7128, -74.0060);
        let enu = p.to_enu(lat, lon);
        let (lat2, lon2) = p.to_wgs84(enu);
        assert!((lat - lat2).abs() < 1e-9, "lat round-trip {lat} vs {lat2}");
        assert!((lon - lon2).abs() < 1e-9, "lon round-trip {lon} vs {lon2}");
    }

    #[test]
    fn east_is_positive_x_north_is_positive_y() {
        let p = EnuProjection::default();
        let east_of = p.to_enu(GeoOrigin::MANHATTAN.lat_deg, GeoOrigin::MANHATTAN.lon_deg + 0.01);
        let north_of = p.to_enu(GeoOrigin::MANHATTAN.lat_deg + 0.01, GeoOrigin::MANHATTAN.lon_deg);
        assert!(east_of.x > 0.0 && east_of.y.abs() < 1e-6);
        assert!(north_of.y > 0.0 && north_of.x.abs() < 1e-6);
    }

    #[test]
    fn one_degree_lat_is_about_111km() {
        let p = EnuProjection::default();
        let north = p.to_enu(GeoOrigin::MANHATTAN.lat_deg + 1.0, GeoOrigin::MANHATTAN.lon_deg);
        // ~111.2 km per degree latitude.
        assert!((north.y - 111_195.0).abs() < 500.0, "got {}", north.y);
    }
}
