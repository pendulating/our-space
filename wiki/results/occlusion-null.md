---
title: Occlusion Null
created: 2026-08-23
updated: 2026-08-23
type: result
tags: [result, occlusion, instrument]
sources: [docs/OCCLUSION_PLAN.md, facct27_digital_eyes_on_the_street/OUTLINE.md §9]
confidence: high
---

# Occlusion Null

Real building line-of-sight is wired end-to-end: 1,063,421 footprints → 5.46M
walls, ~0.2 µs/query. One capture predicate exists in the codebase, so app and
paper cannot diverge.

**Finding:** buildings occlude essentially nothing that `R_i` measures.
- Level: `R_i` 82.023 → 82.018 (−0.007%), Pearson r = 1.0000.
- Disparity: Δβ ≤ 0.006 cameras/SD on every focal variable; two orders below the
  ±0.66 SE on the %Hispanic coefficient.
- Mechanism: streets are the negative space of buildings. Cameras are
  street-mounted; sample points are street vertices. Sightlines run along the
  canyon. Of touched cameras, 96% still see you elsewhere on the walk.

## Sensitivity (range sweep)
Attenuation of `R_i`: 0.007% at 15 m, 0.42% at 60 m, 5.5% at 120 m, 12.5% at 240 m.
Occlusion switches on only past ~60 m. The ×16 arm (63% point attenuation) proves
the wiring is live — a null at ×1 and 63% at ×16 cannot be a miswire.

**Paper framing:** this is evidence for the thesis. Occlusion is a first-order
correction for a place-based measure and a rounding error for a path-based one.
See [[point-vs-path]].

Related: [[sensor-model]] · [[results-placement-bias]] · [[exposure-instrument]]
