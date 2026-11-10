#!/usr/bin/env python3
"""Wallpaper-tinted glass for the COSMIC top bars, like the macOS menu bar.

For a display showing the desktop, its top bar is tinted with the color of the
wallpaper strip behind it (frosted, mostly see-through). If that color would
make the bar's text unreadable in the current light/dark theme, it uses the
matching custom color glass instead (never the Dark/Light styles: those rebuild the bar).

  panel-contrast.py --output eDP-1 --panel Panel-eDP-1   update one bar
  panel-contrast.py                                      update every bar that is
                                                         currently in wallpaper mode
                                                         (run when the wallpaper changes)
Which bars are in wallpaper mode is written by cosmic-maximize-to-workspace to
~/.cache/cosmic-mac/panel-modes.
"""
import argparse
import glob
import io
import os
import re
import subprocess
import sys

from PIL import Image

HOME = os.path.expanduser("~")
C = f"{HOME}/.config/cosmic"
BG_DIR = f"{C}/com.system76.CosmicBackground/v1"
MODES = f"{HOME}/.cache/cosmic-mac/panel-modes"
IMAGE_EXT = (".jpg", ".jpeg", ".png", ".webp", ".jxl", ".avif", ".bmp", ".tif", ".tiff")
STRIP = 0.06          # top 6% of the image sits behind the bar
WALLPAPER_OPACITY = "0.45"


def luminance(rgb):
    r, g, b = rgb
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def open_image(path):
    """Open with Pillow, falling back to ImageMagick for formats like JPEG XL."""
    try:
        return Image.open(path)
    except Exception:
        png = subprocess.run(
            ["magick", path, "-resize", "512x", "png:-"], capture_output=True, check=True
        ).stdout
        return Image.open(io.BytesIO(png))


def image_color(path):
    with open_image(path) as im:
        im = im.convert("RGB")
        w, h = im.size
        strip = im.crop((0, 0, w, max(1, int(h * STRIP)))).resize((64, 4))
        get = getattr(strip, "get_flattened_data", None) or strip.getdata
        pixels = list(get())
    n = len(pixels)
    return tuple(sum(p[i] for p in pixels) / n / 255 for i in range(3))


def wallpaper_entry(output):
    """The wallpaper config for this display, or the one for all displays."""
    fallback = None
    for entry in glob.glob(f"{BG_DIR}/*"):
        try:
            text = open(entry).read()
        except OSError:
            continue
        m = re.search(r'output:\s*"([^"]*)"', text)
        if not m:
            continue
        if m.group(1) == output:
            return text
        if m.group(1) == "all":
            fallback = text
    return fallback


def wallpaper_color(output):
    text = wallpaper_entry(output)
    if not text:
        return None
    path = re.search(r'source:\s*Path\("([^"]+)"\)', text)
    if path:
        p = path.group(1)
        files = [p] if os.path.isfile(p) else sorted(
            f for f in glob.glob(f"{p}/**/*", recursive=True) if f.lower().endswith(IMAGE_EXT)
        )
        colors = []
        for f in files[:20]:
            try:
                colors.append(image_color(f))
            except Exception:
                pass
        if colors:
            return tuple(sum(c[i] for c in colors) / len(colors) for i in range(3))
        return None
    color = re.search(r"source:\s*Color\(\s*Single\(\s*\(?\[?\s*([\d.]+),\s*([\d.]+),\s*([\d.]+)", text)
    if color:
        return tuple(float(c) for c in color.groups())
    return None


def system_is_dark():
    try:
        return open(f"{C}/com.system76.CosmicTheme.Mode/v1/is_dark").read().strip() == "true"
    except OSError:
        return False


def write_if_changed(path, value):
    try:
        if open(path).read().strip() == value:
            return
    except OSError:
        pass
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(value)


def target(output):
    """(r, g, b, opacity) for a display's bar over the desktop, or None."""
    rgb = wallpaper_color(output)
    if rgb is None:
        return None
    lum = luminance(rgb)
    # Always a custom color (never the Dark/Light styles): switching between those and a
    # custom color makes cosmic-panel rebuild the bar, flashing every applet. A color that
    # would clash with the text is darkened (dark theme) or lightened (light theme) instead.
    if system_is_dark() and lum > 0.72:
        k = 0.55 / lum
        rgb = tuple(c * k for c in rgb)
    elif not system_is_dark() and lum < 0.28:
        t = max(0.0, min(1.0, (0.45 - lum) / (1.0 - lum)))
        rgb = tuple(c + (1.0 - c) * t for c in rgb)
    return (*rgb, float(WALLPAPER_OPACITY))


def apply(output, panel):
    t = target(output)
    if t is None:
        print(f"{panel}: could not read the wallpaper for {output}; leaving it as is.")
        return
    value = "Color(({:.3}, {:.3}, {:.3}))".format(*t[:3])
    d = f"{C}/com.system76.CosmicPanel.{panel}/v1"
    write_if_changed(f"{d}/background", value)
    write_if_changed(f"{d}/opacity", WALLPAPER_OPACITY)
    print(f"{panel} ({output}): wallpaper color {tuple(round(c, 2) for c in t[:3])} -> {value}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--output")
    ap.add_argument("--panel")
    ap.add_argument("--print", action="store_true", help="print 'r g b opacity' instead of writing")
    args = ap.parse_args()
    if args.print and args.output:
        t = target(args.output)
        if t is None:
            return 1
        print("{:.4f} {:.4f} {:.4f} {:.3f}".format(*t))
        return 0
    if args.output and args.panel:
        apply(args.output, args.panel)
        return 0
    # Wallpaper changed: refresh every bar that is showing the desktop.
    try:
        lines = open(MODES).read().splitlines()
    except OSError:
        lines = []
    for line in lines:
        parts = line.split()
        if len(parts) == 3 and parts[2] == "wallpaper":
            apply(parts[1], parts[0])
    return 0


if __name__ == "__main__":
    sys.exit(main())
