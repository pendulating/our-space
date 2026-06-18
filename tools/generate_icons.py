#!/usr/bin/env python3
"""Generate the map sensor-class icons as transparent PNGs.

Original, palette-consistent silhouettes (not third-party logos): a CCTV camera,
an owl (Flock/ALPR), a traffic camera on a mast (NYC DOT), and eyeglasses (smart
glasses). Drawn at 4x and downsampled for anti-aliasing.

Run:  python3 tools/generate_icons.py
Out:  crates/app-interactive/assets/icons/{cctv,owl,dot,glasses}.png
"""
import os
from PIL import Image, ImageDraw

OUT = os.path.join(os.path.dirname(__file__), "..", "crates", "app-interactive", "assets", "icons")
S = 4            # supersample
N = 128          # final size
SZ = N * S

# Palette (RGBA) — matches DESIGN.md.
SLATE = (0x2a, 0x3a, 0x52, 255)   # CCTV (cold panopticon ink)
STEEL = (0x41, 0x60, 0x7e, 255)   # ALPR / owl
CYAN = (0x4d, 0x7a, 0x8c, 255)    # DOT cyan-slate
GLASS = (0x34, 0x51, 0x69, 255)   # smart glasses (Tier D slate)
ACE = (0x72, 0x87, 0xa4, 255)     # ACE bus corridor steel
PAPER = (0xef, 0xe6, 0xd2, 255)   # parchment highlight (lens glints, eyes)
TERRA = (0xa8, 0x54, 0x1f, 255)   # accent


def canvas():
    img = Image.new("RGBA", (SZ, SZ), (0, 0, 0, 0))
    return img, ImageDraw.Draw(img)


def save(img, name):
    os.makedirs(OUT, exist_ok=True)
    img.resize((N, N), Image.LANCZOS).save(os.path.join(OUT, name))
    print("wrote", os.path.join(OUT, name))


def px(v):  # scale a 0..128 coord into supersampled space
    return v * S


def cctv():
    """Bullet CCTV camera, angled down-right on a wall mount."""
    img, d = canvas()
    c = SLATE
    # wall mount bracket
    d.rounded_rectangle([px(20), px(30), px(34), px(86)], radius=px(5), fill=c)
    d.rectangle([px(30), px(50), px(46), px(64)], fill=c)  # arm
    # camera body (tilted bullet): a rounded capsule
    d.rounded_rectangle([px(42), px(44), px(104), px(74)], radius=px(15), fill=c)
    # hood/sunshade lip on top
    d.rounded_rectangle([px(46), px(40), px(100), px(50)], radius=px(5), fill=c)
    # lens
    d.ellipse([px(92), px(50), px(112), px(70)], fill=c)
    d.ellipse([px(96), px(54), px(108), px(66)], fill=PAPER)
    d.ellipse([px(99), px(57), px(105), px(63)], fill=SLATE)
    return img


def owl():
    """Owl silhouette with two big eyes — the Flock/ALPR mark."""
    img, d = canvas()
    c = STEEL
    # ear tufts
    d.polygon([(px(36), px(40)), (px(52), px(20)), (px(56), px(46))], fill=c)
    d.polygon([(px(92), px(40)), (px(76), px(20)), (px(72), px(46))], fill=c)
    # body / head (one rounded blob)
    d.ellipse([px(28), px(30), px(100), px(112)], fill=c)
    # eyes
    for ex in (50, 78):
        d.ellipse([px(ex - 14), px(46), px(ex + 14), px(74)], fill=PAPER)
        d.ellipse([px(ex - 7), px(53), px(ex + 7), px(67)], fill=SLATE)
        d.ellipse([px(ex - 4), px(56), px(ex + 2), px(62)], fill=PAPER)
    # beak
    d.polygon([(px(64), px(66)), (px(58), px(78)), (px(70), px(78))], fill=TERRA)
    return img


def dot():
    """Traffic camera on a mast arm — NYC DOT monitoring cam."""
    img, d = canvas()
    c = CYAN
    # mast
    d.rectangle([px(24), px(20), px(34), px(108)], fill=c)
    d.rounded_rectangle([px(16), px(100), px(42), px(110)], radius=px(4), fill=c)  # base
    # horizontal arm
    d.rectangle([px(30), px(34), px(86), px(42)], fill=c)
    # camera box hanging off the arm
    d.rounded_rectangle([px(74), px(40), px(108), px(64)], radius=px(6), fill=c)
    # lens
    d.ellipse([px(98), px(46), px(114), px(62)], fill=c)
    d.ellipse([px(101), px(49), px(111), px(59)], fill=PAPER)
    d.ellipse([px(104), px(52), px(108), px(56)], fill=CYAN)
    return img


def glasses():
    """Tech eyeglasses — the smart-glasses class."""
    img, d = canvas()
    c = GLASS
    w = px(6)
    # lenses (rounded rects)
    d.rounded_rectangle([px(16), px(48), px(56), px(82)], radius=px(12), outline=c, width=w)
    d.rounded_rectangle([px(72), px(48), px(112), px(82)], radius=px(12), outline=c, width=w)
    # bridge
    d.line([(px(56), px(58)), (px(72), px(58))], fill=c, width=w)
    # temples
    d.line([(px(16), px(56)), (px(4), px(50))], fill=c, width=w)
    d.line([(px(112), px(56)), (px(124), px(50))], fill=c, width=w)
    # a small "recording" glint on the right lens (tech tell)
    d.ellipse([px(100), px(52), px(108), px(60)], fill=TERRA)
    return img


def bus():
    """Side-view bus — the ACE camera-enforcement buses."""
    img, d = canvas()
    c = ACE
    # body
    d.rounded_rectangle([px(10), px(38), px(118), px(86)], radius=px(10), fill=c)
    # windows (parchment band)
    d.rounded_rectangle([px(18), px(46), px(110), px(64)], radius=px(4), fill=PAPER)
    # window mullions
    for x in (40, 62, 84):
        d.rectangle([px(x), px(46), px(x + 3), px(64)], fill=c)
    # wheels
    d.ellipse([px(26), px(78), px(46), px(98)], fill=SLATE)
    d.ellipse([px(82), px(78), px(102), px(98)], fill=SLATE)
    # front ACE camera nub (terracotta tell)
    d.ellipse([px(108), px(40), px(118), px(50)], fill=TERRA)
    return img


if __name__ == "__main__":
    save(cctv(), "cctv.png")
    save(owl(), "owl.png")
    save(dot(), "dot.png")
    save(glasses(), "glasses.png")
    save(bus(), "bus.png")
