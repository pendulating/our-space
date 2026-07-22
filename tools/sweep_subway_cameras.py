#!/usr/bin/env python3
"""Sensitivity of the mode-mediated disparity to the assumed MTA subway-system camera count.

The central model (batch od-exposure-{modal,mnl}) bakes a subway-camera count per trip:
cams = cams_station·(2+n) + cams_train·(1+n), n = routed boardings - 1 (GTFS subway-graph router,
2026-07-15; capped 3). [Pre-2026-07-15 n was a distance estimate, transfers ~ line_haul/12km.] It also
emits `commute_subway` = the flow-weighted subway contribution to a BG's commute leg. Since the subway
term STILL enters activity exposure A linearly via commute_subway, the reconstruction below is UNCHANGED
and still valid -- we reconstruct A under alternative assumptions from a SINGLE baked run:
  A_without = A_baked - commute_subway                       # subway fully removed (s=0)
  A(scale)  = A_without + scale · commute_subway             # scale the central model ×scale
  A(flat s) = A_without + s · p_transit                      # counterfactual: exactly s cams/trip
We report pop-weighted correlations with demographics and the flat-s flip points. scale=1 = the
baked central model (sanity: matches the table)."""
import csv, math

def load_join():
    pop={r["id"]:float(r["population"]) for r in csv.DictReader(open("data/snapshots/census/bg_centroids_nyc.csv"))}
    acs={}
    for r in csv.DictReader(open("data/snapshots/census/acs_nyc.csv")):
        acs[r["id"]]=r
    out=[]
    for r in csv.DictReader(open("data/derived/exposure/A_i_mnl_bg_nyc.csv")):
        k=r["home_bg"]; a=acs.get(k)
        if k not in pop or pop[k]<=0 or a is None: continue
        pt=a.get("pop_total")
        if not pt or float(pt)<=0: continue
        pt=float(pt)
        out.append({
            "pop":pop[k],
            "A":float(r["A_modal"]), "cs":float(r["commute_subway"]), "ptr":float(r["p_transit"]),
            "white":(float(a["white_nh"])/pt if a.get("white_nh") else None),
            "inc":(float(a["median_hh_income"]) if a.get("median_hh_income") else None),
            "hisp":(float(a["hispanic"])/pt if a.get("hispanic") else None),
            "black":(float(a["black_nh"])/pt if a.get("black_nh") else None),
        })
    return out

rows=load_join()
ws=[r["pop"] for r in rows]

def pcorr(xs,ys):
    p=[(x,y,w) for x,y,w in zip(xs,ys,ws) if x is not None and y is not None]
    if len(p)<3: return float("nan")
    xs,ys,w=zip(*p); sw=sum(w)
    mx=sum(a*x for a,x in zip(w,xs))/sw; my=sum(a*y for a,y in zip(w,ys))/sw
    cov=sum(a*(x-mx)*(y-my) for a,x,y in zip(w,xs,ys))/sw
    vx=sum(a*(x-mx)**2 for a,x in zip(w,xs))/sw; vy=sum(a*(y-my)**2 for a,y in zip(w,ys))/sw
    return cov/math.sqrt(vx*vy) if vx>0 and vy>0 else float("nan")

def A_scale(r,s): return r["A"] + (s-1.0)*r["cs"]          # scale central model ×s
def A_flat(r,s):  return (r["A"]-r["cs"]) + s*r["ptr"]      # exactly s cameras/trip

demos=[("%White","white"),("income","inc"),("%Hisp","hisp"),("%Black","black")]
def meanA(Af,s): return sum(w*Af(r,s) for w,r in zip(ws,rows))/sum(ws)

# The flat-s per-trip count that reproduces the central model's mean subway contribution:
# effective s = (pop-wtd mean subway cams) / (pop-wtd mean P(transit)).
sw=sum(ws)
eff_s = (sum(w*r["cs"] for w,r in zip(ws,rows))/sw) / (sum(w*r["ptr"] for w,r in zip(ws,rows))/sw)

out={"n_bg": len(rows),
     "central_effective_cams_per_trip": eff_s,
     "note": "A(s) reconstructed linearly from the baked run via the commute_subway column: "
             "A_flat(s) = (A - commute_subway) + s*p_transit. corr = pop-weighted Pearson with "
             "activity-space exposure (A_mnl file). Flip = the flat per-trip subway camera count "
             "at which the correlation crosses zero.",
     "scale_grid": {}, "flat_grid": {}, "flips": {}}

print("=== Scale the central (distance-scaled) subway model ×scale ===")
print(f"{'scale':>6}{'meanA':>8} | "+"".join(f"{d:>9}" for d,_ in demos))
for s in [0.0,0.5,1.0,1.5,2.0,3.0]:
    tag=" <- central" if s==1.0 else ""
    cors={d:pcorr([r[k] for r in rows],[A_scale(r,s) for r in rows]) for d,k in demos}
    out["scale_grid"][f"{s:.1f}"]={"mean_A":meanA(A_scale,s),**cors}
    print(f"{s:>6.1f}{meanA(A_scale,s):>8.1f} | "+"".join(f"{cors[d]:>+9.3f}" for d,_ in demos)+tag)

print("\n=== Counterfactual: exactly s subway cameras on every trip ===")
print(f"{'s':>6}{'meanA':>8} | "+"".join(f"{d:>9}" for d,_ in demos))
for s in [0,3,8,12,15,20,30,50]:
    cors={d:pcorr([r[k] for r in rows],[A_flat(r,s) for r in rows]) for d,k in demos}
    out["flat_grid"][str(s)]={"mean_A":meanA(A_flat,s),**cors}
    print(f"{s:>6}{meanA(A_flat,s):>8.1f} | "+"".join(f"{cors[d]:>+9.3f}" for d,_ in demos))

for label,key in demos:
    def c(s): return pcorr([r[key] for r in rows],[A_flat(r,s) for r in rows])
    lo,hi=0.0,300.0
    if not (math.isnan(c(lo)) or math.isnan(c(hi))) and c(lo)*c(hi)<0:
        for _ in range(50):
            m=(lo+hi)/2
            if c(lo)*c(m)<=0: hi=m
            else: lo=m
        out["flips"][label]=(lo+hi)/2
        print(f"  {label}-corr crosses 0 at s ≈ {(lo+hi)/2:.1f} cams/trip")
    else:
        out["flips"][label]=None

import json, os
os.makedirs("data/derived/results", exist_ok=True)
with open("data/derived/results/subway_sweep.json","w") as fh:
    json.dump(out, fh, indent=2)
print(f"\nN = {len(rows)} BGs. Central model effective mean = {eff_s:.1f} cams/trip.")
print("wrote data/derived/results/subway_sweep.json")
