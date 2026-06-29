//! Map geometry helpers: convert ENU meters to Bevy world space and build the
//! static street / camera / field-of-view meshes.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use sim_core::assets::{GraphAsset, Landmark};
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

/// Perpendicular-offset quad corners for a set of disjoint segments — segment `i` owns
/// verts `[4i, 4i+3]`, a ribbon of half-width `half` world meters at depth `z`. (GL line
/// primitives are stuck at 1 px under WebGPU, so visible-width lines must be quads.)
pub fn ribbon_positions(segments: &[[Enu; 2]], half: f32, z: f32) -> Vec<[f32; 3]> {
    let mut verts = Vec::with_capacity(segments.len() * 4);
    for [a, b] in segments {
        let (ax, ay, bx, by) = (a.x as f32, a.y as f32, b.x as f32, b.y as f32);
        let (dx, dy) = (bx - ax, by - ay);
        let len = (dx * dx + dy * dy).sqrt();
        let inv = if len < 1e-6 { 0.0 } else { half / len };
        let (nx, ny) = (-dy * inv, dx * inv); // unit normal × half-width
        verts.push([ax + nx, ay + ny, z]);
        verts.push([bx + nx, by + ny, z]);
        verts.push([bx - nx, by - ny, z]);
        verts.push([ax - nx, ay - ny, z]);
    }
    verts
}

/// A filled-quad ribbon mesh for disjoint segments (see [`ribbon_positions`]).
pub fn thick_line_list_mesh(segments: &[[Enu; 2]], half: f32, z: f32) -> Mesh {
    let verts = ribbon_positions(segments, half, z);
    let mut indices = Vec::with_capacity(segments.len() * 6);
    for i in 0..segments.len() as u32 {
        let b = i * 4;
        indices.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, verts);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// A filled-quad ribbon where each segment carries its **own** half-width and RGBA
/// vertex color (pair with a white `ColorMaterial` so the colors show through). Used
/// by the roving-coverage overlay to draw each street segment thicker and hotter the
/// more often a camera vehicle has crossed it. `segments[i]` owns verts `[4i, 4i+3]`.
pub fn colored_ribbon_mesh(
    segments: &[[Enu; 2]],
    halfs: &[f32],
    colors: &[[f32; 4]],
    z: f32,
) -> Mesh {
    let n = segments.len();
    let mut verts: Vec<[f32; 3]> = Vec::with_capacity(n * 4);
    let mut vcol: Vec<[f32; 4]> = Vec::with_capacity(n * 4);
    let mut indices: Vec<u32> = Vec::with_capacity(n * 6);
    for (i, [a, b]) in segments.iter().enumerate() {
        let half = halfs[i];
        let (ax, ay, bx, by) = (a.x as f32, a.y as f32, b.x as f32, b.y as f32);
        let (dx, dy) = (bx - ax, by - ay);
        let len = (dx * dx + dy * dy).sqrt();
        let inv = if len < 1e-6 { 0.0 } else { half / len };
        let (nx, ny) = (-dy * inv, dx * inv);
        verts.push([ax + nx, ay + ny, z]);
        verts.push([bx + nx, by + ny, z]);
        verts.push([bx - nx, by - ny, z]);
        verts.push([ax - nx, ay - ny, z]);
        let c = colors[i];
        vcol.extend_from_slice(&[c, c, c, c]);
        let base = (i * 4) as u32;
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, verts);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vcol);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// A 45° hatch fill clipped to a set of polygon rings (ENU meters) — the paving
/// texture for the pedestrian plazas. For each ring we sweep parallel lines along
/// the (1,−1) normal at `spacing` and keep only the spans *inside* the polygon
/// (even–odd crossing pairs), emitted as thin ribbons. Returns one merged mesh.
pub fn hatch_lines_mesh(polys: &[Vec<[f32; 2]>], spacing: f32, half: f32, z: f32) -> Mesh {
    let inv = std::f32::consts::FRAC_1_SQRT_2;
    let d = [inv, inv]; // along the hatch line
    let n = [inv, -inv]; // across (orthonormal to d)
    let mut segs: Vec<[Enu; 2]> = Vec::new();
    for ring in polys {
        let m = ring.len();
        if m < 3 {
            continue;
        }
        let (mut cmin, mut cmax) = (f32::MAX, f32::MIN);
        for p in ring {
            let c = p[0] * n[0] + p[1] * n[1];
            cmin = cmin.min(c);
            cmax = cmax.max(c);
        }
        let mut c = (cmin / spacing).ceil() * spacing;
        while c <= cmax {
            // Along-line params (t = p·d) where the line p·n = c crosses each edge.
            let mut ts: Vec<f32> = Vec::new();
            for i in 0..m {
                let a = ring[i];
                let b = ring[(i + 1) % m];
                let ca = a[0] * n[0] + a[1] * n[1] - c;
                let cb = b[0] * n[0] + b[1] * n[1] - c;
                if (ca > 0.0) != (cb > 0.0) {
                    let f = ca / (ca - cb);
                    let x = a[0] + (b[0] - a[0]) * f;
                    let y = a[1] + (b[1] - a[1]) * f;
                    ts.push(x * d[0] + y * d[1]);
                }
            }
            ts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let mut i = 0;
            while i + 1 < ts.len() {
                let (t0, t1) = (ts[i], ts[i + 1]);
                let p0 = Enu::new((c * n[0] + t0 * d[0]) as f64, (c * n[1] + t0 * d[1]) as f64);
                let p1 = Enu::new((c * n[0] + t1 * d[0]) as f64, (c * n[1] + t1 * d[1]) as f64);
                segs.push([p0, p1]);
                i += 2;
            }
            c += spacing;
        }
    }
    thick_line_list_mesh(&segs, half, z)
}

/// Closest point on segment `a`–`b` to `p` (all in world meters).
fn closest_on_segment(p: Vec2, a: Vec2, b: Vec2) -> Vec2 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 < 1e-6 {
        return a;
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    a + ab * t
}

/// Nearest point on a closed ring (polyline, ENU meters) to `p`. Brute force over the
/// ring's segments — fine for the handful of label anchors we project to the coastline.
pub fn nearest_on_ring(p: Vec2, ring: &[[f64; 2]]) -> Vec2 {
    let n = ring.len();
    if n == 0 {
        return p;
    }
    let mut best = p;
    let mut best_d2 = f32::INFINITY;
    for i in 0..n {
        let a = Vec2::new(ring[i][0] as f32, ring[i][1] as f32);
        let b = Vec2::new(ring[(i + 1) % n][0] as f32, ring[(i + 1) % n][1] as f32);
        let q = closest_on_segment(p, a, b);
        let d2 = p.distance_squared(q);
        if d2 < best_d2 {
            best_d2 = d2;
            best = q;
        }
    }
    best
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

/// A constant-width stroked outline for a closed ENU ring — a thin filled band
/// centered on the ring, for framing the map (e.g. the Manhattan coastline around
/// the street network). `half_width` is in world meters. Each vertex is offset by
/// the averaged adjacent-edge normal (no miter-length scaling, so sharp corners
/// only under-fill slightly — never spike). Returns `None` if the ring is
/// degenerate. TriangleList + `ColorMaterial`, same as the choropleth/wedge meshes.
pub fn stroke_ring_mesh(ring: &[[f64; 2]], half_width: f32, z: f32) -> Option<Mesh> {
    // Drop a duplicated closing vertex if present.
    let ring = if ring.len() >= 2 && ring.first() == ring.last() {
        &ring[..ring.len() - 1]
    } else {
        ring
    };
    let n = ring.len();
    if n < 3 {
        return None;
    }
    let pt = |i: usize| Vec2::new(ring[i][0] as f32, ring[i][1] as f32);
    let mut verts: Vec<[f32; 3]> = Vec::with_capacity(n * 2);
    for i in 0..n {
        let prev = pt((i + n - 1) % n);
        let cur = pt(i);
        let next = pt((i + 1) % n);
        // Left normal of each incident edge ((-dy, dx)); average → vertex normal.
        let e0 = cur - prev;
        let e1 = next - cur;
        let n0 = Vec2::new(-e0.y, e0.x).normalize_or_zero();
        let n1 = Vec2::new(-e1.y, e1.x).normalize_or_zero();
        let mut nrm = (n0 + n1).normalize_or_zero();
        if nrm == Vec2::ZERO {
            nrm = n1; // 180° spike / coincident points — fall back to one edge
        }
        let off = nrm * half_width;
        verts.push([cur.x + off.x, cur.y + off.y, z]); // outer
        verts.push([cur.x - off.x, cur.y - off.y, z]); // inner
    }
    let mut indices: Vec<u32> = Vec::with_capacity(n * 6);
    for i in 0..n {
        let (a, b) = (2 * i as u32, 2 * i as u32 + 1);
        let j = (i + 1) % n;
        let (c, d) = (2 * j as u32, 2 * j as u32 + 1);
        // quad (outer_i, inner_i, inner_j, outer_j) → two triangles
        indices.extend_from_slice(&[a, b, d, a, d, c]);
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, verts);
    mesh.insert_indices(Indices::U32(indices));
    Some(mesh)
}

/// Merge many ENU exterior rings into one filled `TriangleList` (ear-clip each) —
/// the building-footprint ground fabric as a single mesh/draw-call.
pub fn merged_footprints_mesh(polys: &[Vec<[f32; 2]>]) -> Mesh {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for ring in polys {
        let r: &[[f32; 2]] = if ring.len() >= 2 && ring.first() == ring.last() {
            &ring[..ring.len() - 1]
        } else {
            ring
        };
        if r.len() < 3 {
            continue;
        }
        let flat: Vec<f64> = r.iter().flat_map(|p| [p[0] as f64, p[1] as f64]).collect();
        let Ok(tri) = earcutr::earcut(&flat, &[], 2) else { continue };
        if tri.is_empty() {
            continue;
        }
        let base = positions.len() as u32;
        for p in r {
            positions.push([p[0], p[1], 0.0]);
        }
        for &i in &tri {
            indices.push(base + i as u32);
        }
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Flatten landmark surfaces into one 2D filled mesh — every surface projected to the
/// ground plane (height dropped). Vertical faces (walls, draped cables, truss webs)
/// collapse to ~zero projected area and drop out, leaving the horizontal caps (deck
/// slab, tower/pier tops). Rendered flat with a plain light material, so bridges read
/// as a quiet footprint-style crossing — like the building fabric — rather than bold
/// 3D massing that occludes the agents driving over them.
pub fn landmark_flat_mesh(landmarks: &[Landmark]) -> Mesh {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for lm in landmarks {
        for s in &lm.surfaces {
            let n = s.verts.len();
            if n < 3 {
                continue;
            }
            // shoelace area of the ground projection; skip near-vertical/degenerate
            // faces (a wall/cable/truss ribbon collapses to a sliver < ~1 m²).
            let mut area2 = 0.0f64;
            for i in 0..n {
                let a = s.verts[i];
                let b = s.verts[(i + 1) % n];
                area2 += a[0] as f64 * b[1] as f64 - b[0] as f64 * a[1] as f64;
            }
            if area2.abs() < 2.0 {
                continue;
            }
            let flat: Vec<f64> = s.verts.iter().flat_map(|v| [v[0] as f64, v[1] as f64]).collect();
            let Ok(tri) = earcutr::earcut(&flat, &[], 2) else { continue };
            if tri.is_empty() {
                continue;
            }
            let base = positions.len() as u32;
            for v in &s.verts {
                positions.push([v[0], v[1], 0.0]);
            }
            for &i in &tri {
                indices.push(base + i as u32);
            }
        }
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Outward 3D normal of a landmark surface, from its first non-degenerate triangle.
/// `None` if degenerate.
fn surface_normal3(verts: &[[f32; 3]]) -> Option<Vec3> {
    let v0 = Vec3::from_array(verts[0]);
    for i in 1..verts.len().saturating_sub(1) {
        let n = (Vec3::from_array(verts[i]) - v0).cross(Vec3::from_array(verts[i + 1]) - v0);
        if n.length_squared() > 1e-6 {
            return Some(n.normalize());
        }
    }
    None
}

/// A neutral-zinc gray (dark→light by `b ∈ [0,1]`) as **linear** RGBA for a vertex
/// color — so a white `ColorMaterial` (which multiplies by the vertex color) shows
/// it identically to an equivalent per-material sRGB color.
fn gray_linear(b: f32) -> [f32; 4] {
    // Paper theme: landmark massings read as gray ink on white — shadowed faces dark,
    // lit faces medium (never white, so they stay visible against the paper ground).
    let lo = 0x2e as f32 / 255.0;
    let hi = 0xaa as f32 / 255.0;
    let g = lo + (hi - lo) * b.clamp(0.0, 1.0);
    let c = Color::srgb(g, g, g).to_linear();
    [c.red, c.green, c.blue, 1.0]
}

/// Build the landmark massing as **one** per-face-shaded, depth-sorted mesh.
/// Height extrudes **straight up** the screen (`height` world-m per m, no sideways
/// skew, so every building stands vertically); north-facing walls are culled; each
/// face is flat-shaded by a 3D Lambert term against `light` (revealing facets,
/// setbacks, domes); triangles are emitted back-to-front (walls, then roofs on top)
/// for correct self-occlusion. Per-vertex colors ride `Mesh::ATTRIBUTE_COLOR`, so it
/// renders with a plain white `ColorMaterial`.
pub fn landmark_massing_mesh(landmarks: &[Landmark], height: f32, light: Vec3) -> Mesh {
    const AMBIENT: f32 = 0.30;
    const DIFFUSE: f32 = 0.78;
    const CULL_NY: f32 = 0.18; // cull walls whose normal points clearly north
    let light = light.normalize_or_zero();

    struct Face {
        roof: bool,
        depth: f32, // projected-y centroid; higher = farther back
        verts: Vec<[f32; 2]>,
        tris: Vec<usize>,
        color: [f32; 4],
    }
    let mut faces: Vec<Face> = Vec::new();
    for lm in landmarks {
        for s in &lm.surfaces {
            let roof = s.kind == 1;
            let Some(n) = surface_normal3(&s.verts) else { continue };
            if !roof && n.y > CULL_NY {
                continue; // back (north) wall
            }
            let color = gray_linear(AMBIENT + DIFFUSE * n.dot(light).max(0.0));
            // Pure-vertical projection: height adds only to screen-up (world +y).
            let verts: Vec<[f32; 2]> = s.verts.iter().map(|v| [v[0], v[1] + height * v[2]]).collect();
            let flat: Vec<f64> = verts.iter().flat_map(|p| [p[0] as f64, p[1] as f64]).collect();
            let Ok(tris) = earcutr::earcut(&flat, &[], 2) else { continue };
            if tris.is_empty() {
                continue;
            }
            // Painter depth = ground-plane north–south position (the un-extruded ENU y),
            // NOT the height-extruded screen-y. Mixing height in made a tall *near*
            // (south) wall sort as if far back, so dark far faces drew over it — the
            // "shadows clip through the building" artifact. North = larger y = farther.
            let depth = s.verts.iter().map(|v| v[1]).sum::<f32>() / s.verts.len() as f32;
            faces.push(Face { roof, depth, verts, tris, color });
        }
    }
    // Painter order: walls first, roofs last (on top); within each, back (high y) → front.
    faces.sort_by(|a, b| {
        a.roof
            .cmp(&b.roof)
            .then(b.depth.partial_cmp(&a.depth).unwrap_or(std::cmp::Ordering::Equal))
    });
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for f in &faces {
        let base = positions.len() as u32;
        for p in &f.verts {
            positions.push([p[0], p[1], 0.0]);
            colors.push(f.color);
        }
        for &i in &f.tris {
            indices.push(base + i as u32);
        }
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
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
    fn ribbon_offsets_perpendicular_by_half_width() {
        // A horizontal segment along +x: corners should sit ±half in y at the given z.
        let seg = [[Enu::new(0.0, 0.0), Enu::new(10.0, 0.0)]];
        let v = ribbon_positions(&seg, 3.0, 0.2);
        assert_eq!(v.len(), 4, "one quad per segment");
        // TL, TR on the +y side; BR, BL on the -y side; all at z = 0.2.
        assert!((v[0][1] - 3.0).abs() < 1e-5 && (v[1][1] - 3.0).abs() < 1e-5);
        assert!((v[2][1] + 3.0).abs() < 1e-5 && (v[3][1] + 3.0).abs() < 1e-5);
        assert!(v.iter().all(|p| (p[2] - 0.2).abs() < 1e-5));
        // thick_line_list_mesh emits 6 indices (2 tris) per segment.
        let m = thick_line_list_mesh(&seg, 3.0, 0.2);
        assert_eq!(m.indices().map(|i| i.len()), Some(6));
    }

    #[test]
    fn hatch_fills_a_square_and_skips_degenerate() {
        // A 100 m square: the 45° sweep at 10 m spacing crosses it ~15 times, each a
        // single inside span → one quad (4 verts). Allow slack for corner-touching lines.
        let square = vec![vec![[0.0f32, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]]];
        let m = hatch_lines_mesh(&square, 10.0, 0.4, 0.3);
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(pos)) =
            m.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("hatch mesh has positions");
        };
        assert!(pos.len() % 4 == 0, "one quad (4 verts) per clipped span");
        let quads = pos.len() / 4;
        assert!((12..=16).contains(&quads), "≈15 hatch lines across a 100 m square, got {quads}");
        assert!(pos.iter().all(|p| (p[2] - 0.3).abs() < 1e-5), "all at the given z");
        // A sub-spacing polygon yields no full hatch line.
        let tiny = vec![vec![[0.0f32, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]]];
        let mt = hatch_lines_mesh(&tiny, 10.0, 0.4, 0.3);
        let n = mt
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .map(|a| a.len())
            .unwrap_or(0);
        assert!(n <= 4, "a 2 m square catches at most one hatch line");
    }

    #[test]
    fn landmark_massing_builds_mesh() {
        use sim_core::assets::{Landmark, LandmarkSurface};
        // A 10×10×10 box: 4 walls + 1 roof.
        let wall = |a: [f32; 3], b: [f32; 3]| LandmarkSurface {
            kind: 0,
            verts: vec![a, b, [b[0], b[1], 10.0], [a[0], a[1], 10.0]],
        };
        let lm = Landmark {
            name: "Box".into(),
            anchor: [5.0, 5.0],
            height_m: 10.0,
            surfaces: vec![
                wall([0.0, 0.0, 0.0], [10.0, 0.0, 0.0]),   // south
                wall([10.0, 0.0, 0.0], [10.0, 10.0, 0.0]), // east
                wall([10.0, 10.0, 0.0], [0.0, 10.0, 0.0]), // north (culled)
                wall([0.0, 10.0, 0.0], [0.0, 0.0, 0.0]),   // west
                LandmarkSurface {
                    kind: 1,
                    verts: vec![[0.0, 0.0, 10.0], [10.0, 0.0, 10.0], [10.0, 10.0, 10.0], [0.0, 10.0, 10.0]],
                },
            ],
        };
        let mesh = landmark_massing_mesh(&[lm], 0.5, Vec3::new(0.38, -0.48, 0.79));
        // South wall + roof survive (north culled; the perfectly N–S east/west walls
        // project to zero-width lines under pure-vertical extrusion and drop out — a
        // non-issue for the rotated Manhattan grid). Non-empty + per-vertex colors.
        assert!(mesh.count_vertices() >= 8, "south wall + roof survive");
        assert!(mesh.attribute(Mesh::ATTRIBUTE_COLOR).is_some(), "has vertex colors");
    }

    #[test]
    fn stroke_ring_builds_band() {
        // A unit square ring → 4 vertices, 2 per vertex = 8, and 6 indices/segment.
        let ring = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.0]];
        let mesh = stroke_ring_mesh(&ring, 1.0, 0.1).expect("square strokes");
        assert_eq!(mesh.count_vertices(), 8); // closing vertex dropped → 4 ring pts × 2
        // Degenerate ring → None.
        assert!(stroke_ring_mesh(&[[0.0, 0.0], [1.0, 1.0]], 1.0, 0.0).is_none());
    }

    #[test]
    fn polygon_triangulates() {
        let ring = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]];
        let mesh = filled_polygon_mesh(&ring, 0.0).expect("square triangulates");
        assert_eq!(mesh.count_vertices(), 4); // closing vertex dropped
    }

    #[test]
    fn landmark_flat_keeps_only_horizontal_caps() {
        use sim_core::assets::{Landmark, LandmarkSurface};
        // 10×10×10 box: 4 vertical walls + a vertical "cable" ribbon (tagged cap) + the
        // horizontal roof. The flat footprint must keep ONLY the roof — every vertical
        // face collapses to ~zero projected area and drops out.
        let wall = |a: [f32; 3], b: [f32; 3]| LandmarkSurface {
            kind: 0,
            verts: vec![a, b, [b[0], b[1], 10.0], [a[0], a[1], 10.0]],
        };
        let lm = Landmark {
            name: "Box".into(),
            anchor: [5.0, 5.0],
            height_m: 10.0,
            surfaces: vec![
                wall([0.0, 0.0, 0.0], [10.0, 0.0, 0.0]),
                wall([10.0, 0.0, 0.0], [10.0, 10.0, 0.0]),
                wall([10.0, 10.0, 0.0], [0.0, 10.0, 0.0]),
                wall([0.0, 10.0, 0.0], [0.0, 0.0, 0.0]),
                // a vertical ribbon tagged as a cap (like a draped cable) — still drops.
                LandmarkSurface {
                    kind: 1,
                    verts: vec![[2.0, 2.0, 5.0], [8.0, 8.0, 5.0], [8.0, 8.0, 30.0], [2.0, 2.0, 30.0]],
                },
                // the horizontal roof cap → the only surviving footprint.
                LandmarkSurface {
                    kind: 1,
                    verts: vec![[0.0, 0.0, 10.0], [10.0, 0.0, 10.0], [10.0, 10.0, 10.0], [0.0, 10.0, 10.0]],
                },
            ],
        };
        let mesh = landmark_flat_mesh(&[lm]);
        assert_eq!(mesh.count_vertices(), 4, "only the horizontal roof footprint remains");
        assert!(
            mesh.attribute(Mesh::ATTRIBUTE_COLOR).is_none(),
            "flat fill is plain (colored by the material, like the footprints)"
        );
    }
}
