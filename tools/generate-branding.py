#!/usr/bin/env python3
"""Generate the original RF-Rig branding assets RackForge requires.

RackForge validates three PNGs by exact size: a 512x512 icon, a 1600x400
banner and a 1920x1080 splash. They are drawn here rather than committed as
opaque binaries so the visual identity is reviewable, reproducible, and
unmistakably this project's own work — no manufacturer's artwork, no
trademarks, no photographs of anybody's pedal.

Run:  python tools/generate-branding.py
"""

from __future__ import annotations

import math
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "plugin" / "package" / "branding"

FONT_CANDIDATES = (
    Path("C:/Windows/Fonts/arialbd.ttf"),
    Path("C:/Windows/Fonts/segoeuib.ttf"),
    Path("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"),
)

# The palette also lives in rackforge-plugin.toml; keep the two in step.
BOARD = (18, 16, 14)
BOARD_LIGHT = (38, 34, 30)
ENCLOSURE = (44, 48, 52)
ENCLOSURE_EDGE = (86, 92, 98)
INK = (238, 232, 220)
MUTED = (150, 144, 134)
AMBER = (232, 163, 61)
AMBER_DIM = (128, 88, 32)
CABLE = (26, 24, 22)
STEEL = (176, 178, 180)


def font(size: int) -> ImageFont.FreeTypeFont:
    for candidate in FONT_CANDIDATES:
        if candidate.exists():
            return ImageFont.truetype(str(candidate), size)
    return ImageFont.load_default()


def carpet(size: tuple[int, int]) -> Image.Image:
    """The dark textured field everything sits on: a pedalboard's carpet."""
    width, height = size
    image = Image.new("RGB", size, BOARD)
    draw = ImageDraw.Draw(image)
    step = max(6, height // 90)
    for y in range(0, height, step):
        shade = 6 if (y // step) % 2 == 0 else 0
        draw.line([(0, y), (width, y)], fill=(BOARD[0] + shade, BOARD[1] + shade, BOARD[2] + shade))
    for x in range(0, width, step * 3):
        draw.line([(x, 0), (x, height)], fill=(BOARD[0] + 3, BOARD[1] + 3, BOARD[2] + 3))
    return image


def knob(draw: ImageDraw.ImageDraw, centre: tuple[float, float], radius: float, angle: float) -> None:
    x, y = centre
    draw.ellipse(
        [x - radius, y - radius, x + radius, y + radius],
        fill=(30, 30, 32),
        outline=STEEL,
        width=max(1, int(radius * 0.12)),
    )
    draw.ellipse(
        [x - radius * 0.62, y - radius * 0.62, x + radius * 0.62, y + radius * 0.62],
        fill=(52, 52, 56),
    )
    pointer = (
        x + math.cos(angle) * radius * 0.78,
        y + math.sin(angle) * radius * 0.78,
    )
    draw.line([centre, pointer], fill=AMBER, width=max(2, int(radius * 0.22)))


def stompbox(
    draw: ImageDraw.ImageDraw,
    box: tuple[float, float, float, float],
    knobs: int = 3,
    lit: bool = True,
    label: str | None = None,
) -> None:
    """One pedal seen from above."""
    left, top, right, bottom = box
    width = right - left
    height = bottom - top
    corner = width * 0.09

    draw.rounded_rectangle(box, radius=corner, fill=ENCLOSURE, outline=ENCLOSURE_EDGE, width=max(1, int(width * 0.02)))
    # A brushed highlight across the top third.
    draw.rounded_rectangle(
        [left + width * 0.06, top + height * 0.05, right - width * 0.06, top + height * 0.34],
        radius=corner * 0.6,
        fill=(52, 56, 60),
    )

    # Jacks, one on each side: the whole reason these things chain.
    jack_y = top + height * 0.18
    jack_r = width * 0.055
    for jack_x in (left, right):
        draw.ellipse(
            [jack_x - jack_r, jack_y - jack_r, jack_x + jack_r, jack_y + jack_r],
            fill=(20, 20, 22),
            outline=STEEL,
            width=max(1, int(jack_r * 0.35)),
        )

    radius = width * 0.115
    spacing = width / (knobs + 1)
    for index in range(knobs):
        knob(
            draw,
            (left + spacing * (index + 1), top + height * 0.30),
            radius,
            math.radians(-210 + 55 * index),
        )

    # Status lamp.
    lamp = (left + width * 0.5, top + height * 0.55)
    lamp_r = width * 0.05
    draw.ellipse(
        [lamp[0] - lamp_r, lamp[1] - lamp_r, lamp[0] + lamp_r, lamp[1] + lamp_r],
        fill=AMBER if lit else AMBER_DIM,
    )

    # Footswitch.
    switch = (left + width * 0.5, top + height * 0.78)
    switch_r = width * 0.13
    draw.ellipse(
        [switch[0] - switch_r, switch[1] - switch_r, switch[0] + switch_r, switch[1] + switch_r],
        fill=(28, 28, 30),
        outline=STEEL,
        width=max(1, int(switch_r * 0.22)),
    )
    draw.ellipse(
        [switch[0] - switch_r * 0.55, switch[1] - switch_r * 0.55, switch[0] + switch_r * 0.55, switch[1] + switch_r * 0.55],
        fill=STEEL,
    )

    if label:
        label_font = font(max(9, int(height * 0.075)))
        draw.text(
            (left + width * 0.5, top + height * 0.63),
            label,
            font=label_font,
            fill=MUTED,
            anchor="mm",
        )


def cable(draw: ImageDraw.ImageDraw, start: tuple[float, float], end: tuple[float, float], sag: float, width: int) -> None:
    """A patch cable, drawn as a hanging curve rather than a straight line."""
    points = []
    for step in range(41):
        t = step / 40
        x = start[0] + (end[0] - start[0]) * t
        y = start[1] + (end[1] - start[1]) * t + math.sin(math.pi * t) * sag
        points.append((x, y))
    draw.line(points, fill=CABLE, width=width + 2, joint="curve")
    draw.line(points, fill=(64, 60, 56), width=width, joint="curve")


def make_icon() -> None:
    size = 512
    image = carpet((size, size))
    draw = ImageDraw.Draw(image)

    glow = Image.new("RGB", (size, size), BOARD)
    glow_draw = ImageDraw.Draw(glow)
    glow_draw.ellipse([size * 0.18, size * 0.12, size * 0.82, size * 0.76], fill=(70, 46, 16))
    glow = glow.filter(ImageFilter.GaussianBlur(size * 0.09))
    image = Image.blend(image, glow, 0.5)
    draw = ImageDraw.Draw(image)

    stompbox(draw, (size * 0.22, size * 0.10, size * 0.78, size * 0.80), knobs=3, lit=True)

    name_font = font(int(size * 0.13))
    draw.text((size * 0.5, size * 0.90), "RF-RIG", font=name_font, fill=INK, anchor="mm")

    image.save(OUTPUT / "icon.png")


def make_banner() -> None:
    width, height = 1600, 400
    image = carpet((width, height))
    draw = ImageDraw.Draw(image)

    title = font(150)
    subtitle = font(38)
    draw.text((90, height * 0.40), "RF-RIG", font=title, fill=INK, anchor="lm")
    draw.text((96, height * 0.68), "Pedals, solved from the circuit", font=subtitle, fill=AMBER, anchor="lm")

    # Three pedals in series on the right, cabled together.
    box_width = 190
    box_height = 250
    top = (height - box_height) / 2
    positions = [760, 1010, 1260]
    for index, left in enumerate(positions):
        stompbox(
            draw,
            (left, top, left + box_width, top + box_height),
            knobs=3,
            lit=index != 1,
        )
    jack_y = top + box_height * 0.18
    for index in range(len(positions) - 1):
        cable(
            draw,
            (positions[index] + box_width, jack_y),
            (positions[index + 1], jack_y),
            sag=34,
            width=7,
        )
    cable(draw, (positions[-1] + box_width, jack_y), (width - 30, jack_y + 40), sag=26, width=7)

    image.save(OUTPUT / "banner.png")


def make_splash() -> None:
    width, height = 1920, 1080
    image = carpet((width, height))

    glow = Image.new("RGB", (width, height), BOARD)
    glow_draw = ImageDraw.Draw(glow)
    glow_draw.ellipse([width * 0.12, height * 0.20, width * 0.88, height * 0.92], fill=(64, 42, 14))
    glow = glow.filter(ImageFilter.GaussianBlur(90))
    image = Image.blend(image, glow, 0.55)
    draw = ImageDraw.Draw(image)

    title = font(190)
    subtitle = font(52)
    draw.text((width * 0.5, height * 0.20), "RF-RIG", font=title, fill=INK, anchor="mm")
    draw.text(
        (width * 0.5, height * 0.31),
        "A pedalboard modelled at circuit level",
        font=subtitle,
        fill=AMBER,
        anchor="mm",
    )

    box_width = 250
    box_height = 330
    top = height * 0.46
    gap = 60
    total = box_width * 5 + gap * 4
    start = (width - total) / 2
    labels = ["COMP", "DRIVE", "FUZZ", "DELAY", "REVERB"]
    positions = [start + index * (box_width + gap) for index in range(5)]
    for index, left in enumerate(positions):
        stompbox(
            draw,
            (left, top, left + box_width, top + box_height),
            knobs=3,
            lit=index in (1, 3),
            label=labels[index],
        )
    jack_y = top + box_height * 0.18
    for index in range(len(positions) - 1):
        cable(draw, (positions[index] + box_width, jack_y), (positions[index + 1], jack_y), sag=46, width=9)
    cable(draw, (start - 150, jack_y + 60), (start, jack_y), sag=-40, width=9)
    cable(draw, (positions[-1] + box_width, jack_y), (positions[-1] + box_width + 150, jack_y + 60), sag=40, width=9)

    footer = font(34)
    draw.text(
        (width * 0.5, height * 0.92),
        "compressor  ·  overdrive  ·  distortion  ·  fuzz  ·  chorus  ·  delay  ·  reverb",
        font=footer,
        fill=MUTED,
        anchor="mm",
    )

    image.save(OUTPUT / "splash.png")


def main() -> None:
    OUTPUT.mkdir(parents=True, exist_ok=True)
    make_icon()
    make_banner()
    make_splash()
    for name, expected in (("icon.png", (512, 512)), ("banner.png", (1600, 400)), ("splash.png", (1920, 1080))):
        with Image.open(OUTPUT / name) as image:
            assert image.size == expected, f"{name} is {image.size}, expected {expected}"
            assert image.mode in ("RGB", "RGBA"), f"{name} is {image.mode}"
        print(f"wrote {OUTPUT / name}")


if __name__ == "__main__":
    main()
