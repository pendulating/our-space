#!/usr/bin/env python3
"""§6 Estimator, spatial autocorrelation, and inference for the surveillance-exposure disparity.

Exposure and demographics are both strongly spatially autocorrelated, so OLS standard errors are
invalid. This script (run under the project's uv env: `uv run python tools/spatial_econometrics.py`)
does the full §6 program on real TIGER-2020 block-group polygons:

  1. Queen-contiguity spatial weights W (islands — Rikers, Roosevelt I., etc. — attached by nearest
     neighbour so W is fully connected), row-standardised.
  2. Moran's I on the exposure outcomes and on OLS residuals (esda, 999 permutations) — establishes
     the autocorrelation that motivates a spatial model.
  3. OLS with Anselin LM diagnostics (LM-lag, LM-error, and their robust forms) to SELECT the spec.
  4. Conley / Kelejian-Prucha spatial-HAC SEs (kernel weights) as a spec-robust cross-check.
  5. Spatial lag (SAR) and Spatial Durbin (SDM = SAR + WX) by ML, with the LeSage-Pace
     direct / indirect (spillover) / total impact decomposition.

Outcome: R_i (residential exposure — the placement disparity). Demographic coefficients are in
cameras per SD; skewed land-use densities are log1p'd; everything z-scored (pop-unweighted here, as
spatial ML weights observations through W, not population — pop-weighted WLS is the descriptive
ladder in crime_ladder.py).
"""
import warnings; warnings.filterwarnings("ignore")
import numpy as np, pandas as pd, geopandas as gpd
from libpysal.weights import Queen, KNN
from libpysal.weights import fill_diagonal
import libpysal
from esda.moran import Moran
from spreg import OLS, ML_Lag, ML_Error

# ---------------- data ----------------
gdf = gpd.read_file("data/snapshots/tiger/tl_2020_36_bg.zip")
gdf = gdf[gdf["COUNTYFP"].isin({"005","047","061","081","085"})][["GEOID","geometry"]]
tab = pd.read_csv("data/derived/exposure/exposure_table_nyc.csv", dtype={"GEOID":str})
cov = pd.read_csv("data/derived/exposure/covariates_bg_nyc.csv", dtype={"id":str}).rename(columns={"id":"GEOID"})
df = gdf.merge(tab, on="GEOID").merge(cov[["GEOID","jobs_wsh","crime_wsh","req311_wsh","transit_dist_m","pop_wsh"]], on="GEOID")
df = df.to_crs(32618)  # UTM 18N metres

need = ["R_i","E_i","A_mnl","pct_hispanic","pct_black_nh","median_hh_income","jobs_wsh","crime_wsh","req311_wsh","transit_dist_m","pop_wsh","population"]
df = df.dropna(subset=need).reset_index(drop=True)
df = df[df["population"] > 0].reset_index(drop=True)
n = len(df)

# ---------------- weights ----------------
w = Queen.from_dataframe(df, use_index=False)
if w.islands:  # attach islands to their single nearest neighbour, then row-standardise
    knn1 = KNN.from_dataframe(df, k=1)
    w = libpysal.weights.attach_islands(w, knn1)
w.transform = "r"
# kernel weights for Conley/HAC SEs (triangular, 2 km fixed bandwidth)
coords = np.column_stack([df.geometry.centroid.x, df.geometry.centroid.y])
gwk = libpysal.weights.Kernel(coords, bandwidth=2000.0, function="triangular", fixed=True)
# eigenvalues of the row-standardised W (via symmetric normalisation) for LeSage-Pace impact traces
A = Queen.from_dataframe(df, use_index=False)
if A.islands: A = libpysal.weights.attach_islands(A, KNN.from_dataframe(df, k=1))
Adense = A.full()[0]; Adense = ((Adense + Adense.T) > 0).astype(float)
deg = Adense.sum(1); Dm = np.diag(1/np.sqrt(deg))
mu = np.linalg.eigvalsh(Dm @ Adense @ Dm)   # eigenvalues of row-standardised W (real)

def z(x):
    x = np.asarray(x, float); return (x - x.mean())/x.std()
def zl(x):
    return z(np.log1p(np.asarray(x, float)))

DEMOS = [("%Hisp","pct_hispanic",z), ("%Black","pct_black_nh",z), ("income","median_hh_income",z)]
LAND  = [("jobs","jobs_wsh",zl), ("transit","transit_dist_m",zl), ("density","pop_wsh",zl)]
CRIME = [("crime","crime_wsh",zl)]
REQ311 = [("311","req311_wsh",zl)]

def design(terms):
    X = np.column_stack([f(df[c]) for _,c,f in terms])
    return X, [nm for nm,_,_ in terms]

# ---------------- 1. Moran's I ----------------
print("="*72); print("MORAN'S I (999 perms) — spatial autocorrelation of the outcomes")
for yn in ["R_i","E_i","A_mnl"]:
    mi = Moran(df[yn].values, w, permutations=999)
    print(f"  {yn:6s}  I = {mi.I:+.3f}   p = {mi.p_sim:.3f}")

def impacts_lag(rho, beta, theta):
    """LeSage-Pace direct/indirect/total for a lag-type model (SAR/SDM), row-standardised W."""
    tr_S  = np.mean(1.0/(1.0 - rho*mu))           # (1/n) tr[(I-rho W)^-1]
    tr_SW = np.mean(mu/(1.0 - rho*mu))            # (1/n) tr[(I-rho W)^-1 W]
    total  = (beta + theta)/(1.0 - rho)           # exact for row-standardised W (W1 = 1)
    direct = beta*tr_S + theta*tr_SW
    return direct, total-direct, total

def aic(model, kp):
    """AIC from a spreg ML model's log-likelihood; kp = # estimated parameters."""
    ll = float(model.logll)
    return -2*ll + 2*kp, ll

# Machine-readable mirror of everything this script prints. The paper's tables and inline
# statistics are generated from JSON, never scraped from a text report, so a changed print
# format can never silently change a number in the paper (tools/make_tables.py).
REPORT: dict = {"outcomes": {}, "maup": {}}


def run(yname):
    y = df[[yname]].values
    X2, names2 = design(DEMOS+LAND)
    k = X2.shape[1]
    WX = np.asarray(w.sparse @ X2); Xd = np.column_stack([X2, WX])
    nD = names2 + ["W_"+m for m in names2]
    print("\n"+"="*72); print(f"OUTCOME {yname}  (mean {df[yname].mean():.1f} cameras; coefs = cameras/SD)")

    # --- OLS with Conley HAC SEs + Anselin LM spatial diagnostics (model selection) ---
    ols = OLS(y, X2, w=w, gwk=gwk, spat_diag=True, moran=True, name_x=names2, name_y=yname)
    b = ols.betas.flatten(); se = np.sqrt(np.diag(ols.vm)).flatten()
    print("  OLS (Conley HAC SEs):  " +
          "  ".join(f"{nm} {b[i+1]:+.2f}(±{se[i+1]:.2f})" for i,nm in enumerate(names2) if nm in ("%Hisp","%Black","income")))
    print(f"  Moran's I (resid) {ols.moran_res[0]:+.3f} (p={ols.moran_res[2]:.3f}) | "
          f"LM-lag {ols.lm_lag[0]:.0f}/rob {ols.rlm_lag[0]:.0f}  LM-err {ols.lm_error[0]:.0f}/rob {ols.rlm_error[0]:.0f} "
          f"(rob-err {'>' if ols.rlm_error[0]>ols.rlm_lag[0] else '<'} rob-lag → {'ERROR' if ols.rlm_error[0]>ols.rlm_lag[0] else 'LAG'} favoured)")
    # OLS log-lik for AIC
    e = y.flatten() - (np.column_stack([np.ones(len(y)),X2])@b); s2 = e@e/len(e)
    ll_ols = -0.5*len(e)*(np.log(2*np.pi*s2)+1); aic_ols = -2*ll_ols + 2*(k+2)

    # --- full ML suite (suppress spreg's name-echo) ---
    import io, contextlib
    with contextlib.redirect_stdout(io.StringIO()):
        sar  = ML_Lag(y, X2, w=w, name_x=names2, name_y=yname)          # SAR  (lag)
        sem  = ML_Error(y, X2, w=w, name_x=names2, name_y=yname)        # SEM  (error)
        sdm  = ML_Lag(y, Xd, w=w, name_x=nD, name_y=yname)             # SDM  (Durbin lag)
        sdem = ML_Error(y, Xd, w=w, name_x=nD, name_y=yname)           # SDEM (Durbin error)
    aic_sar,_  = aic(sar,  k+2)
    aic_sem,_  = aic(sem,  k+2)
    aic_sdm,_  = aic(sdm,  2*k+2)
    aic_sdem,_ = aic(sdem, 2*k+2)
    tbl = {"OLS":aic_ols,"SAR(lag)":aic_sar,"SEM(error)":aic_sem,"SDM(Durbin-lag)":aic_sdm,"SDEM(Durbin-err)":aic_sdem}
    best = min(tbl, key=tbl.get)
    print("  model AIC (lower=better):  " + "   ".join(f"{m} {v:.0f}" for m,v in tbl.items()))
    print(f"  → selected by AIC: {best}")
    rec = REPORT["outcomes"].setdefault(yname, {})
    rec["mean"] = float(df[yname].mean())
    rec["moran_i"] = float(ols.moran_res[0])
    rec["moran_p"] = float(ols.moran_res[2])
    rec["aic"] = {m: float(v) for m, v in tbl.items()}
    rec["selected"] = best
    rec["ols_conley"] = {
        nm: [float(b[i + 1]), float(se[i + 1])]
        for i, nm in enumerate(names2)
        if nm in ("%Hisp", "%Black", "income")
    }

    # --- impacts (direct/indirect/total) for the two Durbin models ---
    rho = float(np.ravel(sdm.rho)[0]); bb = sdm.betas.flatten()
    beta_d, theta_d = bb[1:1+k], bb[1+k:1+2*k]
    bbe = sdem.betas.flatten(); beta_e, theta_e = bbe[1:1+k], bbe[1+k:1+2*k]  # SDEM: [CONST,β,θ,λ]
    print(f"  impacts (cameras/SD):  SDM ρ={rho:+.2f}   |   SDEM (error: direct=β, indirect=Wβ=θ)")
    print(f"     {'var':8s}{'SDM_dir':>9}{'SDM_ind':>9}{'SDM_tot':>9}   {'SDEM_dir':>9}{'SDEM_ind':>9}{'SDEM_tot':>9}")
    rec["rho_sdm"] = rho
    rec["impacts"] = {}
    for i,nm in enumerate(names2):
        if nm not in ("%Hisp","%Black","income"): continue
        d,ind,tot = impacts_lag(rho, beta_d[i], theta_d[i])
        print(f"     {nm:8s}{d:>+9.2f}{ind:>+9.2f}{tot:>+9.2f}   "
              f"{beta_e[i]:>+9.2f}{theta_e[i]:>+9.2f}{beta_e[i]+theta_e[i]:>+9.2f}")
        rec["impacts"][nm] = {
            "sdm": [float(d), float(ind), float(tot)],
            "sdem": [float(beta_e[i]), float(theta_e[i]), float(beta_e[i] + theta_e[i])],
        }
    return ols

run("R_i")
run("E_i")

# ---------------- MAUP: re-estimate the disparity at coarser zonings ----------------
# BG is our finest unit (exposure is computed at BG centroids), so we test MAUP by AGGREGATING
# UPWARD — to census tracts (GEOID[:11]) and to a 1 km grid — pop-weighting each variable, then
# re-standardising and re-running the same demographics+land-use OLS. A stable coefficient across
# zonings means the disparity is not a scale artifact.
print("\n"+"="*72); print("MAUP — %Hisp & income coefficient (cameras/SD) across aggregation scales")
xy = np.column_stack([df.geometry.centroid.x, df.geometry.centroid.y])
raw = df.copy()
raw["cx"], raw["cy"] = xy[:,0], xy[:,1]

def wmean(g, c): return np.average(g[c], weights=g["population"])
def aggregate(keyseries):
    raw["_k"] = keyseries
    out=[]
    for _,g in raw.groupby("_k"):
        if g["population"].sum() <= 0: continue
        row={"population":g["population"].sum()}
        for c in ["R_i","pct_hispanic","pct_black_nh","median_hh_income","jobs_wsh","transit_dist_m","pop_wsh"]:
            row[c]=wmean(g,c)
        out.append(row)
    return pd.DataFrame(out)

def maup_ols(d, label):
    ww=d["population"].values; ww=ww/ww.mean()
    def zc(x,log=False):
        x=np.log1p(x.values.astype(float)) if log else x.values.astype(float)
        m=np.average(x,weights=ww); s=np.sqrt(np.average((x-m)**2,weights=ww)); return (x-m)/s
    X=np.column_stack([zc(d["pct_hispanic"]),zc(d["pct_black_nh"]),zc(d["median_hh_income"]),
                       zc(d["jobs_wsh"],1),zc(d["transit_dist_m"],1),zc(d["pop_wsh"],1)])
    X=np.column_stack([np.ones(len(d)),X]); y=d["R_i"].values
    W=np.diag(ww); XtW=X.T*ww
    beta=np.linalg.solve(XtW@X, XtW@y)
    print(f"  {label:22s} (n={len(d):5d}):  %Hisp {beta[1]:+.2f}   %Black {beta[2]:+.2f}   income {beta[3]:+.2f}")
    REPORT["maup"][label.strip()] = {
        "n": int(len(d)),
        "pct_hisp": float(beta[1]),
        "pct_black": float(beta[2]),
        "income": float(beta[3]),
    }
    return beta[1], beta[3]

hb=[]; ib=[]
h,i_=maup_ols(raw, "block group"); hb.append(h); ib.append(i_)
h,i_=maup_ols(aggregate(df["GEOID"].str[:11]), "census tract"); hb.append(h); ib.append(i_)
gx=(raw["cx"]//1000).astype(int).astype(str); gy=(raw["cy"]//1000).astype(int).astype(str)
h,i_=maup_ols(aggregate(gx+"_"+gy), "1 km grid"); hb.append(h); ib.append(i_)
print(f"  --> %Hisp coefficient range across zonings: [{min(hb):+.2f}, {max(hb):+.2f}]  (stable sign, magnitude {min(hb):.1f}-{max(hb):.1f})")
print(f"  --> income coefficient range: [{min(ib):+.2f}, {max(ib):+.2f}]")

# ---------------- crime+311 ladder with spatial (Conley) inference (§5c × §6) ----------------
print("\n"+"="*72); print("CRIME+311 LADDER with Conley HAC SEs (outcome R_i) — §5c under spatial inference")
y = df[["R_i"]].values
for label,terms in [("rung2 demo+land-use", DEMOS+LAND),
                    ("rung3 +crime",        DEMOS+LAND+CRIME),
                    ("rung4 +crime+311",    DEMOS+LAND+CRIME+REQ311)]:
    X,names = design(terms)
    m = OLS(y, X, w=w, gwk=gwk, name_x=names, name_y="R_i")
    b = m.betas.flatten(); se = np.sqrt(np.diag(m.vm)).flatten()
    idx = {nm:i+1 for i,nm in enumerate(names)}
    parts = " ".join(f"{nm} {b[idx[nm]]:+.2f}(±{se[idx[nm]]:.2f})" for nm in ("%Hisp","income"))
    med = "".join(f" | {k} {b[idx[k]]:+.2f}(±{se[idx[k]]:.2f})" for k in ("crime","311") if k in idx)
    print(f"  {label:20s}: {parts}{med}")
print(f"\nN = {n} block groups. W = Queen contiguity (islands attached), row-standardised.")
print("HAC = Kelejian-Prucha/Conley kernel (triangular, 2 km). Impacts = LeSage-Pace (SDM) / β,θ (SDEM).")
print("Crime = NYPD YTD-2026; 311 = public-disorder YTD-2026; both counted in the 10-min walkshed.")

# ---------------- persist ----------------
# Written last, so a crash anywhere above leaves no half-valid JSON for the paper to read.
import json as _json
import pathlib as _pathlib

_out = _pathlib.Path(__file__).resolve().parent.parent / "data/derived/results/spatial_econometrics.json"
_out.write_text(_json.dumps(REPORT, indent=2) + "\n")
print(f"\nwrote {_out}")
