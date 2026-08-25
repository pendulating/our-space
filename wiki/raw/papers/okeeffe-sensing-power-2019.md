---
source_url: https://www.pnas.org/doi/10.1073/pnas.1821667116
ingested: 2026-08-23
sha256: pending-reingest
---

# Quantifying the sensing power of vehicle fleets

Kevin P. O'Keeffe, Amin Anjomshoaa, Steven H. Strogatz, Paolo Santi, Carlo Ratti.
*PNAS* (2019). doi:10.1073/pnas.1821667116. Senseable City Lab, MIT.

## Summary

Drive-by sensing: sensors mounted on third-party vehicles scan the city. Question:
how many vehicles to adequately scan a city? Model: the **taxi-drive process** —
taxis travel to randomly chosen destinations via shortest paths, destinations
chosen with **preferential return** `q_n ∝ 1 + ε·v_n` (captures human-mobility
statistics; Song et al. 2010 lineage). Sensing power = covering fraction ⟨C⟩,
the fraction of street segments sensed ≥ once in period T.

Key results:
- Segment popularities p_i are heavy-tailed, follow **Zipf's law**; taxi-drive
  reproduces them, a random-walk null fails qualitatively (Fig. 2D/E).
- Ball-in-bin analytic (Eq. 11):
  `⟨C⟩ ≈ 1 − (1/N_S) Σ_{i=1..N_S} (1 − p_i)^{B̄·N_V}` — agrees with data from
  nine cities / ten datasets.
- **Universal scaling collapse**: ⟨C⟩ vs N_V/B̄ collapses across cities with no
  adjustable parameters (Fig. 4).
- Saturation is fast: ~5% of vehicles cover ~50% of segments; ~50% of vehicles
  for 80%. **10 random taxis cover one-third of Manhattan's street segments daily.**
- Temporal extension ⟨C*⟩(N_w) (≥1 scan in each of N_w subintervals): at N_w = 3,
  N_V = 355 (~3%) still scans half of Manhattan.
- **Spatial-bias caveat (Discussion)**: taxis concentrate in commercial/tourist
  areas → inherent spatial bias "could have harmful consequences, such as
  underservicing socioeconomically disadvantaged neighborhoods"; deferred to SI;
  hybrid dedicated-fleet remedy proposed.
- Host taxonomy: random-destination hosts high sensing power; fixed-route hosts
  (buses, trash trucks) low. Privacy remark: sensors on private cars "might lead
  to privacy concerns" — city-owned fleets suggested instead.
- B̄ estimates are lower bounds → quoted N*_V values likely upper... (their note:
  N*_V likely *lower*, i.e., saturation even easier).
- Extensions proposed: weighted cover metric C = Σ b_i·1(M_i ≥ 1); inferring
  sensing power from network structure alone.

## Relevance to our-space

The most important ancestor. They compute supply-side **coverage** and frame it as
a public good; we invert to **exposure** with distributional incidence. Full
integration points live in [[simulation-risks]] §B (B1–B7): analytic cross-check,
Zipf diagnostic, preferential-return prior, temporal-resolution parallel, their
spatial-bias caveat as our motivation, diminishing returns as the equalization
mechanism, host taxonomy as fleet contrast.

Related: [[okeeffe-sensing-power]] · [[results-equalization]] · [[compounding-test-m1-m3]]
