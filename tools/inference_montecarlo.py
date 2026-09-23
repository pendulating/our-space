#!/usr/bin/env python3
"""Monte Carlo check of the SE estimators in activity_space_inference.py.

Holds the real design fixed (A_mnl sample, pop weights, z-scored %Hispanic as the regressor, real
BG locations, real NTA/PUMA clusters, real LODES destination shares) and simulates outcomes with a
TRUE coefficient of zero under four error structures:

  iid          e_i ~ N(0, s^2 / w_i)                          (what HC1 assumes)
  spatial      shock per 1 km grid cell + iid                 (local dependence: clusters/Conley)
  destination  sum_k p_ik u_k + iid, u_k iid per work BG      (shared-destination dependence:
               p = row-normalised LODES job shares             the structure of A_i itself)
  mixed        spatial + destination + iid

For each estimator it reports mean(SE) / SD(beta_hat) (1 = unbiased) and the rejection rate of
H0: beta = 0 at the 5% level (0.05 = correct size). The destination scenario is the stylized
version of A_i's dependence: A_i is 85% sum_k p_ik R_k, so a shock to one destination's camera
count moves every origin that commutes there.

Writes data/derived/results/inference_montecarlo.json.
"""
import csv, json, math, os, sys

import numpy as np
from scipy.sparse import csr_matrix

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import activity_space_inference as asi  # noqa: E402

N_SIM = int(os.environ.get("OURSPACE_MC_SIMS", "500"))
B_WCB = int(os.environ.get("OURSPACE_MC_WCB_B", "199"))
SEED = 20260923
PAIRS = os.environ.get("OURSPACE_PAIRS", "data/derived/exposure/od_pairs_mnl_nyc.csv")
SCENARIOS = {  # (sd of 1 km cell shock, sd of destination shock, sd of iid noise)
    "iid": (0.0, 0.0, 10.0),
    "spatial": (10.0, 0.0, 10.0),
    "destination": (0.0, 30.0, 1.0),
    "mixed": (10.0, 30.0, 10.0),
}


def main():
    sub, w = asi.sample(["A_mnl", "pct_hisp"] + asi.NEED_BASE)
    n = len(sub)
    geoids = [r["geoid"] for r in sub]
    x = np.array([r["pct_hisp"] for r in sub], float)
    m = np.average(x, weights=w)
    x = (x - m) / math.sqrt(np.average((x - m) ** 2, weights=w))
    X = np.column_stack([np.ones(n), x])

    lat = np.array([asi.cent[g][0] for g in geoids]); lon = np.array([asi.cent[g][1] for g in geoids])
    xy = np.column_stack([lon * 111_320 * math.cos(math.radians(40.7)), lat * 110_540])
    cells = np.unique(np.floor(xy / 1000).astype(int), axis=0, return_inverse=True)[1].ravel()
    groups = {
        "tract": np.array([g[:11] for g in geoids]),
        "nta": np.array([asi.bg_nta[g] for g in geoids]),
        "puma": np.array([asi.tract_puma[g[:11]] for g in geoids]),
    }
    K = {bw: asi.conley_kernel(xy, bw) for bw in asi.CONLEY_BWS}

    # row-normalised destination shares
    pos = {g: i for i, g in enumerate(geoids)}
    work_ix, R, C, V = {}, [], [], []
    with open(PAIRS) as f:
        for r in csv.DictReader(f):
            i = pos.get(r["home_bg"])
            if i is None:
                continue
            R.append(i); C.append(work_ix.setdefault(r["work_bg"], len(work_ix))); V.append(float(r["jobs"]))
    P = csr_matrix((V, (R, C)), shape=(n, len(work_ix)))
    rs = np.asarray(P.sum(1)).ravel()
    P = csr_matrix(P.multiply(1.0 / np.where(rs > 0, rs, 1.0)[:, None]))

    rng = np.random.default_rng(SEED)
    out = {"_readme": {"n_sim": N_SIM, "wcb_B": B_WCB, "seed": SEED, "n": n,
                       "regressor": "z-scored %Hispanic (pop-weighted), A_mnl sample, true beta = 0",
                       "scenarios_sd_cell_dest_iid": SCENARIOS,
                       "metrics": "se_ratio = mean(SE)/SD(beta_hat); size = rejection rate at 5%"},
           "scenarios": {}}
    names = ["hc1", "cr1_tract", "cr1_nta", "cr1_puma"] + [f"conley_{bw/1000:g}km" for bw in asi.CONLEY_BWS]
    for label, (s_cell, s_dest, s_iid) in SCENARIOS.items():
        betas, ses, rej = [], {k: [] for k in names}, {k: 0 for k in names + ["wcb_puma"]}
        n_cells = cells.max() + 1
        for _ in range(N_SIM):
            y = rng.normal(0, s_iid, n) / np.sqrt(w)
            if s_cell:
                y = y + rng.normal(0, s_cell, n_cells)[cells]
            if s_dest:
                y = y + P @ rng.normal(0, s_dest, P.shape[1])
            beta, e, bread, S = asi.fit(y, X, w)
            b = beta[1]
            betas.append(b)
            est = {"hc1": asi.v_from_meat(bread, S.T @ S, n / (n - 2))}
            for g, grp in groups.items():
                est[f"cr1_{g}"] = asi.cr1(bread, S, grp, n, 2)[0]
            for bw in asi.CONLEY_BWS:
                est[f"conley_{bw/1000:g}km"] = asi.kernel_v(bread, S, K[bw])
            for k, Vm in est.items():
                se = math.sqrt(max(Vm[1, 1], 0.0))
                ses[k].append(se)
                rej[k] += abs(b / se) > 1.959964 if se > 0 else 1
            p, _, _ = asi.wild_cluster_p(y, X, w, groups["puma"], 1, rng, B=B_WCB)
            rej["wcb_puma"] += p < 0.05
        sd = float(np.std(betas))
        rec = {"sd_beta": sd,
               "se_ratio": {k: float(np.mean(v) / sd) for k, v in ses.items()},
               "size": {k: v / N_SIM for k, v in rej.items()}}
        out["scenarios"][label] = rec
        print(f"{label:12s} SD(beta) {sd:.3f}  se_ratio: " +
              " ".join(f"{k} {v:.2f}" for k, v in rec["se_ratio"].items()), flush=True)
        print(f"{'':12s} size@5%:       " + " ".join(f"{k} {v:.3f}" for k, v in rec["size"].items()), flush=True)

    with open(os.path.join(asi.OUTDIR, "inference_montecarlo.json"), "w") as f:
        json.dump(out, f, indent=2)
    print(f"wrote {asi.OUTDIR}/inference_montecarlo.json")


if __name__ == "__main__":
    main()
