#!/usr/bin/env python3
"""Verify the our-space wiki: broken wikilinks, orphans, index completeness, frontmatter.

Run from repo root:  python3 .tooling/verify_wiki.py
"""
import os, re
from collections import defaultdict

ROOT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "wiki")
SKIP_DIRS = {".git", ".obsidian", "raw"}  # raw/ is the immutable source layer

def wiki_files():
    out = []
    for dirpath, dirnames, filenames in os.walk(ROOT):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        for f in filenames:
            if f.endswith(".md"):
                out.append(os.path.join(dirpath, f))
    return out

def main():
    files = wiki_files()
    names = {}
    for p in files:
        rel = os.path.relpath(p, ROOT)
        stem = rel[:-3]
        names[stem] = p
        names.setdefault(os.path.basename(p)[:-3], p)
        names.setdefault(rel, p)

    inbound = defaultdict(int)
    broken = []
    for p in files:
        text = open(p, encoding="utf-8").read()
        for m in re.finditer(r"\[\[([^\]|#]+)(?:#[^\]|]*)?(?:\|[^\]]*)?\]\]", text):
            target = m.group(1).strip()
            if p.endswith("SCHEMA.md"):  # schema contains a syntax example
                continue
            if target not in names:
                broken.append((os.path.relpath(p, ROOT), target))
            else:
                inbound[names[target]] += 1

    missing_fm = []
    for p in files:
        b = os.path.basename(p)
        if b in ("README.md", "log.md", "SCHEMA.md", "index.md"):
            continue
        head = open(p, encoding="utf-8").read()[:800]
        if not head.startswith("---") or "updated:" not in head:
            missing_fm.append(os.path.relpath(p, ROOT))

    orphans = sorted(
        os.path.relpath(p, ROOT) for p in files
        if inbound[p] == 0 and os.path.basename(p)[:-3] not in ("index", "SCHEMA", "README", "log", "moc-overview")
    )

    index = open(os.path.join(ROOT, "index.md"), encoding="utf-8").read()
    not_indexed = [
        os.path.relpath(p, ROOT) for p in files
        if os.path.basename(p)[:-3] not in ("README", "SCHEMA", "index", "log")
        and f"[[{os.path.basename(p)[:-3]}" not in index
    ]

    escaped_pipe = []
    for p in files:
        text = open(p, encoding="utf-8").read()
        if re.search(r"\[\[[^\]]*\\\|[^\]]*\]\]", text):
            escaped_pipe.append(os.path.relpath(p, ROOT))

    print(f"files: {len(files)}")
    print(f"\nBROKEN LINKS ({len(broken)}):")
    for f, t in broken:
        print(f"  {f}: [[{t}]]")
    print(f"\nORPHANS ({len(orphans)}):")
    for o in orphans:
        print(f"  {o}")
    print(f"\nNOT IN INDEX ({len(not_indexed)}):")
    for n in not_indexed:
        print(f"  {n}")
    print(f"\nMISSING FRONTMATTER ({len(missing_fm)}):")
    for m in missing_fm:
        print(f"  {m}")
    print(f"\nESCAPED-PIPE LINKS ({len(escaped_pipe)}):")
    for e in escaped_pipe:
        print(f"  {e}")

    bad = sum(len(x) for x in (broken, not_indexed, missing_fm, escaped_pipe))
    print("\nRESULT:", "FAIL" if bad else "PASS", f"({bad} hard issues; {len(orphans)} orphans)")
    return 1 if bad else 0

if __name__ == "__main__":
    raise SystemExit(main())
