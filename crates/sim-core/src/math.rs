//! Minimal 2D vector math in local ENU meters (f64).
//!
//! Deliberately Bevy-free so the analytical core compiles and tests without a
//! game engine or GPU. The ECS/render layer converts these to Bevy `Vec2`/`Vec3`
//! (f32) only at the boundary.

use serde::{Deserialize, Serialize};

/// A 2D point/vector in local ENU meters (east = +x, north = +y).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };

    #[inline]
    pub const fn new(x: f64, y: f64) -> Self {
        Vec2 { x, y }
    }

    #[inline]
    pub fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }

    #[inline]
    pub fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }

    #[inline]
    pub fn scale(self, s: f64) -> Vec2 {
        Vec2::new(self.x * s, self.y * s)
    }

    #[inline]
    pub fn dot(self, o: Vec2) -> f64 {
        self.x * o.x + self.y * o.y
    }

    /// 2D cross product (z-component), useful for orientation tests.
    #[inline]
    pub fn cross(self, o: Vec2) -> f64 {
        self.x * o.y - self.y * o.x
    }

    #[inline]
    pub fn length_sq(self) -> f64 {
        self.dot(self)
    }

    #[inline]
    pub fn length(self) -> f64 {
        self.length_sq().sqrt()
    }

    #[inline]
    pub fn distance(self, o: Vec2) -> f64 {
        self.sub(o).length()
    }

    /// Linear interpolation; `t` in [0, 1].
    #[inline]
    pub fn lerp(self, o: Vec2, t: f64) -> Vec2 {
        Vec2::new(self.x + (o.x - self.x) * t, self.y + (o.y - self.y) * t)
    }

    /// Compass bearing in radians measured clockwise from north (+y).
    /// 0 = north, π/2 = east. Matches how camera/street headings are recorded.
    #[inline]
    pub fn bearing_rad(self) -> f64 {
        self.x.atan2(self.y)
    }
}

/// Smallest absolute difference between two angles (radians), in [0, π].
#[inline]
pub fn angle_diff(a: f64, b: f64) -> f64 {
    let mut d = (a - b) % (2.0 * std::f64::consts::PI);
    if d < -std::f64::consts::PI {
        d += 2.0 * std::f64::consts::PI;
    } else if d > std::f64::consts::PI {
        d -= 2.0 * std::f64::consts::PI;
    }
    d.abs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn distance_and_lerp() {
        let a = Vec2::new(0.0, 0.0);
        let b = Vec2::new(3.0, 4.0);
        assert!((a.distance(b) - 5.0).abs() < 1e-9);
        let m = a.lerp(b, 0.5);
        assert!((m.x - 1.5).abs() < 1e-9 && (m.y - 2.0).abs() < 1e-9);
    }

    #[test]
    fn bearing_compass_convention() {
        // due north
        assert!(angle_diff(Vec2::new(0.0, 1.0).bearing_rad(), 0.0) < 1e-9);
        // due east
        assert!(angle_diff(Vec2::new(1.0, 0.0).bearing_rad(), PI / 2.0) < 1e-9);
    }

    #[test]
    fn angle_diff_wraps() {
        assert!((angle_diff(0.1, 2.0 * PI - 0.1) - 0.2).abs() < 1e-9);
        assert!((angle_diff(0.0, PI) - PI).abs() < 1e-9);
    }
}
