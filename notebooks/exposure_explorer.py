# /// script
# requires-python = ">=3.12"
# dependencies = ["marimo", "altair", "pandas", "numpy"]
# ///
"""Exposure model explorer — an interactive marimo notebook over the block-group
surveillance-exposure outputs in data/derived/exposure/.

Run it:
    uv run marimo edit notebooks/exposure_explorer.py     # interactive
    uv run marimo run  notebooks/exposure_explorer.py     # read-only app

The model (docs/surveillance-exposure-disparity-plan.md):
  R_i      residential exposure — distinct cameras reachable within a 10-minute
           walk of the block group's population-weighted centroid. An OBSERVED
           count, not a recall-corrected estimate: the census finds only ~50% of
           cameras (capture-recapture), but that undercount is non-differential,
           so correcting it rescales exposure ~1.9x without changing the relative
           disparity. See tools/sweep_recall.py; the corrected estimate ships
           alongside as R_i_recall_corrected.
  A_i      activity-space exposure — cameras along commute routes + at job
           destinations, LODES-weighted; three route-choice variants
           (drive-only, fixed modal split, MNL mode choice).
  E_i      combined exposure; counterfactual = excess over a population-neutral
           camera allocation.
"""

import marimo

__generated_with = "0.16.5"
app = marimo.App(width="medium", app_title="Our Space · exposure explorer")


@app.cell
def _():
    import marimo as mo

    return (mo,)


@app.cell
def _(mo):
    mo.md(
        """
        <div style="border-top:3px double #221e16; border-bottom:1px solid #cfcfcf; padding:14px 0 12px; margin-bottom:4px;">
        <div style="font-family:Georgia,serif; font-size:11px; letter-spacing:.18em; text-transform:uppercase; color:#9a3a16; font-weight:700;">Our Space · data desk</div>
        <h1 style="font-family:Georgia,serif; font-weight:600; font-size:34px; line-height:1.1; margin:6px 0 8px; color:#1b1813;">Who lives under the cameras?</h1>
        <p style="font-family:Georgia,serif; font-size:16px; color:#6b6252; max-width:62ch; margin:0;">
        Surveillance exposure for every New York City block group, measured two ways:
        where you <em>live</em> (cameras within a ten-minute walk) and where you
        <em>go</em> (cameras along LODES commute flows). Estimates, not observations —
        every figure carries the model's assumptions.</p>
        </div>
        """
    )
    return


@app.cell
def _():
    import os

    import altair as alt
    import numpy as np
    import pandas as pd

    alt.data_transformers.disable_max_rows()

    # ---- the our-space editorial chart theme (paper, ink, recessive grid) ----
    INK = "#1b1813"
    MUTED = "#6b6252"
    FAINT = "#8c8578"
    RULE = "#e2e2e2"
    RUST = "#9a3a16"
    # Borough identities, fixed order (validated: lightness band, chroma,
    # CVD ΔE 22.1, contrast ≥3:1 on paper).
    BOROUGHS = ["Manhattan", "Brooklyn", "Queens", "Bronx", "Staten Island"]
    BORO_COLORS = ["#dc2626", "#2563eb", "#b45309", "#0d9488", "#9d174d"]
    # Sequential exposure ramp — one rust hue, light → dark (magnitude).
    SEQ = ["#f7ebe4", "#e8c4ad", "#d59a72", "#bc6d3d", "#9a3a16", "#6b2609"]
    # Diverging (counterfactual ±) — cold slate ↔ neutral ↔ rust.
    DIV = ["#34506a", "#7d97b8", "#d6d3d1", "#d59a72", "#9a3a16"]

    @alt.theme.register("ourspace", enable=True)
    def ourspace_theme():
        return {
            "config": {
                "background": "#ffffff",
                "font": "Georgia, 'Times New Roman', serif",
                "title": {
                    "font": "Georgia, serif",
                    "fontSize": 15,
                    "fontWeight": 600,
                    "color": INK,
                    "anchor": "start",
                    "subtitleFont": "Georgia, serif",
                    "subtitleFontSize": 12,
                    "subtitleColor": MUTED,
                },
                "axis": {
                    "labelFont": "Georgia, serif",
                    "labelFontSize": 11,
                    "labelColor": MUTED,
                    "titleFont": "Georgia, serif",
                    "titleFontSize": 11,
                    "titleFontWeight": 400,
                    "titleColor": MUTED,
                    "gridColor": RULE,
                    "gridWidth": 0.6,
                    "domainColor": "#cfcfcf",
                    "tickColor": "#cfcfcf",
                },
                "legend": {
                    "labelFont": "Georgia, serif",
                    "labelFontSize": 11,
                    "labelColor": INK,
                    "titleFont": "Georgia, serif",
                    "titleFontSize": 11,
                    "titleColor": MUTED,
                },
                "view": {"stroke": None},
            }
        }

    ROOT = os.path.normpath(os.path.join(os.path.dirname(__file__), ".."))
    EXP = os.path.join(ROOT, "data", "derived", "exposure")
    return BORO_COLORS, BOROUGHS, DIV, EXP, FAINT, INK, MUTED, RUST, SEQ, alt, np, os, pd


@app.cell
def _(EXP, np, os, pd):
    # ---- load + join the model outputs ----
    table = pd.read_csv(os.path.join(EXP, "exposure_table_nyc.csv"), dtype={"GEOID": str})
    counterfactual = pd.read_csv(os.path.join(EXP, "counterfactual_bg_nyc.csv"), dtype={"id": str})
    covariates = pd.read_csv(os.path.join(EXP, "covariates_bg_nyc.csv"), dtype={"id": str})

    df = table.merge(
        counterfactual[["id", "R_neutral_pop", "excess_pop"]],
        left_on="GEOID",
        right_on="id",
        how="left",
    ).merge(
        covariates[["id", "crime_wsh", "felony_wsh", "transit_dist_m", "req311_wsh"]],
        left_on="GEOID",
        right_on="id",
        how="left",
    )
    df["income_k"] = df["median_hh_income"] / 1000.0
    # A populated block group: the ACS denominator the equity questions need.
    df = df[df["pop_acs"].fillna(0) > 0].copy()
    df["log_R"] = np.log10(df["R_i"].clip(lower=0.1))

    METRICS = {
        "R_i — residential (10-min walkshed)": "R_i",
        "A_drive — activity space, drive routes": "A_drive",
        "A_modal — activity space, fixed mode split": "A_modal",
        "A_mnl — activity space, MNL mode choice": "A_mnl",
        "E_i — combined exposure": "E_i",
        "excess — over population-neutral allocation": "excess_pop",
    }
    return METRICS, df


@app.cell
def _(METRICS, df, mo):
    metric_ui = mo.ui.dropdown(
        options=list(METRICS.keys()),
        value="R_i — residential (10-min walkshed)",
        label="Exposure measure",
    )
    boro_ui = mo.ui.multiselect(
        options=["Manhattan", "Brooklyn", "Queens", "Bronx", "Staten Island"],
        value=["Manhattan", "Brooklyn", "Queens", "Bronx", "Staten Island"],
        label="Boroughs",
    )
    log_ui = mo.ui.switch(value=True, label="log scale")
    mo.hstack([metric_ui, boro_ui, log_ui], justify="start", gap=2)
    return boro_ui, log_ui, metric_ui


@app.cell
def _(METRICS, boro_ui, df, metric_ui):
    metric = METRICS[metric_ui.value]
    d = df[df["borough"].isin(boro_ui.value)].dropna(subset=[metric]).copy()
    d["metric"] = d[metric]
    return d, metric


@app.cell
def _(d, metric, mo):
    import numpy as _np

    _w = d["pop_acs"].to_numpy(dtype=float)
    _v = d["metric"].to_numpy(dtype=float)
    _order = _np.argsort(_v)
    _cw = _np.cumsum(_w[_order]) / _w.sum()
    wmedian = float(_v[_order][_np.searchsorted(_cw, 0.5)])
    top_decile_cut = float(_np.quantile(_v, 0.9))
    top_share = float(_v[_v >= top_decile_cut].sum() / max(_v.sum(), 1e-9))

    def tile(kicker, value, note):
        return mo.md(
            f"""<div style="border-top:2px solid #221e16; padding:8px 14px 10px 0; min-width:180px;">
            <div style="font-family:Georgia,serif; font-size:10px; letter-spacing:.16em; text-transform:uppercase; color:#6b6252; font-weight:700;">{kicker}</div>
            <div style="font-family:Georgia,serif; font-size:30px; font-weight:600; color:#1b1813; font-variant-numeric:tabular-nums;">{value}</div>
            <div style="font-family:Georgia,serif; font-size:11.5px; color:#8c8578;">{note}</div></div>"""
        )

    mo.hstack(
        [
            tile("Block groups", f"{len(d):,}", "populated, in view"),
            tile("Median exposure", f"{wmedian:,.1f}", f"{metric} · population-weighted"),
            tile("Top-decile share", f"{top_share:0.0%}", "of summed exposure held by the most-watched 10%"),
        ],
        justify="start",
        gap=2,
    )
    return


@app.cell
def _(BORO_COLORS, BOROUGHS, alt, d, log_ui, metric, mo):
    _scale = alt.Scale(type="symlog") if log_ui.value else alt.Scale()
    hist = (
        alt.Chart(d[["metric", "borough"]], title=alt.Title(f"How {metric} is distributed", subtitle="One bar per exposure bin; boroughs stacked. Hover for counts."))
        .mark_bar(binSpacing=1)
        .encode(
            x=alt.X("metric:Q", bin=alt.Bin(maxbins=48), title=metric, scale=_scale),
            y=alt.Y("count():Q", title="block groups"),
            color=alt.Color(
                "borough:N",
                scale=alt.Scale(domain=BOROUGHS, range=BORO_COLORS),
                legend=alt.Legend(title=None, orient="top-right"),
            ),
            tooltip=[alt.Tooltip("borough:N"), alt.Tooltip("count():Q", title="block groups")],
        )
        .properties(width=420, height=190)
    )
    ecdf = (
        alt.Chart(d[["metric", "borough"]], title=alt.Title("Cumulative share", subtitle="Reading: y% of block groups sit at or below x."))
        .transform_window(ecdf="cume_dist()", sort=[{"field": "metric"}], groupby=["borough"])
        .mark_line(strokeWidth=2, interpolate="monotone")
        .encode(
            x=alt.X("metric:Q", title=metric, scale=_scale),
            y=alt.Y("ecdf:Q", title="share of block groups", axis=alt.Axis(format="%")),
            color=alt.Color("borough:N", scale=alt.Scale(domain=BOROUGHS, range=BORO_COLORS), legend=None),
            tooltip=[alt.Tooltip("borough:N"), alt.Tooltip("metric:Q", format=",.1f"), alt.Tooltip("ecdf:Q", format=".0%")],
        )
        .properties(width=420, height=190)
    )
    alt.hconcat(hist, ecdf).resolve_scale(color="shared")
    return


@app.cell
def _(SEQ, alt, d, metric, mo, np):
    # ---- the map: BG centroids, sequential rust ramp, quantile-clipped so a few
    # Midtown giants don't wash out the city ----
    _hi = float(np.quantile(d["metric"], 0.98))
    _map = (
        alt.Chart(
            d[["lon", "lat", "metric", "GEOID", "borough", "pop_acs"]],
            title=alt.Title(
                f"{metric} across the city",
                subtitle="Each dot is a block-group centroid (clipped at the 98th percentile). Hover for the estimate.",
            ),
        )
        .mark_circle(size=14, opacity=0.85)
        .encode(
            longitude="lon:Q",
            latitude="lat:Q",
            color=alt.Color(
                "metric:Q",
                scale=alt.Scale(range=SEQ, domainMax=_hi, clamp=True),
                legend=alt.Legend(title=metric, orient="bottom-right", gradientLength=140),
            ),
            tooltip=[
                alt.Tooltip("GEOID:N"),
                alt.Tooltip("borough:N"),
                alt.Tooltip("metric:Q", title=metric, format=",.1f"),
                alt.Tooltip("pop_acs:Q", title="population", format=","),
            ],
        )
        .project(type="mercator")
        .properties(width=640, height=560)
    )
    _map
    return


@app.cell
def _(mo):
    mo.md(
        """
        <div style="border-top:1.5px solid #cfcfcf; margin-top:18px; padding-top:10px;">
        <span style="font-family:Georgia,serif; font-size:11px; letter-spacing:.18em; text-transform:uppercase; color:#9a3a16; font-weight:700;">The equity question</span>
        <p style="font-family:Georgia,serif; font-size:14.5px; color:#34301f; max-width:66ch;">
        Dahir et&nbsp;al. found cameras concentrate in racially diverse neighborhoods.
        The same question, asked of modeled <em>exposure</em>: who carries it? Each dot
        is a block group; the rust line is an ordinary least-squares fit.</p></div>
        """
    )
    return


@app.cell
def _(BORO_COLORS, BOROUGHS, RUST, alt, d, log_ui, metric, mo):
    def equity_panel(xcol, xlabel, fmt=".0%"):
        base = alt.Chart(d[[xcol, "metric", "borough", "GEOID"]].dropna(subset=[xcol])).encode(
            x=alt.X(f"{xcol}:Q", title=xlabel, axis=alt.Axis(format=fmt)),
            y=alt.Y(
                "metric:Q",
                title=metric,
                scale=alt.Scale(type="symlog") if log_ui.value else alt.Scale(),
            ),
        )
        pts = base.mark_circle(size=10, opacity=0.45).encode(
            color=alt.Color("borough:N", scale=alt.Scale(domain=BOROUGHS, range=BORO_COLORS), legend=None),
            tooltip=[
                alt.Tooltip("GEOID:N"),
                alt.Tooltip("borough:N"),
                alt.Tooltip(f"{xcol}:Q", title=xlabel, format=fmt),
                alt.Tooltip("metric:Q", title=metric, format=",.1f"),
            ],
        )
        fit = base.transform_regression(xcol, "metric").mark_line(color=RUST, strokeWidth=2.5)
        return (pts + fit).properties(width=280, height=210)

    _row1 = alt.hconcat(
        equity_panel("pct_black_nh", "share Black (non-Hispanic)"),
        equity_panel("pct_hispanic", "share Hispanic"),
        equity_panel("pct_white_nh", "share white (non-Hispanic)"),
    )
    _row2 = alt.hconcat(
        equity_panel("income_k", "median household income ($k)", fmt="$,.0f"),
        equity_panel("pct_asian_nh", "share Asian (non-Hispanic)"),
        equity_panel("pct_renter", "renter share"),
    )
    alt.vconcat(_row1, _row2)
    return


@app.cell
def _(d, metric, mo, pd):
    # ---- the same relationships as one plain table (color is never the only
    # channel) — Pearson + population-weighted Pearson per covariate ----
    import numpy as _np2

    def _wcorr(x, y, w):
        m = ~(pd.isna(x) | pd.isna(y))
        x, y, w = x[m], y[m], w[m]
        if len(x) < 3:
            return float("nan")
        mx, my = _np2.average(x, weights=w), _np2.average(y, weights=w)
        cov = _np2.average((x - mx) * (y - my), weights=w)
        sx = _np2.sqrt(_np2.average((x - mx) ** 2, weights=w))
        sy = _np2.sqrt(_np2.average((y - my) ** 2, weights=w))
        return float(cov / (sx * sy)) if sx > 0 and sy > 0 else float("nan")

    _covs = [
        ("pct_black_nh", "share Black (NH)"),
        ("pct_hispanic", "share Hispanic"),
        ("pct_white_nh", "share white (NH)"),
        ("pct_asian_nh", "share Asian (NH)"),
        ("income_k", "median income ($k)"),
        ("pct_renter", "renter share"),
        ("crime_wsh", "crime (walkshed)"),
        ("req311_wsh", "311 disorder calls"),
    ]
    _rows = []
    for _c, _label in _covs:
        _rows.append(
            {
                "covariate": _label,
                f"r with {metric}": round(d["metric"].corr(d[_c]), 3),
                "r (population-weighted)": round(
                    _wcorr(d["metric"], d[_c], d["pop_acs"].astype(float)), 3
                ),
            }
        )
    mo.vstack(
        [
            mo.md(
                "<span style='font-family:Georgia,serif;font-size:12px;color:#6b6252;'>"
                "Correlations, not causes — the spatial-econometrics pass "
                "(tools/spatial_econometrics.py) handles confounding structure.</span>"
            ),
            mo.ui.table(pd.DataFrame(_rows), selection=None, pagination=False),
        ]
    )
    return


@app.cell
def _(mo):
    mo.md(
        """
        <div style="border-top:1.5px solid #cfcfcf; margin-top:18px; padding-top:10px;">
        <span style="font-family:Georgia,serif; font-size:11px; letter-spacing:.18em; text-transform:uppercase; color:#9a3a16; font-weight:700;">Does the route model matter?</span>
        <p style="font-family:Georgia,serif; font-size:14.5px; color:#34301f; max-width:66ch;">
        Activity-space exposure under three route-choice assumptions. If the dots hug
        the diagonal, the equity findings are robust to how commuters are routed.</p></div>
        """
    )
    return


@app.cell
def _(BORO_COLORS, BOROUGHS, FAINT, alt, df, mo):
    _m = df[["A_drive", "A_modal", "A_mnl", "borough", "GEOID"]].dropna()

    def _pair(xc, yc):
        _lim = float(max(_m[xc].max(), _m[yc].max()))
        diag = (
            alt.Chart(alt.Data(values=[{"v": 0}, {"v": _lim}]))
            .mark_line(color=FAINT, strokeDash=[4, 4], strokeWidth=1)
            .encode(x=alt.X("v:Q", title=xc), y=alt.Y("v:Q", title=yc))
        )
        pts = (
            alt.Chart(_m)
            .mark_circle(size=9, opacity=0.4)
            .encode(
                x=alt.X(f"{xc}:Q", title=xc),
                y=alt.Y(f"{yc}:Q", title=yc),
                color=alt.Color("borough:N", scale=alt.Scale(domain=BOROUGHS, range=BORO_COLORS), legend=None),
                tooltip=[alt.Tooltip("GEOID:N"), alt.Tooltip(f"{xc}:Q", format=",.1f"), alt.Tooltip(f"{yc}:Q", format=",.1f")],
            )
        )
        r = _m[xc].corr(_m[yc])
        return (diag + pts).properties(
            width=280, height=240, title=alt.Title(f"{xc} vs {yc}", subtitle=f"r = {r:.3f}")
        )

    alt.hconcat(_pair("A_drive", "A_modal"), _pair("A_modal", "A_mnl"), _pair("A_drive", "A_mnl"))
    return


@app.cell
def _(mo):
    mo.md(
        """
        <div style="border-top:1.5px solid #cfcfcf; margin-top:18px; padding-top:10px;">
        <span style="font-family:Georgia,serif; font-size:11px; letter-spacing:.18em; text-transform:uppercase; color:#9a3a16; font-weight:700;">The counterfactual</span>
        <p style="font-family:Georgia,serif; font-size:14.5px; color:#34301f; max-width:66ch;">
        If the city's cameras were spread in proportion to population, who would gain
        and who would lose? Rust = watched <em>more</em> than a neutral allocation
        would predict; slate = less.</p></div>
        """
    )
    return


@app.cell
def _(DIV, alt, df, mo, np):
    _c = df[["lon", "lat", "excess_pop", "GEOID", "borough", "R_i", "R_neutral_pop"]].dropna(subset=["excess_pop"]).copy()
    _lim = float(np.quantile(_c["excess_pop"].abs(), 0.98))
    _cmap = (
        alt.Chart(
            _c,
            title=alt.Title(
                "Excess exposure over a population-neutral allocation",
                subtitle="Diverging at zero; clipped at the 98th percentile of |excess|. Hover for values.",
            ),
        )
        .mark_circle(size=14, opacity=0.85)
        .encode(
            longitude="lon:Q",
            latitude="lat:Q",
            color=alt.Color(
                "excess_pop:Q",
                scale=alt.Scale(range=DIV, domain=[-_lim, _lim], clamp=True),
                legend=alt.Legend(title="excess (cameras)", orient="bottom-right", gradientLength=140),
            ),
            tooltip=[
                alt.Tooltip("GEOID:N"),
                alt.Tooltip("borough:N"),
                alt.Tooltip("R_i:Q", title="actual R_i", format=",.1f"),
                alt.Tooltip("R_neutral_pop:Q", title="neutral R_i", format=",.1f"),
                alt.Tooltip("excess_pop:Q", title="excess", format="+,.1f"),
            ],
        )
        .project(type="mercator")
        .properties(width=640, height=560)
    )
    _cmap
    return


@app.cell
def _(mo):
    mo.md(
        """
        <div style="border-top:1px solid #cfcfcf; margin-top:22px; padding-top:8px; font-family:Georgia,serif; font-size:11.5px; color:#8c8578; line-height:1.5;">
        <strong style="color:#34301f;">Sources & method.</strong> Camera census: Amnesty
        <em>Decode Surveillance NYC</em> + Dahir et&nbsp;al. 2025, cross-source de-duplicated.
        Counts are <em>observed</em>, not recall-corrected — the census finds ~50% of cameras,
        but the undercount is non-differential, so correcting it rescales exposure without
        changing the disparity (<code>tools/sweep_recall.py</code>).
        ALPR via DeFlock/OSM; DOT + enforcement from city
        feeds. Commute flows: Census LODES8. Demographics: ACS 5-year. Pipeline:
        <code>cargo run -p batch -- od-exposure*</code> → data/derived/exposure/.
        Estimates carry confidence tiers; block-group values are modeled, not observed.
        </div>
        """
    )
    return


if __name__ == "__main__":
    app.run()
