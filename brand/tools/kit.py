"""Myos logo kit: stroke masters for every variant (outlined later by Inkscape).

Writes kit/src/<name>.svg plus kit/src/modes.txt (name -> drawing|page),
which tells the outliner whether to crop to the drawing or keep the page.
"""
from pathlib import Path

import sys
ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).parent / "kit-carbon"
SRC = ROOT / "src"
SRC.mkdir(parents=True, exist_ok=True)

CARBON, LIME, CHALK = "#15171C", "#C6F432", "#F4F5F0"
LIME_DEEP = "#86B300"        # lime head on light backgrounds (contrast)
# Rule: lime is never a background; it only colours the mark.

SCHEMES = {                    # name: (body, head)
    "color": (CARBON, LIME_DEEP),     # on light
    "reversed": (CHALK, LIME),        # on carbon / dark
    "carbon": (CARBON, CARBON),       # one ink
    "lime": (LIME, LIME),             # one ink, dark backgrounds only
    "black": ("#000000", "#000000"),
    "white": ("#FFFFFF", "#FFFFFF"),
}


def st(color, w):
    return (f'fill="none" stroke="{color}" stroke-width="{w}" '
            'stroke-linecap="round" stroke-linejoin="round"')


# ---- letters (centre-line geometry, x-height band y 60..148) ----
W = 34


def m(x):
    return (f"M{x} 148V90A30 30 0 0 1 {x+60} 90V148"
            f"M{x+60} 90A30 30 0 0 1 {x+120} 90V148")


def y_arms(x):
    return f"M{x} 60L{x+64} 148L{x+128} 60"


def y_tail(x):
    return f"M{x+64} 148C{x+64} 196 {x+44} 220 {x+12} 228"


def o(x):
    return f"M{x} 104A44 44 0 1 1 {x+88} 104A44 44 0 1 1 {x} 104Z"


def s(x, k=1.12):
    """Two-bowl s, widened by k so its counters open up next to the o."""
    def X(v):
        return f"{x + v * k:.2f}"
    return (f"M{X(66)} 70C{X(58)} 62 {X(48)} 60 {X(36)} 60"
            f"C{X(18)} 60 {X(6)} 70 {X(6)} 82C{X(6)} 96 {X(20)} 101 {X(36)} 104"
            f"C{X(54)} 107 {X(68)} 112 {X(68)} 126C{X(68)} 140 {X(54)} 148 {X(36)} 148"
            f"C{X(22)} 148 {X(10)} 144 {X(2)} 136")


X_M, X_Y, X_O, X_S = 37, 202, 354, 486
WM_W, WM_H = 486 + int(70 * 1.12) + 17 + 37, 262


def wordmark(body, head, with_head=True):
    parts = [f'<path d="{d}" {st(body, W)}/>'
             for d in (m(X_M), y_arms(X_Y), y_tail(X_Y), o(X_O), s(X_S))]
    if with_head:
        parts.append(f'<circle cx="{X_Y + 64}" cy="52" r="22" fill="{head}"/>')
    return "".join(parts)


def symbol(body, head, small=False):
    w, r, cy = (44, 24, 44) if small else (W, 22, 56)
    return (f'<path d="{y_arms(64)}" {st(body, w)}/>'
            f'<path d="{y_tail(64)}" {st(body, w)}/>'
            f'<circle cx="128" cy="{cy}" r="{r}" fill="{head}"/>')


# symbol drawing bbox (normal): x 47..209, y 34..245 -> centre (128, 139.5)
SYM_CX, SYM_CY, SYM_H = 128, 139.5, 211


def svg(w, h, body):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" '
            f'width="{w}" height="{h}">{body}</svg>\n')


def place_symbol(body, head, size, frac, small=False, dy=-0.01):
    """Symbol centred on a size x size canvas, mark height = frac * size."""
    k = size * frac / SYM_H
    tx = size / 2 - SYM_CX * k
    ty = size / 2 - SYM_CY * k + dy * size
    return f'<g transform="translate({tx:.2f} {ty:.2f}) scale({k:.4f})">{symbol(body, head, small)}</g>'


modes = {}


def out(name, doc, mode="drawing"):
    (SRC / f"{name}.svg").write_text(doc)
    modes[name] = mode


for scheme, (body, head) in SCHEMES.items():
    out(f"myos-wordmark-{scheme}", svg(WM_W, WM_H, wordmark(body, head)))
    out(f"myos-symbol-{scheme}", svg(256, 256, symbol(body, head)))
    out(f"myos-symbol-small-{scheme}", svg(256, 256, symbol(body, head, small=True)))
    k = 0.72
    out(f"myos-horizontal-{scheme}", svg(
        256 + 40 + int(WM_W * k), 256,
        symbol(body, head)
        + f'<g transform="translate(296 {128 - 104 * k:.1f}) scale({k})">'
        + wordmark(body, head, with_head=False) + "</g>"))
    ks = 0.5
    sw = max(256, int(WM_W * ks) + 40)
    out(f"myos-stacked-{scheme}", svg(
        sw, 406,
        f'<g transform="translate({(sw - 256) / 2} 0)">{symbol(body, head)}</g>'
        + f'<g transform="translate({(sw - WM_W * ks) / 2:.1f} 250) scale({ks})">'
        + wordmark(body, head, with_head=False) + "</g>"))

# ---- app icons (page mode: keep the full canvas) ----
# iOS: full-bleed 1024 square, system applies the mask.
out("app-icon-ios-1024", svg(1024, 1024,
    f'<rect width="1024" height="1024" fill="{CARBON}"/>'
    + place_symbol(CHALK, LIME, 1024, 0.56)), "page")
# Android adaptive: 108dp canvas (432 px @4x), safe zone 66dp -> mark within ~60 %.
out("android-adaptive-foreground", svg(432, 432,
    place_symbol(CHALK, LIME, 432, 0.40)), "page")
out("android-adaptive-background", svg(432, 432,
    f'<rect width="432" height="432" fill="{CARBON}"/>'), "page")
out("android-adaptive-monochrome", svg(432, 432,
    place_symbol("#000000", "#000000", 432, 0.40)), "page")
# Rounded tile for marketing / desktop.
out("app-icon-tile", svg(1024, 1024,
    f'<rect width="1024" height="1024" rx="230" fill="{CARBON}"/>'
    + place_symbol(CHALK, LIME, 1024, 0.56)), "page")
# Web: favicon uses the small cut on a forest tile (survives light/dark tabs).
out("favicon", svg(256, 256,
    f'<rect width="256" height="256" rx="56" fill="{CARBON}"/>'
    + place_symbol(CHALK, LIME, 256, 0.72, small=True)), "page")
out("maskable-512", svg(512, 512,
    f'<rect width="512" height="512" fill="{CARBON}"/>'
    + place_symbol(CHALK, LIME, 512, 0.46, small=True)), "page")

(SRC / "modes.txt").write_text("".join(f"{k} {v}\n" for k, v in modes.items()))
print(len(modes), "masters")
