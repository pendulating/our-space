#!/usr/bin/env python3
"""Subset the Parabolica Adobe Fonts kit to the glyphs actually used on the page.

Produces self-hosted WOFF2 files in web/dist/fonts/ and a minimal @font-face CSS,
eliminating the render-blocking third-party request to use.typekit.net.

Prereqs:
    pip install fonttools brotli   # or: uv pip install fonttools brotli

Usage:
    python3 tools/subset_fonts.py

The script:
1. Scans web/index.html + web/content/reading/*.md for used characters.
2. Downloads the two Parabolica cuts from the Typekit CSS (requires network).
3. Subsets each to the used glyph set (+ basic Latin fallback).
4. Writes WOFF2 + a local @font-face CSS to web/dist/fonts/.

After running, replace the <link> to use.typekit.net in index.html with:
    <link rel="stylesheet" href="fonts/parabolica.css" />
"""

import re
import sys
import urllib.request
from pathlib import Path

try:
    from fontTools.subset import main as subset_main
except ImportError:
    sys.exit("fonttools not installed: pip install fonttools brotli")

ROOT = Path(__file__).resolve().parent.parent
DIST_FONTS = ROOT / "web" / "dist" / "fonts"

TYPEKIT_CSS_URL = "https://use.typekit.net/vfq0lcs.css"

BASIC_LATIN = (
    " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ"
    "[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~"
    "\u00a0\u00a9\u00ae\u00b7\u2013\u2014\u2018\u2019\u201c\u201d"
    "\u2026\u2192\u2193\u2022\u00b0"
)


def gather_text() -> str:
    texts = [BASIC_LATIN]
    html = ROOT / "web" / "index.html"
    if html.exists():
        texts.append(html.read_text(encoding="utf-8"))
    reading = ROOT / "web" / "content" / "reading"
    if reading.is_dir():
        for md in reading.glob("*.md"):
            texts.append(md.read_text(encoding="utf-8"))
    return "".join(texts)


def fetch_typekit_fonts(css_url: str) -> list[tuple[str, bytes]]:
    """Parse the Typekit CSS and download the WOFF2 sources."""
    req = urllib.request.Request(css_url, headers={"User-Agent": "Mozilla/5.0"})
    css = urllib.request.urlopen(req).read().decode("utf-8")
    fonts = []
    for m in re.finditer(r'url\((https://[^)]+\.woff2)\)', css):
        url = m.group(1)
        name = url.rsplit("/", 1)[-1].split("?")[0]
        data = urllib.request.urlopen(url).read()
        fonts.append((name, data))
    return fonts


def main():
    text = gather_text()
    unicodes = ",".join(f"U+{ord(c):04X}" for c in sorted(set(text)))
    print(f"Glyph set: {len(set(text))} unique characters")

    DIST_FONTS.mkdir(parents=True, exist_ok=True)

    try:
        fonts = fetch_typekit_fonts(TYPEKIT_CSS_URL)
    except Exception as e:
        sys.exit(f"Failed to fetch Typekit fonts: {e}")

    if not fonts:
        sys.exit("No WOFF2 sources found in Typekit CSS")

    for name, data in fonts:
        src = DIST_FONTS / f"_full_{name}"
        src.write_bytes(data)
        out = DIST_FONTS / name
        subset_main([
            str(src),
            f"--unicodes={unicodes}",
            "--flavor=woff2",
            f"--output-file={out}",
            "--layout-features=*",
        ])
        src.unlink()
        saved = len(data) - out.stat().st_size
        print(f"  {name}: {len(data)//1024} KB -> {out.stat().st_size//1024} KB (saved {saved//1024} KB)")

    css_lines = []
    for name, _ in fonts:
        family = "parabolica" if "display" not in name else "parabolica"
        css_lines.append(f"""@font-face {{
  font-family: "parabolica-text";
  src: url("./{name}") format("woff2");
  font-display: swap;
}}""")
    (DIST_FONTS / "parabolica.css").write_text("\n".join(css_lines) + "\n")
    print(f"\nWrote {DIST_FONTS / 'parabolica.css'}")
    print("Replace the Typekit <link> in index.html with:")
    print('  <link rel="stylesheet" href="fonts/parabolica.css" />')


if __name__ == "__main__":
    main()
