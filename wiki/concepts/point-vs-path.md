---
title: Point vs Path
created: 2026-08-23
updated: 2026-08-23
type: concept
tags: [method, instrument, claim]
sources: [facct27_digital_eyes_on_the_street/OUTLINE.md §9, docs/OCCLUSION_PLAN.md]
confidence: high
---

# Point vs Path

Place-based exposure measures are fragile to geometry; path-based ones are not.
Three independent measurements say the same thing:

1. **Occlusion** ([[occlusion-null]]) — building line-of-sight deletes 0.007% of
   `R_i`. Streets are the negative space of buildings. Sightlines run along the
   canyon.
2. **Unknown camera headings** ([[sensor-model]]) — a directional camera misses a
   single point with probability ≈ 330/360, but sees almost every point of a full
   walk because the walk subtends a wide arc from the camera.
3. **The headline** ([[results-equalization]]) — residence-based measures show a
   steep gradient; trajectory-based measures show saturation.

The distinction arrived as a debugging device — the only way to tell "occlusion
does nothing" apart from "occlusion is broken" — and became the cleanest statement
of the paper's thesis. Use it in §4 and again in §6.4.

Related: [[thesis-in-one-sentence]] · [[kwan-neap]] · [[measurement-paper-thesis]]
