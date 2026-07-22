#!/usr/bin/env python3
"""Need-neutral counterfactual (§5a): excess = actual - need-neutral exposure, by group.
'Group X experiences N% more surveillance exposure than need-neutral placement predicts.'

Writes data/derived/results/counterfactual.json so the paper's numbers are reproducible and
diff-able rather than living only in a terminal scrollback.
"""
import csv, json, math, os
from collections import defaultdict

OUTDIR = "data/derived/results"
RESULTS = {"blocks": {}, "correlations": {}}

def load(path, key, cols):
    out={}
    with open(path) as f:
        for r in csv.DictReader(f):
            try: out[r[key]]={c:float(r[c]) for c in cols if r.get(c) not in (None,"")}
            except ValueError: pass
    return out

cf = load("data/derived/exposure/counterfactual_bg_nyc.csv","id",
          ["population","R_actual","R_neutral_pop","R_neutral_ambient","excess_pop","excess_ambient"])
acs= load("data/snapshots/census/acs_nyc.csv","id",
          ["pop_total","white_nh","black_nh","hispanic","asian_nh","median_hh_income","below_poverty","pov_universe"])

keys=[k for k in cf if k in acs and cf[k].get("population",0)>0]
def pop(k): return cf[k]["population"]

def pw(ks, f):
    num=den=0.0
    for k in ks:
        v=f(k)
        if v is None: continue
        num+=pop(k)*v; den+=pop(k)
    return num/den if den else float("nan")

# --- verify excess sums to ~0 (pop-weighted) ---
tot_pop=sum(pop(k) for k in keys)
sx_pop=sum(pop(k)*cf[k]["excess_pop"] for k in keys)/tot_pop
sx_amb=sum(pop(k)*cf[k]["excess_ambient"] for k in keys)/tot_pop
print(f"[check] pop-wtd mean excess (should be ~0): pop-proxy={sx_pop:+.4f}  ambient-proxy={sx_amb:+.4f}")
print(f"[check] pop-wtd mean R_actual = {pw(keys,lambda k:cf[k]['R_actual']):.2f}\n")

def pct(k,num):
    p=acs[k].get("pop_total"); v=acs[k].get(num)
    return v/p if p and v is not None else None
def inc(k): return acs[k].get("median_hh_income")

# --- income quintiles (pop-weighted) ---
# Each bin = a fifth of the PEOPLE living in income-reporting BGs. The CDF must be normalized
# by the income-present population, NOT tot_pop: ACS suppresses income in ~1,040 BGs, so
# cum/tot_pop never reaches 1 and Q5 silently became a ~600-BG ultra-rich sliver rather than a
# quintile (fixed 2026-07-14; this is also the construction inequality_stats/make_tables use).
ik=[k for k in keys if inc(k) is not None]
ik.sort(key=inc)
tot_ik=sum(pop(k) for k in ik)
cum=0.0; q={}
for k in ik:
    cum+=pop(k); q[k]=min(int(cum/tot_ik*5),4)
byq=defaultdict(list)
for k in ik: byq[q[k]].append(k)

def block(title, groups):
    print(f"=== {title} ===")
    print(f"{'group':<22}{'R_act':>8}{'R_neu(pop)':>11}{'excess':>9}{'%excess':>9}{'R_neu(amb)':>11}{'exc_amb':>9}{'%exc_a':>8}")
    rec = {}
    for name,ks in groups:
        ra=pw(ks,lambda k:cf[k]['R_actual'])
        rnp=pw(ks,lambda k:cf[k]['R_neutral_pop'])
        rna=pw(ks,lambda k:cf[k]['R_neutral_ambient'])
        ex=pw(ks,lambda k:cf[k]['excess_pop'])
        exa=pw(ks,lambda k:cf[k]['excess_ambient'])
        print(f"{name:<22}{ra:>8.1f}{rnp:>11.1f}{ex:>+9.1f}{(ra/rnp-1)*100:>+8.1f}%{rna:>11.1f}{exa:>+9.1f}{(ra/rna-1)*100:>+7.1f}%")
        rec[name] = {"n_bg": len(ks), "R_actual": ra,
                     "R_neutral_pop": rnp, "excess_pop": ex, "pct_excess_pop": (ra/rnp-1)*100,
                     "R_neutral_ambient": rna, "excess_ambient": exa, "pct_excess_ambient": (ra/rna-1)*100}
    RESULTS["blocks"][title] = rec
    print()

block("Excess exposure by income quintile",
      [(f"Q{i+1} (inc {pw(byq[i],inc):.0f})", byq[i]) for i in range(5)])

# --- race/ethnicity: BGs where a group is the plurality ---
def plurality(k):
    shares={"White":pct(k,"white_nh"),"Black":pct(k,"black_nh"),
            "Hispanic":pct(k,"hispanic"),"Asian":pct(k,"asian_nh")}
    shares={g:v for g,v in shares.items() if v is not None}
    return max(shares,key=shares.get) if shares else None
byr=defaultdict(list)
for k in keys:
    g=plurality(k)
    if g: byr[g].append(k)
block("Excess exposure by plurality race/ethnicity",
      [(f"{g}-plurality (n={len(byr[g])})", byr[g]) for g in ["White","Black","Hispanic","Asian"] if byr[g]])

# --- %group terciles (majority-minority gradient) ---
def terciles(getter, label):
    ks=[k for k in keys if getter(k) is not None]
    ks.sort(key=getter)
    cum=0.0; t={}
    for k in ks:
        cum+=pop(k); t[k]=min(int(cum/sum(pop(x) for x in ks)*3),2)
    g=defaultdict(list)
    for k in ks: g[t[k]].append(k)
    block(f"Excess by {label} tercile (low→high)",
          [(f"T{i+1} ({label} {pw(g[i],getter)*100:.0f}%)", g[i]) for i in range(3)])

terciles(lambda k:pct(k,"hispanic"), "%Hispanic")
terciles(lambda k:pct(k,"black_nh"), "%Black")
terciles(lambda k:pct(k,"white_nh"), "%White")

# --- correlations of excess with demographics ---
def pcorr(ks,xg,yg):
    xs=[];ys=[];ws=[]
    for k in ks:
        x=xg(k);y=yg(k)
        if x is None or y is None: continue
        xs.append(x);ys.append(y);ws.append(pop(k))
    sw=sum(ws)
    if sw==0 or len(xs)<3: return float("nan")
    mx=sum(w*x for w,x in zip(ws,xs))/sw; my=sum(w*y for w,y in zip(ws,ys))/sw
    cov=sum(w*(x-mx)*(y-my) for w,x,y in zip(ws,xs,ys))/sw
    vx=sum(w*(x-mx)**2 for w,x in zip(ws,xs))/sw; vy=sum(w*(y-my)**2 for w,y in zip(ws,ys))/sw
    return cov/math.sqrt(vx*vy) if vx>0 and vy>0 else float("nan")
print("=== Correlation of EXCESS (actual - need-neutral) with demographics ===")
demos=[("%White",lambda k:pct(k,"white_nh")),("income",inc),
       ("%Hispanic",lambda k:pct(k,"hispanic")),("%Black",lambda k:pct(k,"black_nh"))]
print(f"{'':<12}{'excess_pop':>12}{'excess_amb':>12}{'R_actual':>12}")
for dn,dg in demos:
    c = {"excess_pop": pcorr(keys,dg,lambda k:cf[k]['excess_pop']),
         "excess_ambient": pcorr(keys,dg,lambda k:cf[k]['excess_ambient']),
         "R_actual": pcorr(keys,dg,lambda k:cf[k]['R_actual'])}
    RESULTS["correlations"][dn] = c
    print(f"{dn:<12}{c['excess_pop']:>12.3f}{c['excess_ambient']:>12.3f}{c['R_actual']:>12.3f}")
print(f"\nN = {len(keys)} block groups; total pop = {tot_pop/1e6:.2f}M")

RESULTS["n_bg"] = len(keys)
RESULTS["total_population"] = tot_pop
RESULTS["conservation_check"] = {"mean_excess_pop": sx_pop, "mean_excess_ambient": sx_amb}
os.makedirs(OUTDIR, exist_ok=True)
with open(os.path.join(OUTDIR, "counterfactual.json"), "w") as f:
    json.dump(RESULTS, f, indent=2)
print(f"wrote {OUTDIR}/counterfactual.json")
