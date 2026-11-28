#!/usr/bin/env python3
"""Build Mailcrane brand + app assets.

Run from anywhere:  python3 design/build.py
Needs rsvg-convert, ImageMagick (`convert`) and fontTools.
DM Sans is read from $DMSANS_TTF, or the Google Fonts path below.
Only the generated folders next to this script are deleted and rebuilt.
"""
import os, shutil, subprocess, sys
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.boundsPen import BoundsPen

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import palette  # noqa: E402

OUT = os.path.dirname(os.path.abspath(__file__))
GENERATED = ["brand", "app-icon", "splash", "ui-icons", "illustrations"]
for d in GENERATED:
    shutil.rmtree(os.path.join(OUT, d), ignore_errors=True)

# ---------------------------------------------------------------- palette
# Read from colors.json, the single source (#495): edit the colours there.
C = palette.load()

# ---------------------------------------------------------------- crane mark
# Origami crane on a 100-unit grid. Content bbox x 4..96, y 8..76 (centre 50,42).
# Each facet: (points, role). Roles map to colours per variant.
FACETS = [
    ("48,58 66,58 74,16", "back"),   # far wing
    ("34,58 60,58 44,8", "front"),   # near wing
    ("58,58 70,58 96,30", "back"),   # tail
    ("42,58 32,58 14,26", "front"),  # neck
    ("14,26 4,33 19,31", "front"),   # head / beak
    ("28,58 72,58 50,76", "front"),  # body
    ("50,58 72,58 50,76", "keel"),   # body fold
]
VARIANTS = {
    "white":  {"front": "#FFFFFF", "back": C["tint"], "keel": C["primaryContainer"]},
    "color":  {"front": C["primary"], "back": "#8FA8F0", "keel": C["primaryDark"]},
    "mono":   {"front": "#000000", "back": "#000000@0.45", "keel": "#000000@0.7"},
}

def pts_to_path(pts):
    p = [tuple(map(float, xy.split(","))) for xy in pts.split()]
    return "M" + " L".join(f"{x:g},{y:g}" for x, y in p) + " Z"

def crane_group(variant, cx, cy, scale):
    col = VARIANTS[variant]
    def fill(c):
        c, _, a = c.partition("@")
        return f'fill="{c}"' + (f' fill-opacity="{a}"' if a else "")
    polys = "\n    ".join(
        f'<path d="{pts_to_path(p)}" {fill(col[r])}/>' for p, r in FACETS)
    return (f'<g transform="translate({cx:g} {cy:g}) scale({scale:g}) translate(-50 -42)">\n'
            f'    {polys}\n  </g>')

def svg(w, h, body, vb=None):
    vb = vb or f"0 0 {w} {h}"
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{vb}" '
            f'width="{w}" height="{h}">\n  {body}\n</svg>\n')

def write(path, text):
    full = os.path.join(OUT, path)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w") as f:
        f.write(text)
    return full

def png(src, dst, w, h=None, bg=None):
    full = os.path.join(OUT, dst)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    cmd = ["rsvg-convert", "-w", str(w), "-h", str(h or w), src, "-o", full]
    if bg:
        cmd[1:1] = ["-b", bg]
    subprocess.run(cmd, check=True)
    return full

def strip_alpha(path, bg):
    subprocess.run(["convert", path, "-background", bg, "-alpha", "remove",
                    "-alpha", "off", path], check=True)

# ---------------------------------------------------------------- brand marks
for v in VARIANTS:
    s = write(f"brand/mark-{v}.svg",
              svg(100, 76, crane_group(v, 50, 38, 1.0), vb="0 0 100 76"))
    png(s, f"brand/png/mark-{v}-512.png", 512, 389)

# Wordmark: glyph outlines from DM Sans Bold, so the SVG needs no font installed.
font = TTFont(os.environ.get(
    "DMSANS_TTF", "/usr/share/fonts/truetype/google-fonts/ofl/dmsans/DMSans[opsz,wght].ttf"))
font = instancer.instantiateVariableFont(font, {"wght": 700, "opsz": 14})
gs, cmap = font.getGlyphSet(), font.getBestCmap()
upm = font["head"].unitsPerEm

def text_path(text, tracking=-0.02):
    x, parts = 0, []
    for ch in text:
        g = cmap[ord(ch)]
        pen = SVGPathPen(gs)
        gs[g].draw(pen)
        d = pen.getCommands()
        if d:
            parts.append(f'<path transform="translate({x:g} 0)" d="{d}"/>')
        x += gs[g].width + tracking * upm
    return parts, x - tracking * upm

parts, adv = text_path("Mailcrane")
cap = font["OS/2"].sCapHeight
TEXT_H = 44                         # cap-height in output px
ts = TEXT_H / cap
mark_w = 92 * 1.05                  # crane drawn at scale 1.05 => ~97 px wide
gap = 22
W = int(mark_w + gap + adv * ts + 8)
H = 84
def wordmark(variant, text_col):
    crane = crane_group(variant, mark_w / 2 + 4, H / 2 + 6, 1.05)
    txt = (f'<g fill="{text_col}" transform="translate({mark_w + gap + 4:g} {H/2 + TEXT_H/2:g}) '
           f'scale({ts:.5f} {-ts:.5f})">\n    ' + "\n    ".join(parts) + "\n  </g>")
    return svg(W, H, crane + "\n  " + txt)
for name, v, tc in [("wordmark", "color", C["ink"]), ("wordmark-white", "white", "#FFFFFF"),
                    ("wordmark-mono", "mono", "#000000")]:
    s = write(f"brand/{name}.svg", wordmark(v, tc))
    png(s, f"brand/png/{name}@2x.png", W * 2, H * 2)

# ---------------------------------------------------------------- app icon
# Master on a 1024 square; crane at scale ~6.2 keeps it inside Android's 66/108 safe zone too.
def icon_svg(size=1024, rounded=False):
    r = f' rx="{size*0.2237:g}"' if rounded else ""
    grad = (f'<defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1">'
            f'<stop offset="0" stop-color="#3563E9"/><stop offset="1" stop-color="{C["primary"]}"/>'
            f'</linearGradient></defs>')
    return svg(size, size, grad + f'\n  <rect width="{size}" height="{size}"{r} fill="url(#g)"/>\n  '
               + crane_group("white", size / 2, size * 0.5 + size * 0.01, size / 100 * 0.68))

master = write("app-icon/icon-master.svg", icon_svg())
write("app-icon/icon-preview-rounded.svg", icon_svg(rounded=True))

# Android adaptive: foreground (108dp canvas), background colour, monochrome
def fg_svg(variant, px=108):
    return svg(px, px, crane_group(variant, px / 2, px / 2 + 1, px / 100 * 0.60))
fg = write("app-icon/android/_src/ic_launcher_foreground.svg", fg_svg("white"))
mono = write("app-icon/android/_src/ic_launcher_monochrome.svg", fg_svg("mono"))

def vector_drawable(variant, tint_mono=False):
    col = VARIANTS[variant]
    s = 108 / 100 * 0.60
    paths = []
    for p, r in FACETS:
        pts = [tuple(map(float, xy.split(","))) for xy in p.split()]
        pts = [((x - 50) * s + 54, (y - 42) * s + 55) for x, y in pts]
        d = "M" + " L".join(f"{x:.2f},{y:.2f}" for x, y in pts) + " Z"
        c, _, a = col[r].partition("@")
        alpha = f"{round(float(a or 1) * 255):02X}"
        fill = (alpha + "FFFFFF") if tint_mono else "#" + alpha + c[1:].upper()
        fill = fill if fill.startswith("#") else "#" + fill
        paths.append(f'    <path android:fillColor="{fill}" android:pathData="{d}"/>')
    return ('<?xml version="1.0" encoding="utf-8"?>\n'
            '<vector xmlns:android="http://schemas.android.com/apk/res/android"\n'
            '    android:width="108dp" android:height="108dp"\n'
            '    android:viewportWidth="108" android:viewportHeight="108">\n'
            + "\n".join(paths) + "\n</vector>\n")

res = "app-icon/android/res"
write(f"{res}/drawable/ic_launcher_foreground.xml", vector_drawable("white"))
write(f"{res}/drawable/ic_launcher_monochrome.xml", vector_drawable("mono", tint_mono=True))
write(f"{res}/values/ic_launcher_background.xml",
      '<?xml version="1.0" encoding="utf-8"?>\n<resources>\n'
      f'    <color name="ic_launcher_background">{C["primary"]}</color>\n</resources>\n')
adaptive = ('<?xml version="1.0" encoding="utf-8"?>\n'
            '<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">\n'
            '    <background android:drawable="@color/ic_launcher_background"/>\n'
            '    <foreground android:drawable="@drawable/ic_launcher_foreground"/>\n'
            '    <monochrome android:drawable="@drawable/ic_launcher_monochrome"/>\n'
            '</adaptive-icon>\n')
write(f"{res}/mipmap-anydpi-v26/ic_launcher.xml", adaptive)
write(f"{res}/mipmap-anydpi-v26/ic_launcher_round.xml", adaptive)

# Legacy launcher PNGs (pre-Android 8) — square-ish rounded + round
legacy_sq = write("app-icon/android/_src/legacy_square.svg",
                  svg(192, 192, f'<rect x="8" y="8" width="176" height="176" rx="28" fill="{C["primary"]}"/>\n  '
                      + crane_group("white", 96, 98, 192 / 100 * 0.64)))
legacy_rd = write("app-icon/android/_src/legacy_round.svg",
                  svg(192, 192, f'<circle cx="96" cy="96" r="88" fill="{C["primary"]}"/>\n  '
                      + crane_group("white", 96, 98, 192 / 100 * 0.60)))
for d, px in {"mdpi": 48, "hdpi": 72, "xhdpi": 96, "xxhdpi": 144, "xxxhdpi": 192}.items():
    png(legacy_sq, f"{res}/mipmap-{d}/ic_launcher.png", px)
    png(legacy_rd, f"{res}/mipmap-{d}/ic_launcher_round.png", px)

# Play Store listing icon 512, full-bleed square (Play applies the mask), no alpha
ps = png(master, "app-icon/android/playstore-icon-512.png", 512)
strip_alpha(ps, C["primary"])

# ---------------------------------------------------------------- splash
# Android 12+ splash: icon on 288dp canvas, keep content inside 192dp circle.
sp = write("splash/splash-icon.svg", svg(288, 288, crane_group("white", 144, 146, 288 / 100 * 0.62)))
png(sp, "splash/android/splash_icon_xxxhdpi.png", 1152)
write("splash/splash-icon-color.svg", svg(288, 288, crane_group("color", 144, 146, 288 / 100 * 0.62)))
write("splash/README.txt",
      f"Splash background: {C['primary']} with splash-icon.svg (white),\n"
      f"or white/{C['surface']} with splash-icon-color.svg. Dark mode: {C['darkSurface']} + white icon.\n")

# ---------------------------------------------------------------- UI icons
# 24px grid, 1.75 stroke, round caps/joins. Entries: list of (d, filled?)
F = True
ICONS = {
    "inbox":    ["M3 13h5l1.5 2.5h5L16 13h5",
                 "M5.5 5h13l2.5 8v5a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18v-5z"],
    "mail":     ["M4.5 5h15A1.5 1.5 0 0 1 21 6.5v11a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5v-11A1.5 1.5 0 0 1 4.5 5z",
                 "M3.5 6.5l8.5 6.5 8.5-6.5"],
    # Opened: the flap up, for "Mark as read" next to the closed "mail" (#511).
    "mail_open": ["M3 10.5v8A1.5 1.5 0 0 0 4.5 20h15a1.5 1.5 0 0 0 1.5-1.5v-8",
                  "M3 10.5L12 4l9 6.5", "M3.5 11l8.5 6 8.5-6"],
    "sent":     ["M21 3L10 14", "M21 3l-7 18-4-7-7-4z"],
    "drafts":   ["M14 3H6.5A1.5 1.5 0 0 0 5 4.5v15A1.5 1.5 0 0 0 6.5 21h11a1.5 1.5 0 0 0 1.5-1.5V8z",
                 "M14 3v5h5", "M9 13h6", "M9 17h4"],
    "archive":  ["M3 4.5h18v4H3z", "M5 8.5v10A1.5 1.5 0 0 0 6.5 20h11a1.5 1.5 0 0 0 1.5-1.5v-10", "M10 12h4"],
    "trash":    ["M4 7h16", "M9 7V4.5h6V7", "M6 7l1 12.5A1.5 1.5 0 0 0 8.5 21h7a1.5 1.5 0 0 0 1.5-1.5L18 7",
                 "M10 11v6", "M14 11v6"],
    "star":     ["M12 3.5l2.6 5.3 5.9.9-4.25 4.1 1 5.8L12 16.9l-5.25 2.7 1-5.8L3.5 9.7l5.9-.9z"],
    "spam":     ["M8.2 3h7.6L21 8.2v7.6L15.8 21H8.2L3 15.8V8.2z", "M12 7.5v5",
                 ("M12 15.25a1.1 1.1 0 1 1 0 2.2a1.1 1.1 0 1 1 0-2.2z", F)],
    "label":    ["M3.5 3.5h8l9 9-8 8-9-9z", ("M8 6.9a1.1 1.1 0 1 1 0 2.2a1.1 1.1 0 1 1 0-2.2z", F)],
    "important": ["M4 6.5h11.5l4.5 5.5-4.5 5.5H4l4.5-5.5z"],
    "folder":   ["M3 7.5A1.5 1.5 0 0 1 4.5 6H9l2 2h8.5A1.5 1.5 0 0 1 21 9.5v8a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5z"],
    "filter":   ["M4 5h16l-6 7.5V19l-4-2v-4.5z"],
    "snooze":   ["M12 3a9 9 0 1 1 0 18a9 9 0 1 1 0-18z", "M12 7.5V12l3 2"],
    "search":   ["M11 4a7 7 0 1 1 0 14a7 7 0 1 1 0-14z", "M16 16l4.5 4.5"],
    "compose":  ["M4 20h4L19 9l-4-4L4 16z", "M13.5 6.5l4 4"],
    "reply":    ["M9 7l-5 5 5 5", "M4 12h10a6 6 0 0 1 6 6v1"],
    "forward":  ["M15 7l5 5-5 5", "M20 12H10a6 6 0 0 0-6 6v1"],
    "attach":   ["M20 11.5l-7.8 7.8a5 5 0 0 1-7.1-7.1l8.2-8.2a3.4 3.4 0 0 1 4.8 4.8l-8.2 8.2a1.7 1.7 0 0 1-2.4-2.4l7.4-7.4"],
    "settings": ["M4 7h9", "M17 7h3", "M4 17h3", "M11 17h9",
                 "M15 5a2 2 0 1 1 0 4a2 2 0 1 1 0-4z", "M9 15a2 2 0 1 1 0 4a2 2 0 1 1 0-4z"],
    "account":  ["M12 4a4 4 0 1 1 0 8a4 4 0 1 1 0-8z", "M4.5 20a7.5 7.5 0 0 1 15 0"],
    "menu":     ["M4 7h16", "M4 12h16", "M4 17h16"],
    "more":     [("M12 3.5a1.5 1.5 0 1 1 0 3a1.5 1.5 0 1 1 0-3z", F),
                 ("M12 10.5a1.5 1.5 0 1 1 0 3a1.5 1.5 0 1 1 0-3z", F),
                 ("M12 17.5a1.5 1.5 0 1 1 0 3a1.5 1.5 0 1 1 0-3z", F)],
    "back":     ["M15 5l-7 7 7 7"],
    "close":    ["M6 6l12 12", "M18 6L6 18"],
    "check":    ["M5 12.5l4.5 4.5L19 7.5"],
    "refresh":  ["M20 11a8 8 0 1 0-2.3 5.7", "M20 4.5V11h-6.5"],
    "error":    ["M12 3a9 9 0 1 1 0 18a9 9 0 1 1 0-18z", "M12 7.5v5",
                 ("M12 15.25a1.1 1.1 0 1 1 0 2.2a1.1 1.1 0 1 1 0-2.2z", F)],
}

def norm(e):
    return e if isinstance(e, tuple) else (e, False)

for name, entries in ICONS.items():
    body = []
    for d, filled in map(norm, entries):
        if filled:
            body.append(f'<path d="{d}" fill="currentColor" stroke="none"/>')
        else:
            body.append(f'<path d="{d}"/>')
    write(f"ui-icons/svg/{name}.svg",
          '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" '
          'fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" '
          'stroke-linejoin="round">\n  ' + "\n  ".join(body) + "\n</svg>\n")
    vd = []
    for d, filled in map(norm, entries):
        if filled:
            vd.append(f'    <path android:fillColor="@android:color/white" android:pathData="{d}"/>')
        else:
            vd.append(f'    <path android:strokeColor="@android:color/white" android:strokeWidth="1.75"\n'
                      f'        android:strokeLineCap="round" android:strokeLineJoin="round"\n'
                      f'        android:pathData="{d}"/>')
    write(f"ui-icons/android-vector/ic_mc_{name}.xml",
          '<?xml version="1.0" encoding="utf-8"?>\n'
          '<!-- Tint with app:tint / android:tint, e.g. ?attr/colorOnSurface -->\n'
          '<vector xmlns:android="http://schemas.android.com/apk/res/android"\n'
          '    android:width="24dp" android:height="24dp"\n'
          '    android:viewportWidth="24" android:viewportHeight="24"\n'
          '    android:tint="?android:attr/colorControlNormal">\n' + "\n".join(vd) + "\n</vector>\n")

# ---------------------------------------------------------------- illustrations
def illo(name, extra):
    write(f"illustrations/{name}.svg", svg(240, 200, extra))
    png(os.path.join(OUT, f"illustrations/{name}.svg"), f"illustrations/png/{name}@3x.png", 720, 600)

tray = (f'<path d="M62 118h36l8 14h28l8-14h36" fill="none" stroke="{C["primary"]}" stroke-width="5" '
        f'stroke-linecap="round" stroke-linejoin="round"/>\n  '
        f'<path d="M78 76h84l16 42v30a8 8 0 0 1-8 8H70a8 8 0 0 1-8-8v-30z" fill="none" '
        f'stroke="{C["primary"]}" stroke-width="5" stroke-linejoin="round"/>')
bg = f'<circle cx="120" cy="104" r="88" fill="{C["primaryContainer"]}"/>'
illo("empty-inbox", bg + "\n  " + tray + "\n  " + crane_group("color", 152, 42, 0.62) +
     f'\n  <path d="M58 40q10-6 20 0M42 58q8-5 16 0" fill="none" stroke="{C["tint"]}" '
     f'stroke-width="3" stroke-linecap="round"/>')
illo("no-results", bg + "\n  " +
     f'<circle cx="108" cy="96" r="38" fill="#fff" stroke="{C["primary"]}" stroke-width="6"/>\n  '
     f'<path d="M136 124l30 30" stroke="{C["primary"]}" stroke-width="10" stroke-linecap="round"/>\n  '
     f'<path d="M94 96h28" stroke="{C["tint"]}" stroke-width="6" stroke-linecap="round"/>')
illo("welcome", bg + "\n  " + crane_group("color", 120, 100, 1.25) +
     f'\n  <path d="M40 150q20-10 40 0t40 0 40 0 40 0" fill="none" stroke="{C["tint"]}" '
     f'stroke-width="4" stroke-linecap="round"/>')

print("built", OUT, "wordmark", W, "x", H)
