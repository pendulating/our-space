---
title: Civic Tool Ethics
created: 2026-08-23
updated: 2026-08-23
type: concept
tags: [ethics, privacy]
sources: [facct27_digital_eyes_on_the_street/OUTLINE.md §11]
confidence: high
---

# Ethics and Endmatter Positions

The paper's endmatter (+1 page) commits to:

- **Instrument framing.** An exposure estimator, never a surveillance map or an
  evasion guide. No individual-level claims (ecological, BG-level). The project
  explicitly refuses a "least-surveilled route" optimizer.
- **Client-side routes.** Routes stay in the browser, never transmitted or logged.
  One exception: address lookup sends the query text / pin coordinate to the
  key-free NYC GeoSearch API ([[web-build]]).
- **Adverse impacts.** A published exposure map could be misread as a policing-gap
  map. Mitigations: frame as placement audit; publish the counterfactual (what
  should be) alongside the actual; keep the camera layer at group-dedup resolution
  rather than device coordinates ([[cross-source-dedup]]).
- **Data licensing.** All sources open/licensed: Amnesty CC BY-NC-ND 4.0
  (non-commercial research use; the one NoDerivatives layer — handling rationale in
  docs/ARCHITECTURE.md), Dahir CC BY 4.0, LODES/ACS public domain, OSM ODbL.
  DOT cameras: coordinates only; images never read, fetched, stored, or
  redistributed ([[camera-layers]]).

Related: [[civic-tool]] · [[epistemic-tiers]] · [[paper-manuscript]]
