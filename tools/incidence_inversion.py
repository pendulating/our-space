#!/usr/bin/env python3
"""M3 — the incidence-inversion test (OUTLINE §8), the novel statistic.

Where does a mobile camera's capture HAPPEN, and who bears it? For every routed home->work
pair, `batch od-exposure-mnl` (with OURSPACE_EMIT_PAIRS) emits the expected dashcam and ACE
encounters for one traversal, split by the borough each 50 m route sample falls in
(nearest block-group centroid): columns m_dash_{bx,bk,mn,qn,si,unk} and m_ace_{...}.
Weighting by the LODES flow gives expected captures per (capture borough, home borough)
cell. Residence-based measurement books all of a person's exposure to where they live;
place-based governance (POST Act notice, signage) books a capture to where the camera is.
The inversion measures the gap: the share of captures that happen outside the bearer's
home borough, and the home-borough mix of what each borough's streets generate.

Until 2026-09-23 this script attributed every capture on a route to the WORK borough,
which conflated "captured on the way" with "captured at the destination" and made the
statistic mostly a function of commute length. The per-borough columns fix that; the
work-borough view is kept as a labelled comparison (destination concentration), not as
the headline.

Outputs (data/derived/results/incidence_inversion.json):
- dash/ace share of expected captures happening outside the bearer's home borough;
- capture-borough x home-borough matrix (row-normalised: who bears what each borough's
  streets generate) and its column-normalised twin (where each borough's residents get
  captured);
- per-borough "captured at home" share for residents (diagonal of the column view);
- the legacy work-borough view for comparison;
- top-10 destination BGs by total dashcam volume with the home-borough mix of their
  commuters (destination concentration, not capture location).
"""
import csv, json, os
from collections import defaultdict

PAIRS = os.environ.get("OURSPACE_PAIRS", "data/derived/exposure/od_pairs_mnl_nyc.csv")
OUT = "data/derived/results/incidence_inversion.json"

BORO = {"36005": "Bronx", "36047": "Brooklyn", "36061": "Manhattan", "36081": "Queens", "36085": "Staten Island"}
SLOT = {"bx": "Bronx", "bk": "Brooklyn", "mn": "Manhattan", "qn": "Queens", "si": "Staten Island", "unk": "unknown"}
ORDER = ["Manhattan", "Brooklyn", "Queens", "Bronx", "Staten Island"]


def boro_of(geoid):
    return BORO.get(geoid[:5], "?")


def norm_rows(cell):
    out = defaultdict(dict)
    tot = defaultdict(float)
    for (a, b), v in cell.items():
        tot[a] += v
    for (a, b), v in cell.items():
        out[a][b] = round(v / tot[a], 4) if tot[a] else 0.0
    return {k: dict(sorted(v.items())) for k, v in sorted(out.items())}


def main():
    with open(PAIRS) as f:
        rd = csv.DictReader(f)
        cols = set(rd.fieldnames or [])
        need = {"home_bg", "work_bg", "jobs", "m_ace", "m_dash"}
        missing = need - cols
        if missing:
            raise SystemExit(f"{PAIRS} lacks {sorted(missing)} — regenerate with OURSPACE_EMIT_PAIRS=<path> batch od-exposure-mnl …")
        boro_cols = [c for c in cols if c.startswith("m_dash_") and c[7:] in SLOT]
        if not boro_cols:
            raise SystemExit(
                f"{PAIRS} has no per-borough capture columns (m_dash_bx …); regenerate with a "
                "batch built after 2026-09-23 so captures are attributed to where they happen."
            )
        cap = {"dash": defaultdict(float), "ace": defaultdict(float)}   # (capture_boro, home_boro)
        work_cell = defaultdict(float)                                    # (work_boro, home_boro), legacy view
        per_work = defaultdict(lambda: defaultdict(float))
        work_tot = defaultdict(float)
        n_rows = 0
        for r in rd:
            hb, wb = boro_of(r["home_bg"]), boro_of(r["work_bg"])
            jobs = float(r["jobs"])
            for kind in ("dash", "ace"):
                for slot, name in SLOT.items():
                    v = float(r.get(f"m_{kind}_{slot}", 0.0) or 0.0) * jobs
                    if v:
                        cap[kind][(name, hb)] += v
            dash = float(r["m_dash"]) * jobs
            work_cell[(wb, hb)] += dash
            per_work[r["work_bg"]][hb] += dash
            work_tot[r["work_bg"]] += dash
            n_rows += 1

    if not cap["dash"]:
        raise SystemExit("no usable pair rows")

    out = {"_readme": {
        "statistic": "expected mobile captures by (borough where the capture happens, home borough of the person captured)",
        "attribution": "each 50 m route sample booked to the borough of its nearest BG centroid (batch od-exposure-mnl, 2026-09-23+)",
        "pairs_file": PAIRS, "n_pairs": n_rows,
        "guardrail": "expected-encounter decomposition weighted by LODES flows; no causal claim; dose per commute traversal",
    }, "n_pairs": n_rows}

    for kind in ("dash", "ace"):
        c = cap[kind]
        tot = sum(c.values())
        unk = sum(v for (b, h), v in c.items() if b == "unknown")
        outside = sum(v for (b, h), v in c.items() if b != h and b != "unknown")
        out[f"{kind}_share_captured_outside_home_boro"] = outside / (tot - unk) if tot > unk else None
        out[f"{kind}_share_unattributed"] = unk / tot if tot else None
        # row view: what borough B's streets generate, by who bears it
        out[f"{kind}_matrix_row_capture_boro_col_home_share"] = norm_rows({k: v for k, v in c.items() if k[0] != "unknown"})
        # column view: where borough H's residents get captured
        out[f"{kind}_matrix_row_home_boro_col_capture_share"] = norm_rows({(h, b): v for (b, h), v in c.items() if b != "unknown"})
        out[f"{kind}_captured_in_home_boro_share_by_home"] = {
            h: out[f"{kind}_matrix_row_home_boro_col_capture_share"].get(h, {}).get(h, 0.0) for h in ORDER
        }

    # Legacy view (pre-2026-09-23 definition), kept for comparison only.
    tot_w = sum(work_cell.values())
    out["legacy_work_boro_view"] = {
        "dash_share_work_boro_ne_home_boro": sum(v for (w, h), v in work_cell.items() if w != h) / tot_w if tot_w else None,
        "dash_matrix_row_work_col_home_share": norm_rows(work_cell),
        "note": "books every capture on a commute to the WORK borough; conflates capture location with destination",
    }

    top_works = sorted(work_tot.items(), key=lambda kv: -kv[1])[:10]
    out["top10_destination_bgs_by_dash_volume"] = [{
        "work_bg": wg, "work_boro": boro_of(wg), "total_expected_dash_captures_on_commutes_to_it": round(vol, 1),
        "home_boro_shares": [{"boro": b, "share": round(v / vol, 4)}
                             for b, v in sorted(per_work[wg].items(), key=lambda kv: -kv[1])[:3]],
    } for wg, vol in top_works]

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w") as f:
        json.dump(out, f, indent=2)

    print(f"M3 incidence-inversion  ({n_rows} pairs)")
    for kind in ("dash", "ace"):
        s = out[f"{kind}_share_captured_outside_home_boro"]
        print(f"  {kind}: captures happening OUTSIDE the bearer's home borough: {s*100:.1f}%   "
              f"(unattributed {out[f'{kind}_share_unattributed']*100:.2f}%)")
    print("  dash: who bears what each borough's streets generate (row = capture borough):")
    m = out["dash_matrix_row_capture_boro_col_home_share"]
    for b in ORDER:
        row = m.get(b, {})
        if row:
            print(f"    {b:<14} <- " + ", ".join(f"{h}:{v*100:.0f}%" for h, v in sorted(row.items(), key=lambda kv: -kv[1])))
    print("  dash: share of residents' captures that happen in their own borough: " +
          ", ".join(f"{h} {v*100:.0f}%" for h, v in out["dash_captured_in_home_boro_share_by_home"].items()))
    lg = out["legacy_work_boro_view"]["dash_share_work_boro_ne_home_boro"]
    print(f"  (legacy work-borough view: {lg*100:.1f}% — for comparison only)")
    print(f"  -> {OUT}")


if __name__ == "__main__":
    main()
