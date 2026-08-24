#!/usr/bin/env python3
"""M2 — the compounding test (OUTLINE §8): does mobile surveillance fall on the
same people already carrying the most fixed-camera exposure?

The single statistic that settles the thesis:

    pop-weighted corr(R_i, M_i^act) across block groups

    corr > 0  =>  mobile surveillance COMPOUNDS the fixed-camera disparity — it
                  falls on the same people already most exposed.
    corr < 0  =>  mobile surveillance OFFSETS it — the fleets concentrate where
                  fixed cameras are thin.

Plus: the share of population in the top decile of BOTH distributions (the
"double burden" share; perfect independence predicts 1%). Spearman rank
correlation is reported alongside Pearson because both measures are heavily
right-skewed. All population-weighted, per house convention (a person is the
unit of moral concern, not a block group).

Interpretive guardrails baked into the output:
- M_i^act is day-averaged over waking hours and its dashcam penetration/capture
  are parameters (0.40/0.40). The correlation is scale-free, so those parameters
  CANNOT move it — only the spatial field can. State this in the paper.
- Blank M columns mean "bake predates M1", not zero. The script refuses to run
  on a pre-M1 table rather than silently reporting corr=nan.

Writes  data/derived/results/compounding.json
"""
import csv, json, os

import numpy as np

TABLE = "data/derived/exposure/exposure_table_nyc.csv"
OUT = "data/derived/results/compounding.json"


def wmean(x, w):
    return float(np.sum(x * w) / np.sum(w))


def wcov(x, y, w):
    mx, my = wmean(x, w), wmean(y, w)
    return float(np.sum(w * (x - mx) * (y - my)) / np.sum(w))


def wpearson(x, y, w):
    vx, vy = wcov(x, x, w), wcov(y, y, w)
    if vx <= 0 or vy <= 0:
        return float("nan")
    return wcov(x, y, w) / np.sqrt(vx * vy)


def wrankdata(v, w):
    """Population-weighted mid-ranks: sort by value, assign each BG the weight-share
    of everyone strictly below plus half its own tie group. Returns ranks in [0,1]."""
    order = np.argsort(v, kind="stable")
    ranks = np.empty(len(v))
    i = 0
    cum = 0.0
    total = np.sum(w)
    while i < len(order):
        j = i
        while j + 1 < len(order) and v[order[j + 1]] == v[order[i]]:
            j += 1
        grp_w = np.sum(w[order[i : j + 1]])
        mid = cum + grp_w / 2.0
        for k in range(i, j + 1):
            ranks[order[k]] = mid / total
        cum += grp_w
        i = j + 1
    return ranks


def top_decile_overlap(r_mask, m_mask, w):
    return float(np.sum(w * (r_mask & m_mask)) / np.sum(w))


def main():
    rows = list(csv.DictReader(open(TABLE)))
    need = {"population", "R_i", "M_ace_act_mnl", "M_dash_act_mnl"}
    have = set(rows[0].keys()) if rows else set()
    missing = need - have
    if missing:
        raise SystemExit(
            f"{TABLE} lacks {sorted(missing)} — re-run `batch exposure-table` after an "
            "M1 bake (od-exposure-mnl/modal with m_ace_act/m_dash_act columns). "
            "Blank-vs-zero ambiguity is not acceptable for a headline statistic."
        )

    # Rows with everything present. M columns blank => pre-M1 source files.
    keep = []
    for r in rows:
        try:
            pop = float(r["population"])
            ri = float(r["R_i"])
            ace = r["M_ace_act_mnl"]
            dash = r["M_dash_act_mnl"]
            if pop <= 0 or ace == "" or dash == "":
                continue
            keep.append((pop, ri, float(ace), float(dash), r))
        except (ValueError, KeyError):
            continue
    if len(keep) < 100:
        raise SystemExit(f"only {len(keep)} usable rows — table looks pre-M1; refusing")

    pop = np.array([k[0] for k in keep])
    ri = np.array([k[1] for k in keep])
    m_act = np.array([k[2] + k[3] for k in keep])  # total mobile activity-space rate
    m_ace = np.array([k[2] for k in keep])
    m_dash = np.array([k[3] for k in keep])

    out = {
        "_readme": {
            "statistic": "pop-weighted corr(R_i, M_i^act); >0 compounds, <0 offsets",
            "m_act_definition": "M_ace_act_mnl + M_dash_act_mnl (expected encounters per commute traversal)",
            "n_bgs": int(len(keep)),
            "guardrails": [
                "correlation is scale-free: dashcam penetration/capture parameters cannot move it",
                "M_i^act is day-averaged over waking hours (see batch MobileLayers::day_weights)",
                "population-weighted throughout",
            ],
        },
        "n_bgs": int(len(keep)),
        "total_pop": float(np.sum(pop)),
    }

    # Headline correlations.
    out["corr_R_vs_M_act_popwtd"] = wpearson(ri, m_act, pop)
    out["corr_R_vs_M_act_unweighted"] = wpearson(ri, m_act, np.ones_like(ri))
    # Rank version (robust to skew).
    rr, mr = wrankdata(ri, pop), wrankdata(m_act, pop)
    out["spearman_R_vs_M_act_popwtd"] = wpearson(rr, mr, pop)
    # Per-class.
    out["corr_R_vs_M_ace_popwtd"] = wpearson(ri, m_ace, pop)
    out["corr_R_vs_M_dash_popwtd"] = wpearson(ri, m_dash, pop)
    # Cross-mobile: do the two fleets overlap each other?
    out["corr_M_ace_vs_M_dash_popwtd"] = wpearson(m_ace, m_dash, pop)

    # Double-burden share: top decile of R_i AND top decile of M_i^act (BG-level deciles;
    # report the pop-weighted share of people living in double-decile BGs).
    r_cut = np.quantile(ri, 0.9)
    m_cut = np.quantile(m_act, 0.9)
    r_top, m_top = ri >= r_cut, m_act >= m_cut
    out["share_pop_top_decile_R"] = float(np.sum(pop * r_top) / np.sum(pop))
    out["share_pop_top_decile_M"] = float(np.sum(pop * m_top) / np.sum(pop))
    out["share_pop_top_decile_both"] = top_decile_overlap(r_top, m_top, pop)
    out["share_pop_top_decile_both_if_independent"] = 0.01

    # Quintile means (mirrors inequality_stats.py's presentation).
    order = np.argsort(ri, kind="stable")
    qpop = np.cumsum(pop[order]) / np.sum(pop)
    quint = np.searchsorted(qpop, [0.2, 0.4, 0.6, 0.8])
    bounds = [0, *quint, len(order)]
    qtab = []
    for q in range(5):
        sel = order[bounds[q] : bounds[q + 1]]
        qtab.append(
            {
                "quintile_of_R": q + 1,
                "pop_share": float(np.sum(pop[sel]) / np.sum(pop)),
                "mean_R": wmean(ri[sel], pop[sel]),
                "mean_M_act": wmean(m_act[sel], pop[sel]),
            }
        )
    out["by_R_quintile"] = qtab

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w") as f:
        json.dump(out, f, indent=2)

    c = out["corr_R_vs_M_act_popwtd"]
    verdict = "COMPOUNDS" if c > 0 else "OFFSETS"
    print(f"M2 compounding test  (N={len(keep)} BGs, pop-wtd)")
    print(f"  corr(R_i, M_i^act) = {c:+.4f}   -> mobile surveillance {verdict} the fixed disparity")
    print(f"  Spearman           = {out['spearman_R_vs_M_act_popwtd']:+.4f}")
    print(f"  per-class: ACE {out['corr_R_vs_M_ace_popwtd']:+.4f}  DASH {out['corr_R_vs_M_dash_popwtd']:+.4f}")
    print(
        f"  double-decile share: {out['share_pop_top_decile_both']*100:.2f}% of people "
        f"(independence predicts 1.00%)"
    )
    print(f"  -> {OUT}")


if __name__ == "__main__":
    main()
