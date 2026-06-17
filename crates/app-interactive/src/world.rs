//! Map geometry helpers: convert ENU meters to Bevy world space and build the
//! static street / camera / field-of-view meshes.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use sim_core::assets::GraphAsset;
use sim_core::Vec2 as Enu;

/// We render directly in ENU meters: 1 world unit = 1 meter.
pub fn to_world(p: Enu, z: f32) -> Vec3 {
    Vec3::new(p.x as f32, p.y as f32, z)
}

/// Flatten every edge polyline into a `LineList` vertex buffer.
pub fn street_line_positions(asset: &GraphAsset) -> Vec<[f32; 3]> {
    let mut v = Vec::new();
    for e in &asset.edges {
        for w in e.polyline.windows(2) {
            v.push([w[0][0] as f32, w[0][1] as f32, 0.0]);
            v.push([w[1][0] as f32, w[1][1] as f32, 0.0]);
        }
    }
    v
}

/// One merged `TriangleList` of textured `size`-meter quads, one per position,
/// each UV-mapped 0..1 so a `ColorMaterial { texture }` paints the class icon at
/// every camera. Collapses thousands of camera markers into a single
/// mesh/entity/draw-call (a 4-vert quad is even cheaper than the old polygon
/// dots), preserving the perf win while showing recognizable icons.
pub fn merged_icon_quads(positions: &[Enu], size: f32) -> Mesh {
    let h = size * 0.5;
    let n = positions.len();
    let mut verts: Vec<[f32; 3]> = Vec::with_capacity(n * 4);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(n * 4);
    let mut indices: Vec<u32> = Vec::with_capacity(n * 6);
    for p in positions {
        let (x, y) = (p.x as f32, p.y as f32);
        let base = verts.len() as u32;
        // World +y is north (up); image v increases downward, so the top corners
        // (higher y) get v = 0.
        verts.push([x - h, y + h, 0.0]); uvs.push([0.0, 0.0]); // top-left
        verts.push([x + h, y + h, 0.0]); uvs.push([1.0, 0.0]); // top-right
        verts.push([x + h, y - h, 0.0]); uvs.push([1.0, 1.0]); // bottom-right
        verts.push([x - h, y - h, 0.0]); uvs.push([0.0, 1.0]); // bottom-left
        indices.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, verts);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// A `LineList` mesh from raw segment endpoints.
pub fn line_list_mesh(positions: Vec<[f32; 3]>) -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh
}

/// A `LineStrip` mesh through an ordered list of ENU points (used for the route).
pub fn line_strip_mesh(points: &[Enu], z: f32) -> Mesh {
    let positions: Vec<[f32; 3]> = points.iter().map(|p| [p.x as f32, p.y as f32, z]).collect();
    let mut mesh = Mesh::new(PrimitiveTopology::LineStrip, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh
}

/// A filled field-of-view wedge fan with its apex at the local origin, opening
/// toward `heading_rad` (compass bearing: 0 = +y/north, clockwise) with the
/// given half-angle and range. The entity's `Transform` places the apex.
pub fn wedge_mesh(heading_rad: f32, half_fov_rad: f32, range_m: f32, segments: usize) -> Mesh {
    let mut positions: Vec<[f32; 3]> = vec![[0.0, 0.0, 0.0]];
    for i in 0..=segments {
        let t = i as f32 / segments as f32;
        let bearing = heading_rad + (t * 2.0 - 1.0) * half_fov_rad;
        // Compass bearing -> world XY (north = +y).
        positions.push([range_m * bearing.sin(), range_m * bearing.cos(), 0.0]);
    }
    let mut indices = Vec::new();
    for i in 1..positions.len() - 1 {
        indices.push(0u32);
        indices.push(i as u32);
        indices.push((i + 1) as u32);
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// A filled polygon mesh from an ENU exterior ring, triangulated with ear
/// clipping (handles concave block-group boundaries). Returns `None` if the ring
/// is degenerate.
pub fn filled_polygon_mesh(ring: &[[f64; 2]], z: f32) -> Option<Mesh> {
    // Drop a duplicated closing vertex if present.
    let ring = if ring.len() >= 2 && ring.first() == ring.last() {
        &ring[..ring.len() - 1]
    } else {
        ring
    };
    if ring.len() < 3 {
        return None;
    }
    let flat: Vec<f64> = ring.iter().flat_map(|p| [p[0], p[1]]).collect();
    let indices = earcutr::earcut(&flat, &[], 2).ok()?;
    if indices.is_empty() {
        return None;
    }
    let positions: Vec<[f32; 3]> = ring.iter().map(|p| [p[0] as f32, p[1] as f32, z]).collect();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_indices(Indices::U32(indices.iter().map(|&i| i as u32).collect()));
    Some(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::assets::{EdgeData, NodePoint, Provenance};
    use sim_core::projection::GeoOrigin;

    #[test]
    fn street_positions_two_per_segment() {
        let asset = GraphAsset {
            origin: GeoOrigin::MANHATTAN,
            nodes: vec![NodePoint { x: 0.0, y: 0.0 }, NodePoint { x: 1.0, y: 0.0 }],
            edges: vec![EdgeData {
                from: 0,
                to: 1,
                length_m: 1.0,
                polyline: vec![[0.0, 0.0], [0.5, 0.0], [1.0, 0.0]], // 2 segments
                segment_id: None,
            }],
            provenance: Provenance {
                source: String::new(),
                url: String::new(),
                license: String::new(),
                as_of: String::new(),
                notes: String::new(),
            },
        };
        // 2 segments -> 4 line-list vertices.
        assert_eq!(street_line_positions(&asset).len(), 4);
    }

    #[test]
    fn polygon_triangulates() {
        let ring = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]];
        let mesh = filled_polygon_mesh(&ring, 0.0).expect("square triangulates");
        assert_eq!(mesh.count_vertices(), 4); // closing vertex dropped
    }
}
