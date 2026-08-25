---
title: Open Work Before Submission
created: 2026-08-23
updated: 2026-08-23
type: deadline
tags: [deadline, facct27]
sources: [facct27_digital_eyes_on_the_street/OUTLINE.md §9, §13, docs/TODO.md]
confidence: high
---

# Open Work Before Submission

Ordered by the OUTLINE's writing plan (§13); items 1 of 5 sessions are done
(P0 fixed). With ~9 weeks to the abstract deadline:

## Methods (largest open refinement)
- Model unknown camera headings as an expectation-over-heading (arc-union)
  primary treatment; the ~12% CCTV over-count is already disclosed.
  Targeted before camera-ready at the latest.

## Writing (numbers exist for all computed sections)
- §4 Instrument first ([[exposure-instrument]]), including the occlusion
  sensitivity table and [[point-vs-path]] spine.
- §6.1 + §6.2 ([[results-placement-bias]], [[results-laundering-ladder]]);
  §6.3 ([[results-equalization]]) with Gini/top-decile numbers.
- §1 + §2 + §7 + §8 last; intro once results have told you what the paper says.

## Figures
- F1 teaser and F2 schematic need making; F4 is the money figure
  ([[figures-and-tables]]).

## Hygiene before the artifact deadline
- Commit `data/derived/` or a manifest + hashes.
- `spatial_econometrics.py` and `undercount_spatial.py` still stdout-only;
  persist JSON like the rest.
- Income-free spatial-model robustness variant (holes in the Queen-contiguity
  graph at N=5,547).
- Drop `poverty_rate` (100% empty) from the pre-reg control list.
- Verify ACE fleet counts by hand (mta.info blocks fetching).

Related: [[facct-27-deadlines]] · [[compounding-test-m1-m3]] · [[results-robustness]]
