---
title: Sensor Model
created: 2026-08-23
updated: 2026-08-23
type: concept
tags: [instrument, method]
sources: [facct27_digital_eyes_on_the_street/OUTLINE.md §4.1]
confidence: high
---

# Sensor Model

Every sensor is a **fixed frustum** or a **mobile intensity field**. This
abstraction lets one instrument admit both kinds of eye.

## Fixed
Camera *c* = wedge `{apex p_c, heading θ_c, half-FOV α_c, range R_c}`. It captures
point *x* iff `|x − p_c| ≤ R_c` ∧ `angle(x − p_c, θ_c) ≤ α_c` ∧ line-of-sight clear.
Defaults: 70° / 15 m (CCTV), 360° / 30 m (DOT monitoring).

## Mobile
Class *k* = space-time intensity field `λ_k(segment, t)`. Encounters are
independent rare events → Poisson: `E_k = ∫ λ_k(x(t), t) · p_capture · dt`,
`P(≥1) = 1 − e^{−E_k}`. Implemented in `sim-core::mobile`; used only by
`batch heatmap`, not yet by the block-group tables ([[compounding-test-m1-m3]]).

## Unknown headings
Amnesty records no heading column, so a bullet camera's direction is unrecoverable.
Only 15.5% of 28,954 cameras are typed at all; among typed cameras domes beat
bullets 5:1 (2,266 vs 444). The honest model is an expectation over unknown heading:
a uniform-random-bearing directional camera sees a point set `S` with probability
`measure(⋃_{p∈S}[bearing(p) ± θ/2]) / 360`. This is ≈ θ/360 ≈ 0.19 at a single
point but → 1 over a walkshed — [[point-vs-path]] again. Expected over-count of
CCTV coverage ≈ 12%; disclosed in the drafted results.

Related: [[camera-layers]] · [[occlusion-null]] · [[capture-recapture-undercount]]
