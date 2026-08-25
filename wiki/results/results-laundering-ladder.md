---
title: Results — Laundering Ladder
created: 2026-08-23
updated: 2026-08-23
type: result
tags: [result, econometrics]
sources: [facct27_digital_eyes_on_the_street/OUTLINE.md §6.2]
confidence: high
---

# The Laundering Ladder

Control ladder on `R_i`, pop-weighted WLS with HC1, cameras per SD. Re-run
2026-07-13 on the corrected sample: demographic ladders N=6,393, income N=5,547.

| Focal | 1: total | 2: +land use | 3: +crime | 4: +crime+311 | N |
|---|---|---|---|---|---|
| **%Hispanic** | **+10.15** | +6.62 | +4.06 | **+2.99 (±0.50)** | 6,393 |
| %White | −4.41 | −7.65 | −3.05 | −2.94 (±0.61) | 6,393 |
| %Black | −1.59 | +4.21 | −0.44 | +1.60 (±0.49) | 6,393 |
| income | −1.81 | −5.56 | −3.15 | −2.55 (±0.60) | 5,547 |

The mediators are themselves racialized:
`crime ~ %Hisp +0.31 / %White −0.29`; `311 ~ %Hisp +0.28 / %White −0.14`.

**Interpretation:** controlling for crime launders the disparity. The attenuation
(10.15 → 2.99) measures what the disorder narrative can rationalize, not what is
legitimate. Lead with the no-control number in the paper. Under Conley HAC spatial
inference, +3.88 ±0.60 survives rung 4 regardless.

This is T2 in the figure plan; see [[figures-and-tables]].

Related: [[results-placement-bias]] · [[spatial-econometrics]] · [[results-robustness]]
