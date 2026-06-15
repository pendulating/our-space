//! Sensor capture geometry: directional field-of-view wedges and cheap 2D
//! building-footprint line-of-sight occlusion.
//!
//! Manhattan sightlines are wall-limited, so ignoring occlusion grossly
//! over-counts fixed-camera coverage. We test the camera->walker sightline
//! against nearby building-footprint edges with a simple segment-intersection.

use crate::math::{angle_diff, Vec2};

/// A directional view frustum projected to 2D: an apex, a look bearing, a
/// half-angle, and a maximum range. Compass bearing convention (0 = north).
#[derive(Debug, Clone, Copy)]
pub struct FrustumWedge {
    pub apex: Vec2,
    /// Look direction as a compass bearing in radians (0 = north, clockwise).
    pub heading_rad: f64,
    /// Half of the horizontal field of view, in radians.
    pub half_fov_rad: f64,
    /// Maximum useful capture range, in meters.
    pub range_m: f64,
}

impl FrustumWedge {
    /// Build from human-friendly degrees. `full_fov_deg` is the *full* angular
    /// width; pass `None` heading for an omnidirectional sensor (dome/PTZ),
    /// which is modeled as a full 360° wedge.
    pub fn from_degrees(
        apex: Vec2,
        heading_deg: Option<f64>,
        full_fov_deg: f64,
        range_m: f64,
    ) -> Self {
        match heading_deg {
            Some(h) => FrustumWedge {
                apex,
                heading_rad: h.to_radians(),
                half_fov_rad: 0.5 * full_fov_deg.to_radians(),
                range_m,
            },
            None => FrustumWedge {
                apex,
                heading_rad: 0.0,
                half_fov_rad: std::f64::consts::PI, // 360° coverage
                range_m,
            },
        }
    }

    /// Is `p` within range and within the angular wedge (ignoring occlusion)?
    pub fn covers_unoccluded(&self, p: Vec2) -> bool {
        let to_p = p.sub(self.apex);
        let dist = to_p.length();
        if dist > self.range_m || dist < f64::EPSILON {
            // Range fail. (A point exactly at the apex counts as covered.)
            return dist <= self.range_m;
        }
        if self.half_fov_rad >= std::f64::consts::PI {
            return true; // omnidirectional
        }
        angle_diff(to_p.bearing_rad(), self.heading_rad) <= self.half_fov_rad
    }
}

/// Returns true if open segments (a1,a2) and (b1,b2) properly cross.
/// Uses orientation signs; collinear/touching cases return false (good enough
/// for occlusion — a sightline merely grazing a wall corner is treated as clear).
pub fn segments_cross(a1: Vec2, a2: Vec2, b1: Vec2, b2: Vec2) -> bool {
    let d1 = a2.sub(a1).cross(b1.sub(a1));
    let d2 = a2.sub(a1).cross(b2.sub(a1));
    let d3 = b2.sub(b1).cross(a1.sub(b1));
    let d4 = b2.sub(b1).cross(a2.sub(b1));
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

/// A building footprint edge (a wall segment) for occlusion testing.
#[derive(Debug, Clone, Copy)]
pub struct OccluderEdge {
    pub a: Vec2,
    pub b: Vec2,
}

/// Is the sightline from `from` to `to` blocked by any of the given occluders?
/// Callers should pre-filter occluders to those near the segment (e.g. via an
/// R-tree query) so this stays O(few).
pub fn sightline_blocked(from: Vec2, to: Vec2, occluders: &[OccluderEdge]) -> bool {
    occluders
        .iter()
        .any(|e| segments_cross(from, to, e.a, e.b))
}

/// Full capture test: in the wedge AND with a clear sightline.
pub fn captures(wedge: &FrustumWedge, target: Vec2, occluders: &[OccluderEdge]) -> bool {
    wedge.covers_unoccluded(target) && !sightline_blocked(wedge.apex, target, occluders)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn north_cam() -> FrustumWedge {
        // Camera at origin looking north, 90° full FOV, 20 m range.
        FrustumWedge::from_degrees(Vec2::ZERO, Some(0.0), 90.0, 20.0)
    }

    #[test]
    fn in_wedge_and_range() {
        let cam = north_cam();
        assert!(cam.covers_unoccluded(Vec2::new(0.0, 10.0))); // straight ahead
        assert!(cam.covers_unoccluded(Vec2::new(5.0, 10.0))); // within 45° of north
    }

    #[test]
    fn out_of_range_and_behind() {
        let cam = north_cam();
        assert!(!cam.covers_unoccluded(Vec2::new(0.0, 25.0))); // too far
        assert!(!cam.covers_unoccluded(Vec2::new(0.0, -5.0))); // behind (south)
        assert!(!cam.covers_unoccluded(Vec2::new(15.0, 1.0))); // ~86° off-axis
    }

    #[test]
    fn omnidirectional_covers_all_directions_in_range() {
        let dome = FrustumWedge::from_degrees(Vec2::ZERO, None, 0.0, 10.0);
        assert!(dome.covers_unoccluded(Vec2::new(0.0, -9.0)));
        assert!(dome.covers_unoccluded(Vec2::new(7.0, 0.0)));
        assert!(!dome.covers_unoccluded(Vec2::new(0.0, -11.0)));
    }

    #[test]
    fn segment_crossing() {
        // a horizontal segment and a vertical one that cross
        assert!(segments_cross(
            Vec2::new(-1.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, -1.0),
            Vec2::new(0.0, 1.0),
        ));
        // parallel, no cross
        assert!(!segments_cross(
            Vec2::new(-1.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(-1.0, 1.0),
            Vec2::new(1.0, 1.0),
        ));
    }

    #[test]
    fn occlusion_blocks_capture() {
        let cam = north_cam();
        let target = Vec2::new(0.0, 10.0);
        // A wall between the camera and the target (at y = 5).
        let wall = [OccluderEdge {
            a: Vec2::new(-5.0, 5.0),
            b: Vec2::new(5.0, 5.0),
        }];
        assert!(cam.covers_unoccluded(target));
        assert!(sightline_blocked(cam.apex, target, &wall));
        assert!(!captures(&cam, target, &wall));
        // No wall -> capture succeeds.
        assert!(captures(&cam, target, &[]));
    }

    #[test]
    fn half_fov_boundary() {
        let cam = north_cam(); // half FOV = 45°
        // exactly 45° off north, in range
        let p = Vec2::new((PI / 4.0).sin() * 10.0, (PI / 4.0).cos() * 10.0);
        assert!(cam.covers_unoccluded(p));
    }
}
