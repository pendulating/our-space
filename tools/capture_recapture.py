#!/usr/bin/env python3
"""§7 Undercount model — capture-recapture on two independent CCTV censuses.

The crowdsourced camera census undercounts, plausibly correlated with the outcome (the sharpest
reviewer objection). But we have TWO independent enumerations of NYC street cameras:
  - Amnesty "Decode Surveillance NYC" — crowdsourced human decoding of Google Street View
    (intersections reporting >=1 camera; `n_cameras_median>=1`), citywide.
  - Dahir et al. 2025 — ML camera detection on a sparse GSV panorama sample (`camera_count>0`).
A camera site seen by BOTH is a recapture, so a Lincoln-Petersen / Chapman estimator recovers the
TRUE number of sites and the detection (recall) rate — globally and per borough (spatially-varying).

We then show how the R_i disparity moves under (a) raw counts, (b) a global recall inflation, and
(c) spatially-varying recall — i.e. whether the undercount could DRIVE or MASK the disparity.

CAVEAT (stated, not hidden): both censuses derive from Google Street View, so their detections are
positively dependent (shared substrate). Positive dependence inflates the overlap m, which biases
Chapman's N-hat DOWNWARD — so the estimated undercount is a CONSERVATIVE lower bound (true cameras
>= N-hat). Amnesty (human) vs Dahir (ML) differ enough in process to retain partial independence.

Pure numpy; 50 m matching via a meters grid; bootstrap bands over both lists.
"""
import csv, json, math, os, random
import numpy as np

MATCH_M = 50.0     # DEDUP_RADIUS_M from the baking pipeline: same physical site
random.seed(0)     # Math.random() is unavailable in workflow scripts; here plain python is fine

# ---- meters projection (equirectangular about NYC) ----
LAT0 = 40.70
MLAT = 111320.0
MLON = 111320.0 * math.cos(math.radians(LAT0))
def to_m(lat, lon): return ((lon + 74.0) * MLON, (lat - LAT0) * MLAT)

BORO = {"36005":"Bronx","36047":"Brooklyn","36061":"Manhattan","36081":"Queens","36085":"Staten Island"}

# ---- load Amnesty sites (citywide, >=1 camera) ----
amn=[]
for r in csv.DictReader(open("data/snapshots/amnesty/counts_per_intersections.csv")):
    try:
        if not r.get("n_cameras_median") or float(r["n_cameras_median"])<1: continue
        lat,lon=float(r["Lat"]),float(r["Long"])
    except (ValueError,TypeError): continue
    amn.append((lat,lon,r.get("BoroName","").strip()))

# ---- load Dahir NY detections ----
dah=[]
for r in csv.DictReader(open("data/snapshots/dahir/map_data.csv")):
    if r.get("city")!="New York": continue
    try:
        if not r.get("camera_count") or int(float(r["camera_count"]))<1: continue
        dah.append((float(r["lat"]),float(r["lon"])))
    except (ValueError,TypeError): continue

# ---- assign each Dahir point a borough via nearest Amnesty site (cheap, same GSV substrate) ----
amn_m=np.array([to_m(la,lo) for la,lo,_ in amn])
amn_boro=[b for _,_,b in amn]

def nearest_amnesty(pm):
    d2=(amn_m[:,0]-pm[0])**2+(amn_m[:,1]-pm[1])**2
    i=int(np.argmin(d2)); return i, math.sqrt(d2[i])

# ---- 50 m grid over Amnesty sites for fast matching ----
def build_grid(pts_m, cell):
    g={}
    for i,(x,y) in enumerate(pts_m):
        g.setdefault((int(x//cell),int(y//cell)),[]).append(i)
    return g
grid=build_grid(amn_m, MATCH_M)
def amnesty_within(pm, r=MATCH_M):
    cx,cy=int(pm[0]//MATCH_M),int(pm[1]//MATCH_M); hits=[]
    for dx in (-1,0,1):
        for dy in (-1,0,1):
            for i in grid.get((cx+dx,cy+dy),[]):
                if (amn_m[i,0]-pm[0])**2+(amn_m[i,1]-pm[1])**2 <= r*r: hits.append(i)
    return hits

dah_m=[to_m(la,lo) for la,lo in dah]
dah_boro=[]; matched=[]
for pm in dah_m:
    hits=amnesty_within(pm)
    matched.append(len(hits)>0)
    j,_=nearest_amnesty(pm); dah_boro.append(amn_boro[j])

def chapman(n1,n2,m):
    return (n1+1)*(n2+1)/(m+1)-1.0

def estimate(idx_amn, idx_dah):
    """Chapman N-hat + recall for a subset (indices into amn / dah)."""
    n1=len(idx_amn); n2=len(idx_dah)
    m=sum(matched[i] for i in idx_dah)
    if m==0: return None
    N=chapman(n1,n2,m)
    observed=n1+ (n2-m)             # distinct sites actually observed (union)
    return dict(n1=n1,n2=n2,m=m,N=N,recall=observed/N,
                p_amnesty=m/n2, p_dahir=m/n1, observed=observed)

allA=list(range(len(amn))); allD=list(range(len(dah)))
g=estimate(allA,allD)
print("="*70)
print("GLOBAL capture-recapture (Amnesty x Dahir, citywide)")
print(f"  Amnesty sites n1={g['n1']:,}  Dahir detections n2={g['n2']}  overlap m={g['m']}")
print(f"  Chapman N-hat (true sites) = {g['N']:,.0f}")
print(f"  observed (union) = {g['observed']:,}  ->  census recall = {g['recall']*100:.1f}%")
print(f"  per-source detection: Amnesty p={g['p_amnesty']*100:.1f}%  Dahir p={g['p_dahir']*100:.1f}%")

# ---- bootstrap bands (resample both lists with replacement) ----
def boot(idxA, idxD, B=2000):
    Ns=[]; recs=[]
    for _ in range(B):
        ra=[random.choice(idxA) for _ in idxA]
        rd=[random.choice(idxD) for _ in idxD]
        n1=len(ra); n2=len(rd); m=sum(matched[i] for i in rd)
        if m==0: continue
        N=chapman(n1,n2,m); Ns.append(N); recs.append((n1+n2-m)/N)
    Ns.sort(); recs.sort()
    lo,hi=Ns[int(.025*len(Ns))],Ns[int(.975*len(Ns))]
    rlo,rhi=recs[int(.025*len(recs))],recs[int(.975*len(recs))]
    return (lo,hi),(rlo,rhi),recs
(Nlo,Nhi),(rlo,rhi),RECALL_DRAWS=boot(allA,allD)
print(f"  95% bootstrap: N-hat [{Nlo:,.0f}, {Nhi:,.0f}]  recall [{rlo*100:.1f}%, {rhi*100:.1f}%]")

# ---- per-borough spatially-varying recall ----
print("\nPER-BOROUGH detection (spatially-varying recall):")
print(f"  {'borough':<15}{'n1':>7}{'n2':>6}{'m':>5}{'N-hat':>9}{'recall':>9}")
boro_recall={}
for b in ["Manhattan","Brooklyn","Bronx","Queens","Staten Island"]:
    iA=[i for i in allA if amn[i][2]==b]
    iD=[i for i in allD if dah_boro[i]==b]
    e=estimate(iA,iD)
    if e:
        boro_recall[b]=e["recall"]
        print(f"  {b:<15}{e['n1']:>7,}{e['n2']:>6}{e['m']:>5}{e['N']:>9,.0f}{e['recall']*100:>8.1f}%")
    else:
        print(f"  {b:<15}{len(iA):>7,}{len(iD):>6}{0:>5}{'—':>9}{'—':>9}  (no overlap)")

# ============================================================
# Disparity movement under (a) raw, (b) global recall, (c) spatial recall.
# Inflate ONLY the UNCONFIRMED sub-population (R_i_unconfirmed: CCTV-census-only groups no
# surveyed source attests). A CCTV group co-located with a DOT/ALPR/enforcement record is
# itself surveyed and must not be inflated — this is the canonical reconstruction
#     R(r) = R_i_unconfirmed / r + (R_i - R_i_unconfirmed)
# that sweep_recall.py and the batch use. (Until 2026-07-14 this block inflated all of R_cctv,
# which disagreed with sweep_recall.py by ~1% on the corrected mean.)
# ============================================================
tab=[r for r in csv.DictReader(open("data/derived/exposure/exposure_table_nyc.csv"))]
def f(r,k):
    v=r.get(k)
    try: return float(v) if v not in (None,"") else None
    except ValueError: return None

rows=[]
for r in tab:
    pop=f(r,"population"); unc=f(r,"R_i_unconfirmed"); ri=f(r,"R_i")
    boro=BORO.get(r["GEOID"][:5])
    if pop and pop>0 and unc is not None and ri is not None and boro:
        rows.append(dict(pop=pop, R_unc=unc, R_conf=ri-unc, boro=boro,
            white=f(r,"pct_white_nh"), inc=f(r,"median_hh_income"),
            hisp=f(r,"pct_hispanic"), black=f(r,"pct_black_nh")))
w=np.array([r["pop"] for r in rows])

def pcorr(xkey, Rvals):
    xs=[r[xkey] for r in rows]
    pairs=[(x,y,wi) for x,y,wi in zip(xs,Rvals,w) if x is not None]
    xs,ys,ws=map(np.array,zip(*pairs)); sw=ws.sum()
    mx=np.average(xs,weights=ws); my=np.average(ys,weights=ws)
    cov=np.average((xs-mx)*(ys-my),weights=ws)
    vx=np.average((xs-mx)**2,weights=ws); vy=np.average((ys-my)**2,weights=ws)
    return cov/math.sqrt(vx*vy) if vx>0 and vy>0 else float("nan")

def Rset(mode):
    out=[]
    for r in rows:
        if mode=="raw": rec=1.0
        elif mode=="global": rec=g["recall"]
        else: rec=boro_recall.get(r["boro"], g["recall"])
        out.append(r["R_unc"]/rec + r["R_conf"])
    return out

print("\nR_i disparity under undercount corrections (pop-wtd corr with demographics):")
print(f"  {'correction':<26}{'meanR':>8}{'%White':>9}{'income':>9}{'%Hisp':>9}{'%Black':>9}")
corrections={}
for mode,label in [("raw","(a) raw counts (recall=1)"),
                   ("global","(b) global recall"),
                   ("spatial","(c) spatially-varying recall")]:
    R=Rset(mode); meanR=np.average(R,weights=w)
    corrections[mode]={"label":label,"mean_R":float(meanR),
        "corr_white":pcorr('white',R),"corr_income":pcorr('inc',R),
        "corr_hispanic":pcorr('hisp',R),"corr_black":pcorr('black',R)}
    print(f"  {label:<26}{meanR:>8.1f}"
          f"{pcorr('white',R):>+9.3f}{pcorr('inc',R):>+9.3f}{pcorr('hisp',R):>+9.3f}{pcorr('black',R):>+9.3f}")
print(f"\nN = {len(rows)} BGs. Undercount is a CONSERVATIVE lower bound (shared-GSV dependence biases N-hat down).")

OUTDIR="data/derived/results"
os.makedirs(OUTDIR, exist_ok=True)
with open(os.path.join(OUTDIR,"capture_recapture.json"),"w") as fh:
    json.dump({
        "global": {k: (float(v) if isinstance(v,(int,float)) else v) for k,v in g.items()},
        "bootstrap_95": {"N_lo": float(Nlo), "N_hi": float(Nhi),
                         "recall_lo": float(rlo), "recall_hi": float(rhi)},
        "borough_recall": {b: float(r) for b,r in boro_recall.items()},
        # The full bootstrap distribution of the census recall, not just its CI. `sweep_recall.py`
        # pushes each of these draws through the disparity coefficient (cheaply — the correction is
        # linear, so no batch re-run) to get an EXACT propagated band rather than a CI-endpoint
        # approximation.
        "recall_draws": [float(r) for r in RECALL_DRAWS],
        "disparity_under_correction": corrections,
        "n_bg": len(rows),
        "match_radius_m": MATCH_M,
        "caveat": "Both censuses derive from Google Street View -> positive dependence inflates "
                  "overlap m -> biases N-hat DOWN. This is a conservative lower bound.",
    }, fh, indent=2)
print(f"wrote {OUTDIR}/capture_recapture.json")
