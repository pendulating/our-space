#!/usr/bin/env python3
"""M3 — the incidence-inversion test (OUTLINE §8), the novel statistic.

For each work block group j where mobile devices are dense, decompose the
exposure that density generates by the HOME block group of the people it
captures (LODES gives home->work flows directly). Expected headline:

    "Manhattan's rideshare dashcams generate X% of their captures on residents
     of the Bronx and Queens."

Residence-based measurement books 100% of a work-BG's exposure to the BG where
the device sits. The inversion measures who actually bears it — and if the
answer is "commuters from outer boroughs", place-based governance and
place-based measurement are both looking at the wrong address.

Input: the per-pair CSV emitted by `OURSPACE_EMIT_PAIRS=<path> batch
od-exposure-mnl …` (one row per routed home->work pair with street-share-
weighted expected encounters). Each pair row is an EXPECTED-encounters object;
totaling m_dash over all pairs with work_bg == j estimates the dashcam capture
volume generated in j, and splitting by home_bg decomposes it.

Outputs (data/derived/results/incidence_inversion.json):
- citywide: share of ALL mobile captures borne by workers whose home is outside
  the borough where the capture happens (the inversion headline);
- per-borough matrix: captures generated in borough B borne by residents of
  borough H, as shares;
- per-work-BG concentration: for the top-10 destination BGs by total mobile
  volume, the top-3 home boroughs bearing their captures.
"""
import csv, json, os
from collections import defaultdict

PAIRS = os.environ.get("OURSPACE_PAIRS", "data/derived/exposure/od_pairs_mnl_nyc.csv")
ACS = "data/snapshots/census/acs_nyc.csv"
OUT = "data/derived/results/incidence_inversion.json"

BORO = {
    "36005": "Bronx",
    "36047": "Brooklyn",
    "36061": "Manhattan",
    "36081": "Queens",
    "36085": "Staten Island",
}


def boro_of(geoid):
    return BORO.get(geoid[:5], "?")


def main():
    # Homeborough lookup from ACS (every home BG appears there).
    home_boro = {}
    with open(ACS) as f:
        for r in csv.DictReader(f):
            home_boro[r["id"]] = boro_of(r["id"])

    # Aggregate expected encounters onto (work_boro <- home_boro) cells.
    cell = defaultdict(float)          # (work_boro, home_boro) -> sum of m_dash*jobs? no:
    cell_ace = defaultdict(float)      # pair rows are already per-flow encounter objects
    cell_dash = defaultdict(float)
    per_work = defaultdict(lambda: defaultdict(float))  # work_geoid -> home_geoid -> dash
    work_tot = defaultdict(float)
    n_rows = 0
    with open(PAIRS) as f:
        rd = csv.DictReader(f)
        need = {"home_bg", "work_bg", "jobs", "m_ace", "m_dash"}
        missing = need - set(rd.fieldnames or [])
        if missing:
            raise SystemExit(
                f"{PAIRS} lacks {sorted(missing)} — regenerate with "
                "OURSPACE_EMIT_PAIRS=<path> batch od-exposure-mnl …"
            )
        for r in rd:
            hb, wb = r["home_bg"], r["work_bg"]
            jobs = float(r["jobs"])
            ace = float(r["m_ace"]) * jobs   # pair m_* are PER TRAVERSAL; scale by flow to
            dash = float(r["m_dash"]) * jobs  # get the BG-pair's total expected encounters
            hb_boro = home_boro.get(hb, "?")
            wb_boro = boro_of(wb)
            cell[(wb_boro, hb_boro)] += dash
            cell_dash[(wb_boro, hb_boro)] += dash
            cell_ace[(wb_boro, hb_boro)] += ace
            per_work[wb][hb] += dash
            work_tot[wb] += dash
            n_rows += 1

    if not cell:
        raise SystemExit("no usable pair rows")

    out = {"_readme": {
        "statistic": "share of mobile captures generated in borough B borne by residents of borough H",
        "pairs_file": PAIRS,
        "n_pairs": n_rows,
        "guardrail": "expected-encounter decomposition; LODES flows weight it; no causal claim",
    }, "n_pairs": n_rows}

    # Citywide inversion: share of all captures happening OUTSIDE the worker's home borough.
    tot = sum(cell_dash.values())
    out_of_home = sum(v for (wb, hb), v in cell_dash.items() if wb != hb)
    out["dash_share_generated_outside_home_boro"] = out_of_home / tot if tot else None
    tot_ace = sum(cell_ace.values())
    out_of_home_ace = sum(v for (wb, hb), v in cell_ace.items() if wb != hb)
    out["ace_share_generated_outside_home_boro"] = out_of_home_ace / tot_ace if tot_ace else None

    # Borough x borough matrix (row = where the capture happens, col = home of bearer).
    matrix = defaultdict(dict)
    for (wb, hb), v in sorted(cell_dash.items()):
        row_tot = sum(cell_dash[(wb, h)] for (_, h) in [k for k in cell_dash if k[0] == wb])
        matrix[wb][hb] = round(v / row_tot, 4) if row_tot else 0.0
    out["dash_matrix_row_work_col_home_share"] = {k: dict(v) for k, v in matrix.items()}

    # Top destination BGs by total dashcam volume + top-3 home boroughs bearing each.
    top_works = sorted(work_tot.items(), key=lambda kv: -kv[1])[:10]
    tops = []
    for wg, vol in top_works:
        homes = sorted(per_work[wg].items(), key=lambda kv: -kv[1])
        boro_agg = defaultdict(float)
        for hg, v in homes:
            boro_agg[boro_of(hg)] += v
        top3 = sorted(boro_agg.items(), key=lambda kv: -kv[1])[:3]
        tops.append({
            "work_bg": wg,
            "work_boro": boro_of(wg),
            "total_expected_dash_captures": round(vol, 1),
            "home_boro_shares": [
                {"boro": b, "share": round(v / vol, 4)} for b, v in top3
            ],
        })
    out["top10_work_bgs_by_mobile_volume"] = tops

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w") as f:
        json.dump(out, f, indent=2)

    print(f"M3 incidence-inversion  ({n_rows} pairs)")
    s = out["dash_share_generated_outside_home_boro"]
    if s is not None:
        print(f"  dashcam captures generated OUTSIDE the worker's home borough: {s*100:.1f}%")
    s2 = out["ace_share_generated_outside_home_boro"]
    if s2 is not None:
        print(f"  ACE: {s2*100:.1f}%")
    print("  borough matrix (work <- home):")
    for wb in ["Manhattan", "Brooklyn", "Queens", "Bronx", "Staten Island"]:
        row = matrix.get(wb, {})
        if row:
            pretty = ", ".join(f"{h}:{v*100:.0f}%" for h, v in sorted(row.items(), key=lambda kv: -kv[1]))
            print(f"    {wb:<14} <- {pretty}")
    print(f"  -> {OUT}")


if __name__ == "__main__":
    main()
