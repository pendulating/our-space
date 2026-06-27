# Further-reading cards

Each `*.md` file in this folder is one card in the **Further reading** panel beside the
page header. To add a card, drop a new markdown file here and rebuild (`./web/build.sh`) —
`tools/build_reading.py` bakes every file into `web/dist/reading.json`, which the page
fetches at runtime.

## Format

A YAML-ish front-matter block, then a short blurb:

```markdown
---
title: The exact title of the work
source: Authors / publication · venue · year
kind: paper        # paper | article (drives the small badge)
url: https://…     # where the card links
order: 4           # sort order in the panel (lower = higher up)
---
One or two sentences on why it's worth reading, ideally tying it to the watched commons.
```

`title` and `url` are required; a file missing either is skipped. Filenames are sorted, so
a `NN-` prefix keeps things tidy, but `order:` is what actually sorts the panel.

## Teaser images

Drop an image in `images/` named after the card file — e.g. `images/03-nexar-dashcam-breach.jpg`
for `03-nexar-dashcam-breach.md` — and it becomes the card's teaser banner. JPEG, PNG, WebP,
AVIF, GIF, and SVG all work; or point at a specific file with an `image:` front-matter field.

On rebuild, `tools/build_reading.py` copies the image into `web/dist/reading-img/` and
references it from `reading.json`. **Self-host the image — don't hotlink the source site.**
The panel is about the watched commons; making a visitor's browser fetch a teaser from a
third-party domain just to view it would leak that visit, exactly the kind of thing we map.
To pull a source's social-share image: grab its `og:image`, then downscale to a ~480px-wide
teaser (`magick in.jpg -resize '480x270^' -gravity center -extent 480x270 -strip -quality 82
images/NN-slug.jpg`). For works with no good image (e.g. paywalled papers), an on-theme SVG
placeholder lives alongside the photos.
