#!/usr/bin/env python3
"""§7 flesh-out — is the CCTV undercount racialized, and does it change the disparity?

capture_recapture.py established a citywide census recall of ~50% (Amnesty x Dahir, Chapman). Here
we (1) fit a DETECTION MODEL — the occupancy/N-mixture question: does the probability that the
crowdsourced census detects a KNOWN-present camera vary with neighbourhood demographics? — and
(2) PROPAGATE the capture-recapture uncertainty into the headline disparity coefficient, so the
undercount enters the inference (plan §6 uncertainty bullet i) rather than being asserted away.

Detection model: for each Dahir ML detection (a camera we know is present), matched=1 if the
Amnesty crowdsource also has a site within 50 m (i.e. Amnesty detected it). Logit(matched ~
demographics + density) at the detection's block group. Flat coefficients ⇒ non-differential
undercount ⇒ the disparity is not an artifact of who-gets-mapped.

Propagation: bootstrap the two camera lists → per-borough recall draws → inflate each BG's CCTV
count by 1/recall(borough) → re-estimate the pop-weighted %Hispanic disparity coefficient on R_i.
The resulting band is the disparity's uncertainty DUE TO the undercount.

Run: uv run python tools/undercount_spatial.py
"""
import warnings; warnings.filterwarnings("ignore")
import math, numpy as np, pandas as pd
import statsmodels.api as sm

MATCH_M = 50.0
BORO = {"36005":"Bronx","36047":"Brooklyn","36061":"Manhattan","36081":"Queens","36085":"Staten Island"}
LAT0=40.70; MLAT=111320.0; MLON=111320.0*math.cos(math.radians(LAT0))
def to_m(lat,lon): return ((lon+74.0)*MLON, (lat-LAT0)*MLAT)

# ---- raw camera lists ----
amn=pd.read_csv("data/snapshots/amnesty/counts_per_intersections.csv")
amn=amn[(pd.to_numeric(amn["n_cameras_median"],errors="coerce")>=1)].dropna(subset=["Lat","Long"])
amn_ll=amn[["Lat","Long","BoroName"]].values
amn_m=np.array([to_m(la,lo) for la,lo,_ in amn_ll])

dah=pd.read_csv("data/snapshots/dahir/map_data.csv")
dah=dah[(dah["city"]=="New York") & (pd.to_numeric(dah["camera_count"],errors="coerce")>=1)]
dah_ll=dah[["lat","lon"]].values
dah_m=np.array([to_m(la,lo) for la,lo in dah_ll])

# ---- 50 m match via grid ----
grid={}
for i,(x,y) in enumerate(amn_m): grid.setdefault((int(x//MATCH_M),int(y//MATCH_M)),[]).append(i)
def amnesty_near(pm):
    cx,cy=int(pm[0]//MATCH_M),int(pm[1]//MATCH_M)
    for dx in (-1,0,1):
        for dy in (-1,0,1):
            for i in grid.get((cx+dx,cy+dy),[]):
                if (amn_m[i,0]-pm[0])**2+(amn_m[i,1]-pm[1])**2 <= MATCH_M*MATCH_M: return True
    return False
matched=np.array([amnesty_near(pm) for pm in dah_m], float)

# ---- assign each Dahir detection to its nearest block-group centroid → demographics ----
cent=pd.read_csv("data/snapshots/census/bg_centroids_nyc.csv",dtype={"id":str})
cent_m=np.array([to_m(la,lo) for la,lo in cent[["lat","lon"]].values])
tab=pd.read_csv("data/derived/exposure/exposure_table_nyc.csv",dtype={"GEOID":str}).set_index("GEOID")
cov=pd.read_csv("data/derived/exposure/covariates_bg_nyc.csv",dtype={"id":str}).set_index("id")

def nearest_bg(pm):
    d2=(cent_m[:,0]-pm[0])**2+(cent_m[:,1]-pm[1])**2
    return cent["id"].values[int(np.argmin(d2))]
dah_bg=[nearest_bg(pm) for pm in dah_m]

rows=[]
for g,mm in zip(dah_bg,matched):
    if g not in tab.index or g not in cov.index: continue
    t=tab.loc[g]; c=cov.loc[g]
    try:
        rows.append(dict(matched=mm,
            hisp=float(t["pct_hispanic"]), black=float(t["pct_black_nh"]),
            inc=float(t["median_hh_income"]),
            dens=math.log1p(float(c["pop_wsh"])), jobs=math.log1p(float(c["jobs_wsh"]))))
    except (ValueError,KeyError): pass
D=pd.DataFrame(rows).dropna()
def z(s): return (s-s.mean())/s.std()

print("="*70)
print(f"DETECTION MODEL — Logit(Amnesty detects a Dahir-known camera), N={len(D)} detections, "
      f"{int(D['matched'].sum())} matched")
X=sm.add_constant(pd.DataFrame({k:z(D[k]) for k in ["hisp","black","inc","dens","jobs"]}))
res=sm.Logit(D["matched"].values, X).fit(disp=0)
print(f"  {'term':8s}{'coef':>9}{'se':>8}{'p':>8}   (coef = log-odds of being detected per SD)")
for name in X.columns:
    print(f"  {name:8s}{res.params[name]:>+9.3f}{res.bse[name]:>8.3f}{res.pvalues[name]:>8.3f}")
sig=[n for n in ["hisp","black","inc","dens","jobs"] if res.pvalues[n]<0.05]
print(f"  → demographic detection bias: {'NONE significant (non-differential undercount)' if not any(s in sig for s in ['hisp','black','inc']) else 'present in '+','.join(s for s in sig if s in ['hisp','black','inc'])}")

# Persist — the paper's "the undercount is non-differential" sentence cites these p-values, so
# they must live in data/derived/results/ like every other quoted statistic (make_tables reads it).
import json, os
DET={"n_detections": int(len(D)), "n_matched": int(D["matched"].sum()),
     "terms": {name: {"coef": float(res.params[name]), "se": float(res.bse[name]),
                      "p": float(res.pvalues[name])} for name in X.columns if name!="const"},
     "non_differential": not any(s in sig for s in ["hisp","black","inc"]),
     "note": "Logit(Amnesty crowdsource detects a Dahir-ML-known camera) on z-scored BG "
             "demographics + density; flat demographic terms => the census undercount is "
             "non-differential w.r.t. the demographics of interest."}

# ============================================================
# PROPAGATE undercount uncertainty into the %Hispanic disparity coefficient on R_i.
# ============================================================
def chapman(n1,n2,m): return (n1+1)*(n2+1)/(m+1)-1.0
amn_boro=[b for _,_,b in amn_ll]
# borough of each Dahir detection via nearest Amnesty (shared GSV substrate)
def near_amn_boro(pm):
    d2=(amn_m[:,0]-pm[0])**2+(amn_m[:,1]-pm[1])**2; return amn_boro[int(np.argmin(d2))]
dah_boro=[near_amn_boro(pm) for pm in dah_m]

# build the analysis frame for the disparity coefficient
adf=[]
for g in tab.index:
    t=tab.loc[g]; boro=BORO.get(g[:5])
    try:
        pop=float(t["population"]); rc=float(t["R_cctv"]); ri=float(t["R_i"])
    except (ValueError,KeyError): continue
    if pop>0 and boro and not math.isnan(rc):
        adf.append((pop, rc, ri-rc, float(t["pct_hispanic"]), boro))
adf=pd.DataFrame(adf,columns=["pop","R_cctv","R_other","hisp","boro"]).dropna()

def hisp_coef(recall_by_boro):
    """pop-weighted OLS coefficient of R_i_corrected on z(%Hisp) (cameras/SD)."""
    rec=adf["boro"].map(lambda b: recall_by_boro.get(b,np.nan)).values
    R=adf["R_cctv"].values/rec + adf["R_other"].values
    w=adf["pop"].values; w=w/w.mean()
    x=adf["hisp"].values; x=(x-np.average(x,weights=w))/math.sqrt(np.average((x-np.average(x,weights=w))**2,weights=w))
    X=np.column_stack([np.ones(len(x)),x]); XtW=X.T*w
    return np.linalg.solve(XtW@X, XtW@R)[1]

allA=np.arange(len(amn_m)); allD=np.arange(len(dah_m))
def recall_boro(iA,iD):
    out={}
    for b in ["Manhattan","Brooklyn","Bronx","Queens","Staten Island"]:
        a=[i for i in iA if amn_boro[i]==b]; d=[i for i in iD if dah_boro[i]==b]
        m=sum(matched[i] for i in d)
        if m==0 or not a: out[b]=np.nan; continue
        N=chapman(len(a),len(d),m); out[b]=(len(a)+len(d)-m)/N
    glob=np.nanmean(list(out.values()))
    return {b:(v if not np.isnan(v) else glob) for b,v in out.items()}

point=hisp_coef({b:1.0 for b in BORO.values()})           # raw (recall=1)
point_corr=hisp_coef(recall_boro(allA,allD))               # spatial-recall corrected
rng=np.random.default_rng(0)
draws=[]
for _ in range(500):
    ra=rng.choice(allA,size=len(allA),replace=True)
    rd=rng.choice(allD,size=len(allD),replace=True)
    # recompute matched-count per borough from the resample
    def rec_from(ra,rd):
        out={}
        for b in ["Manhattan","Brooklyn","Bronx","Queens","Staten Island"]:
            a=[i for i in ra if amn_boro[i]==b]; d=[i for i in rd if dah_boro[i]==b]
            m=sum(matched[i] for i in d)
            if m==0 or not a: out[b]=np.nan; continue
            N=chapman(len(a),len(d),m); out[b]=(len(a)+len(d)-m)/N
        glob=np.nanmean([v for v in out.values() if not np.isnan(v)])
        return {b:(v if not np.isnan(v) else glob) for b,v in out.items()}
    draws.append(hisp_coef(rec_from(ra,rd)))
draws=np.sort(draws)
lo,hi=draws[int(.025*len(draws))],draws[int(.975*len(draws))]
print("\n"+"="*70)
print("PROPAGATION — %Hispanic disparity coefficient on R_i (cameras/SD), pop-weighted")
print(f"  raw counts (recall=1)                : {point:+.2f}")
print(f"  undercount-corrected (point)         : {point_corr:+.2f}")
print(f"  undercount-corrected 95% band        : [{lo:+.2f}, {hi:+.2f}]")
print(f"  → the disparity coefficient is {'ROBUST' if lo>0 else 'SENSITIVE'} to undercount "
      f"(correction {'raises' if point_corr>point else 'lowers'} it; band excludes 0: {lo>0})")

os.makedirs("data/derived/results", exist_ok=True)
with open("data/derived/results/detection_model.json","w") as fh:
    json.dump({"detection_model": DET,
               # cameras/SD values: the correction rescales the OUTCOME'S UNITS (~1.95x), so the
               # coefficient roughly doubling here is a units artifact, NOT an amplification of
               # the disparity. Quote the detection model; lead with scale-free correlations
               # (capture_recapture.json / recall_sensitivity.json). Kept for the record.
               "propagation_cameras_per_sd": {"raw": float(point), "corrected": float(point_corr),
                                              "band_95": [float(lo), float(hi)]}}, fh, indent=2)
print("wrote data/derived/results/detection_model.json")
