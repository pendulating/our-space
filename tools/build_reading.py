#!/usr/bin/env python3
"""Bake the Further-reading cards: scan web/content/reading/*.md and emit
web/dist/reading.json, which the deployed page fetches at runtime.

Each markdown file is one card — a YAML-ish front-matter block (title, source, kind, url,
order) followed by a short blurb. Drop a new .md in the folder and rebuild to add a card;
see web/content/reading/README.md for the format.

Teaser images: drop an image next to the card in web/content/reading/images/ named after
the card file (e.g. 03-nexar-dashcam-breach.jpg for 03-nexar-dashcam-breach.md), or set an
explicit `image:` field in the front-matter. The image is copied to web/dist/reading-img/
(self-hosted — we never hotlink the source site, so viewing the panel pings no third party)
and referenced from reading.json. SVG/JPEG/PNG/WebP/AVIF/GIF are all accepted.
"""
import json
import pathlib
import shutil

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "web" / "content" / "reading"
IMG_SRC = SRC / "images"
OUT = ROOT / "web" / "dist" / "reading.json"
IMG_OUT = ROOT / "web" / "dist" / "reading-img"
IMG_EXTS = (".jpg", ".jpeg", ".png", ".webp", ".avif", ".gif", ".svg")


def parse(path: pathlib.Path) -> dict:
    """Split a card file into its front-matter fields + a whitespace-collapsed blurb."""
    text = path.read_text(encoding="utf-8")
    meta, blurb = {}, text
    if text.startswith("---"):
        # "---\n<front-matter>\n---\n<blurb>" — maxsplit 2 keeps any '---' in the blurb.
        parts = text.split("---", 2)
        if len(parts) == 3:
            _, front, blurb = parts
            for line in front.strip().splitlines():
                if ":" in line:
                    key, value = line.split(":", 1)
                    meta[key.strip()] = value.strip()
    meta["blurb"] = " ".join(blurb.split())  # collapse newlines/indentation into one line
    return meta


def find_image(stem: str, explicit):
    """Resolve a card's teaser image: an explicit `image:` field (relative to images/ or
    the reading dir) wins; otherwise auto-discover images/<card-stem>.<ext>."""
    candidates = []
    if explicit:
        candidates += [IMG_SRC / explicit, SRC / explicit]
    candidates += [IMG_SRC / f"{stem}{ext}" for ext in IMG_EXTS]
    for c in candidates:
        if c.is_file():
            return c
    return None


def main() -> None:
    cards = []
    shutil.rmtree(IMG_OUT, ignore_errors=True)  # rebuild the teaser set from scratch
    if SRC.is_dir():
        for path in sorted(SRC.glob("*.md")):
            if path.name.lower() == "readme.md":
                continue
            card = parse(path)
            if not (card.get("title") and card.get("url")):
                print(f"reading: skipping {path.name} (missing title or url)")
                continue
            img = find_image(path.stem, card.get("image"))
            if img:
                IMG_OUT.mkdir(parents=True, exist_ok=True)
                shutil.copy2(img, IMG_OUT / img.name)
                card["image"] = f"reading-img/{img.name}"
            else:
                card.pop("image", None)  # drop a dangling/explicit ref with no file
            cards.append(card)
    cards.sort(key=lambda c: (int(c.get("order", "999") or "999"), c.get("title", "")))
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(cards, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    n_img = sum(1 for c in cards if c.get("image"))
    print(f"reading: {len(cards)} card(s) ({n_img} with teaser) -> {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
