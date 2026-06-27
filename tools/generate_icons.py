#!/usr/bin/env python3
"""Generate the map sensor-class icons as transparent PNGs.

Original, palette-consistent silhouettes (not third-party logos): a CCTV camera,
an owl (Flock/ALPR), a traffic camera on a mast (NYC DOT), and eyeglasses (smart
glasses). Drawn at 4x and downsampled for anti-aliasing.

Run:  python3 tools/generate_icons.py
Out:  crates/app-interactive/assets/icons/{cctv,owl,dot,glasses}.png
"""
import os
from PIL import Image, ImageDraw, ImageFont

OUT = os.path.join(os.path.dirname(__file__), "..", "crates", "app-interactive", "assets", "icons")
S = 4            # supersample
N = 128          # final size
SZ = N * S

# Palette (RGBA) — matches theme.rs. Warm = surveillance (red→orange ramp, high
# contrast on white paper); cool blue = transit/infrastructure.
SLATE = (0x2a, 0x3a, 0x52, 255)   # cold ink (pictorial silhouettes, bus wheels)
STEEL = (0x41, 0x60, 0x7e, 255)   # ALPR / owl (pictorial)
CYAN = (0x4d, 0x7a, 0x8c, 255)    # DOT cyan-slate (pictorial)
GLASS = (0x34, 0x51, 0x69, 255)   # smart glasses (Tier D slate)
ACE = (0x25, 0x63, 0xeb, 255)     # ACE bus corridor — transit blue (blue-600)
# Surveillance wordmark inks (must match theme.rs map:: tokens).
MAROON = (0x7f, 0x1d, 0x1d, 255)  # CCTV — dense baseline, recedes by value
RED = (0xdc, 0x26, 0x26, 255)     # FLOCK / ALPR — the headline threat
ORANGE_600 = (0xea, 0x58, 0x0c, 255)  # ENF — photo-enforcement
AMBER_700 = (0xb4, 0x53, 0x09, 255)   # DOT — traffic cams
# Internal detail is a cold light-steel notch (never warm paper) so each mark
# stays a restrained, monochrome engraving rather than a flat-design glyph.
LIGHT = (0xc2, 0xce, 0xdb, 235)   # cold highlight (lens centers, glass, windows)


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
    """Bullet CCTV on a wall mount — spare engraved silhouette."""
    img, d = canvas()
    c = SLATE
    d.rounded_rectangle([px(18), px(34), px(30), px(84)], radius=px(4), fill=c)  # mount
    d.rectangle([px(28), px(52), px(44), px(62)], fill=c)                        # arm
    d.rounded_rectangle([px(40), px(46), px(102), px(72)], radius=px(13), fill=c)  # body
    # lens: a single recessed ring (cold light center), no bright glint
    d.ellipse([px(90), px(50), px(110), px(70)], fill=c)
    d.ellipse([px(95), px(55), px(105), px(65)], fill=LIGHT)
    return img


def owl():
    """Owl — austere field-guide silhouette (Flock/ALPR), not a cartoon."""
    img, d = canvas()
    c = STEEL
    # low ear tufts
    d.polygon([(px(40), px(42)), (px(54), px(26)), (px(58), px(48))], fill=c)
    d.polygon([(px(88), px(42)), (px(74), px(26)), (px(70), px(48))], fill=c)
    # head + body as a single upright form
    d.ellipse([px(30), px(34), px(98), px(108)], fill=c)
    # facial disc — a faint cold ring, the only internal detail
    d.ellipse([px(40), px(46), px(88), px(86)], outline=LIGHT, width=px(2))
    # small, level eyes (restrained — no white cartoon discs)
    for ex in (53, 75):
        d.ellipse([px(ex - 5), px(58), px(ex + 5), px(68)], fill=LIGHT)
    return img


def dot():
    """Traffic camera on a mast arm — NYC DOT monitoring cam (spare)."""
    img, d = canvas()
    c = CYAN
    d.rectangle([px(24), px(20), px(33), px(108)], fill=c)                          # mast
    d.rounded_rectangle([px(16), px(102), px(41), px(110)], radius=px(3), fill=c)   # base
    d.rectangle([px(29), px(34), px(86), px(41)], fill=c)                           # arm
    d.rounded_rectangle([px(74), px(40), px(108), px(62)], radius=px(5), fill=c)    # camera box
    d.ellipse([px(98), px(45), px(114), px(61)], fill=c)                            # lens
    d.ellipse([px(102), px(49), px(110), px(57)], fill=LIGHT)
    return img


def glasses():
    """Smart glasses — clean thin engraved frames, no playful accent."""
    img, d = canvas()
    c = GLASS
    w = px(5)
    d.rounded_rectangle([px(16), px(50), px(56), px(80)], radius=px(10), outline=c, width=w)
    d.rounded_rectangle([px(72), px(50), px(112), px(80)], radius=px(10), outline=c, width=w)
    d.line([(px(56), px(60)), (px(72), px(60))], fill=c, width=w)   # bridge
    d.line([(px(16), px(58)), (px(4), px(52))], fill=c, width=w)    # temples
    d.line([(px(112), px(58)), (px(124), px(52))], fill=c, width=w)
    return img


def load_font(size):
    """A bold sans face for the operator wordmarks; falls back to PIL's default."""
    for p in (
        "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
        "/Library/Fonts/Arial Bold.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
    ):
        if os.path.exists(p):
            return ImageFont.truetype(p, size)
    return ImageFont.load_default()


def brand(label, color):
    """A branded wordmark glyph (e.g. 'DOT') in the operator's ink on transparent
    paper — an engraved stamp, not a filled badge, to keep the field-journal
    austerity. Sized to span ~84% of the tile width."""
    img, d = canvas()
    target = int(SZ * 0.84)
    size = SZ // 2
    f = load_font(size)
    w = d.textbbox((0, 0), label, font=f)[2]
    if w > 0:
        size = max(8, int(size * target / w))
        f = load_font(size)
    bbox = d.textbbox((0, 0), label, font=f)
    tw, th = bbox[2] - bbox[0], bbox[3] - bbox[1]
    d.text(((SZ - tw) // 2 - bbox[0], (SZ - th) // 2 - bbox[1]), label, font=f, fill=color)
    return img


def bus():
    """Side-view transit bus — spare silhouette."""
    img, d = canvas()
    c = ACE
    d.rounded_rectangle([px(12), px(40), px(116), px(84)], radius=px(8), fill=c)   # body
    # a single thin window band (cold light), no mullion clutter
    d.rounded_rectangle([px(20), px(48), px(92), px(60)], radius=px(3), fill=LIGHT)
    # door seam
    d.rectangle([px(100), px(48), px(104), px(76)], fill=LIGHT)
    # wheels
    d.ellipse([px(28), px(76), px(46), px(94)], fill=SLATE)
    d.ellipse([px(82), px(76), px(100), px(94)], fill=SLATE)
    return img


if __name__ == "__main__":
    # Pictorial silhouettes (mobile agents still use bus/glasses).
    save(cctv(), "cctv.png")
    save(owl(), "owl.png")
    save(dot(), "dot.png")
    save(glasses(), "glasses.png")
    save(bus(), "bus.png")
    # Branded wordmarks for the fixed map markers + Operators-view chips: the icons
    # become the operator's name, in the operator's ink. Surveillance ramp — distinct
    # in hue and value so the dense layers stay legible on the white ground.
    save(brand("CCTV", MAROON), "brand_cctv.png")
    save(brand("DOT", AMBER_700), "brand_dot.png")
    save(brand("FLOCK", RED), "brand_flock.png")
    save(brand("ENF", ORANGE_600), "brand_enforce.png")
