#!/usr/bin/env python3
"""§7 — what the camera-census undercount does to the disparity, and what it does NOT do.

The street-view CCTV census finds only about half the cameras that are there
(`capture_recapture.py`: Chapman recall ~0.501, 95% [0.458, 0.544]). The obvious worry is that
this undercount MANUFACTURES the disparity. The obvious hope is that correcting for it AMPLIFIES
the disparity. This script shows that neither is true, and that a coefficient reported in
"cameras per SD" will mislead you into believing the second one.

HOW IT WORKS WITHOUT RE-RUNNING THE BATCH.
The correction inflates only the CCTV-census-only ("unconfirmed") sub-population — groups attested
by a *surveyed* census (DOT / ALPR / enforcement) are complete enumerations and are counted at face
value. So the correction is LINEAR in the recall r:

    R_i(r) = R_i_unconfirmed / r + (R_i - R_i_unconfirmed)

The batch emits both terms, so every recall value — including all 2,000 bootstrap draws from the
capture-recapture — is reconstructable here, arithmetically. (Same trick as
`sweep_subway_cameras.py`. It is pinned by a Rust unit test:
`recall_correction_is_linear_and_reconstructable_from_one_bake`.)

THE POINT.
Because detection is NON-DIFFERENTIAL (`undercount_spatial.py`: no demographic gradient — %Hisp
p=0.89, %Black p=0.89, income p=0.42), the correction is a near-uniform rescaling. A near-uniform
rescaling cannot create or destroy a *relative* disparity. So:

  - the coefficient in cameras/SD roughly DOUBLES  -- but so does the outcome's SD. Mechanical.
  - the scale-free correlation barely moves        -- that is the real answer.

Reporting only the first is a units artifact dressed up as a finding.

Writes data/derived/results/recall_sensitivity.json
"""
import csv, json, math, os

import numpy as np

TABLE = "data/derived/exposure/exposure_table_nyc.csv"
CR = "data/derived/results/capture_recapture.json"
OUTDIR = "data/derived/results"

DEMOS = [("%Hispanic", "pct_hispanic"), ("%Black", "pct_black_nh"),
         ("%White", "pct_white_nh"), ("income", "median_hh_income")]


def fnum(d, k):
    v = d.get(k)
    try:
        return float(v) if v not in (None, "") else None
    except ValueError:
        return None


rows = []
with open(TABLE) as f:
    for r in csv.DictReader(f):
        pop, raw, unc = fnum(r, "population"), fnum(r, "R_i"), fnum(r, "R_i_unconfirmed")
        if not pop or pop <= 0 or raw is None or unc is None:
            continue
        rows.append({"pop": pop, "raw": raw, "unc": unc,
                     **{k: fnum(r, k) for _, k in DEMOS}})

W = np.array([r["pop"] for r in rows], float)
W = W / W.mean()
RAW = np.array([r["raw"] for r in rows], float)
UNC = np.array([r["unc"] for r in rows], float)


def R_at(r):
    """The recall-corrected exposure at recall r — pure arithmetic, no batch re-run."""
    return UNC / r + (RAW - UNC)


def disparity(y, xkey):
    """Population-weighted coefficient (cameras/SD of x) AND the scale-free correlation."""
    x = np.array([r[xkey] if r[xkey] is not None else np.nan for r in rows], float)
    m = ~np.isnan(x)
    xx, yy, ww = x[m], y[m], W[m]
    mx = np.average(xx, weights=ww)
    sx = math.sqrt(np.average((xx - mx) ** 2, weights=ww))
    my = np.average(yy, weights=ww)
    sy = math.sqrt(np.average((yy - my) ** 2, weights=ww))
    zx, zy = (xx - mx) / sx, (yy - my) / sy
    corr = float(np.average(zx * zy, weights=ww))
    return {"coef_cameras_per_sd": corr * sy, "corr": corr, "outcome_mean": my, "outcome_sd": sy}


# ---- 1. the sweep over recall ------------------------------------------------------------
GRID = [1.0, 0.90, 0.80, 0.70, 0.63, 0.544, 0.501, 0.458, 0.40, 0.30]
out = {"grid": {}, "note": "r=1.0 is the observed (uncorrected) count. 0.501 = Chapman estimate."}

print(f"\nExposure and disparity vs. assumed census recall r   (N={len(rows)} BGs, pop-weighted)\n")
print(f"{'recall r':>9}{'mean R_i':>10}{'SD':>8}   |"
      + "".join(f"{n:>21}" for n, _ in DEMOS))
print(f"{'':>27}   |" + "".join(f"{'cam/SD':>11}{'corr':>10}" for _ in DEMOS))
print("-" * 115)
for r in GRID:
    y = R_at(r)
    cells, rec = "", {}
    for name, key in DEMOS:
        d = disparity(y, key)
        rec[name] = d
        cells += f"{d['coef_cameras_per_sd']:>+11.2f}{d['corr']:>+10.3f}"
    mean_y = float(np.average(y, weights=W))
    sd_y = float(math.sqrt(np.average((y - mean_y) ** 2, weights=W)))
    out["grid"][f"{r:.3f}"] = {"mean_R": mean_y, "sd_R": sd_y, "demographics": rec}
    star = " <- observed" if r == 1.0 else (" <- Chapman" if r == 0.501 else "")
    print(f"{r:>9.3f}{mean_y:>10.1f}{sd_y:>8.1f}   |{cells}{star}")

# ---- 2. the decomposition: how much of the "doubling" is real? ---------------------------
y1, y2 = R_at(1.0), R_at(0.501)
print("\n" + "=" * 115)
print('DECOMPOSING THE APPARENT AMPLIFICATION (observed r=1.0  ->  Chapman r=0.501)\n')
print(f"  {'group':<12}{'coef cam/SD':>22}{'ratio':>8}   {'correlation':>20}{'ratio':>8}   {'GENUINE change':>16}")
print("  " + "-" * 100)
out["decomposition"] = {}
for name, key in DEMOS:
    a, b = disparity(y1, key), disparity(y2, key)
    coef_ratio = b["coef_cameras_per_sd"] / a["coef_cameras_per_sd"]
    sd_ratio = b["outcome_sd"] / a["outcome_sd"]
    genuine = coef_ratio / sd_ratio          # == corr ratio; the scale-free part
    out["decomposition"][name] = {
        "coef_raw": a["coef_cameras_per_sd"], "coef_corrected": b["coef_cameras_per_sd"],
        "coef_ratio": coef_ratio, "outcome_sd_ratio": sd_ratio,
        "corr_raw": a["corr"], "corr_corrected": b["corr"], "genuine_ratio": genuine,
    }
    print(f"  {name:<12}{a['coef_cameras_per_sd']:>+10.2f} ->{b['coef_cameras_per_sd']:>+9.2f}"
          f"{coef_ratio:>8.2f}x   {a['corr']:>+9.3f} ->{b['corr']:>+8.3f}"
          f"{b['corr']/a['corr']:>8.2f}x   {(genuine-1)*100:>+14.1f}%")
print(f"\n  The outcome's SD itself grows {disparity(y2,'pct_hispanic')['outcome_sd']/disparity(y1,'pct_hispanic')['outcome_sd']:.2f}x.")
print("  A coefficient in cameras/SD MUST grow with it. That growth is a unit change, not a finding.")
print("=" * 115)

# ---- 3. exact propagation of the recall CI through the coefficient -----------------------
if os.path.exists(CR):
    draws = json.load(open(CR)).get("recall_draws") or []
    if draws:
        print(f"\nPropagating the capture-recapture recall CI ({len(draws)} bootstrap draws) "
              f"through the disparity:\n")
        print(f"  {'group':<12}{'coef cam/SD (95%)':>30}{'correlation (95%)':>28}")
        print("  " + "-" * 68)
        out["bootstrap"] = {}
        for name, key in DEMOS:
            cs, rs = [], []
            for r in draws:
                d = disparity(R_at(r), key)
                cs.append(d["coef_cameras_per_sd"]); rs.append(d["corr"])
            cs, rs = np.sort(cs), np.sort(rs)
            lo, hi = cs[int(0.025 * len(cs))], cs[int(0.975 * len(cs))]
            rlo, rhi = rs[int(0.025 * len(rs))], rs[int(0.975 * len(rs))]
            out["bootstrap"][name] = {"coef_lo": float(lo), "coef_hi": float(hi),
                                      "corr_lo": float(rlo), "corr_hi": float(rhi)}
            print(f"  {name:<12}{f'[{lo:+.2f}, {hi:+.2f}]':>30}{f'[{rlo:+.3f}, {rhi:+.3f}]':>28}")
        print("\n  NOTE: the coefficient band is WIDE because recall rescales the outcome; the")
        print("  correlation band is TIGHT because the undercount is non-differential. Report both,")
        print("  and lead with the correlation — it is the one that answers the reviewer's question.")

os.makedirs(OUTDIR, exist_ok=True)
with open(os.path.join(OUTDIR, "recall_sensitivity.json"), "w") as f:
    json.dump(out, f, indent=2)
print(f"\nwrote {OUTDIR}/recall_sensitivity.json")
print("Recall is a CONSERVATIVE LOWER BOUND: both censuses derive from Street View, so their")
print("positive dependence inflates the overlap and biases the estimated recall UP.")
