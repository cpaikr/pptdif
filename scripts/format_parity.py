#!/usr/bin/env python3
"""Compares the geometry and text formatting pptdif resolves through
layouts, masters, and themes with what PowerPoint itself reports.

Usage: python -I scripts/format_parity.py [--show] FORMATS_EXE DECK...

FORMATS_EXE is the `formats` example (`cargo build -p pptdif --example
formats`). For each slide shape PowerPoint is asked for Left, Top, Width,
and Height, and for each paragraph whose runs share one effective format,
its font size, bold, font name, and alignment. Values pptdif leaves unset
are PowerPoint's own defaults (18 pt, not bold, left-aligned) and are
compared as those. Needs Windows, PowerPoint, and pywin32. Prints counts
only unless --show is given (public fixtures only: --show names the slide,
shape, and property that differ, never text).
"""

from __future__ import annotations

import json
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ALIGN = {1: "l", 2: "ctr", 3: "r", 4: "just", 5: "dist"}


def ours(exe: str, deck: str) -> tuple[dict, dict]:
    proc = subprocess.run([exe, deck], capture_output=True, check=True)
    geometry: dict = {}
    paragraphs: dict = defaultdict(list)
    for line in proc.stdout.decode("utf-8").splitlines():
        item = json.loads(line)
        key = (item["slide"], item["shape"])
        if item["kind"] == "geometry":
            geometry[key] = (item["left"], item["top"], item["width"], item["height"])
        else:
            paragraphs[(*key, item["paragraph"])].append(item)
    return geometry, paragraphs


def walk(shapes):
    for i in range(1, shapes.Count + 1):
        shape = shapes.Item(i)
        yield shape
        if shape.Type == 6:  # msoGroup
            for k in range(1, shape.GroupItems.Count + 1):
                yield shape.GroupItems.Item(k)


def main() -> int:
    import win32com.client as win32

    args = sys.argv[1:]
    show = "--show" in args
    args = [a for a in args if a != "--show"]
    exe, decks = args[0], args[1:]
    app = win32.DispatchEx("PowerPoint.Application")
    compared = differ = 0
    try:
        for deck in decks:
            geometry, paragraphs = ours(exe, deck)
            pres = app.Presentations.Open(str(Path(deck).resolve()), ReadOnly=True, Untitled=False, WithWindow=False)
            try:
                for s in range(1, pres.Slides.Count + 1):
                    for shape in walk(pres.Slides(s).Shapes):
                        key = (s, shape.Id)
                        if key in geometry:
                            mine = geometry[key]
                            theirs = (shape.Left, shape.Top, shape.Width, shape.Height)
                            for name, a, b in zip(("left", "top", "width", "height"), mine, theirs):
                                compared += 1
                                if abs(a - b) > 0.05:
                                    differ += 1
                                    if show:
                                        print(f"slide {s} shape {shape.Id} {name}: {a:.2f} against {b:.2f}")
                        if not shape.HasTextFrame or not shape.TextFrame.HasText:
                            continue
                        text = shape.TextFrame.TextRange
                        for p in range(1, text.Paragraphs().Count + 1):
                            runs = paragraphs.get((s, shape.Id, p), [])
                            if not runs or len({(r["size"], r["bold"], r["font"]) for r in runs}) != 1:
                                continue
                            r = runs[0]
                            para = text.Paragraphs(p)
                            mine = {
                                "size": float((r["size"] or "18pt").removesuffix("pt")),
                                "bold": r["bold"] in ("1", "true"),
                                "align": r["align"] or "l",
                            }
                            theirs = {
                                "size": float(para.Font.Size),
                                "bold": bool(para.Font.Bold),
                                "align": ALIGN.get(para.ParagraphFormat.Alignment, "?"),
                            }
                            if r["font"]:
                                mine["font"] = r["font"]
                                theirs["font"] = str(para.Font.Name)
                            for name in mine:
                                compared += 1
                                if mine[name] != theirs[name]:
                                    differ += 1
                                    if show:
                                        print(f"slide {s} shape {shape.Id} paragraph {p} {name}: "
                                              f"{mine[name]!r} against {theirs[name]!r}")
            finally:
                pres.Close()
    finally:
        if app.Presentations.Count == 0:
            app.Quit()
    print(f"{compared} properties compared, {differ} differ")
    return 1 if differ else 0


if __name__ == "__main__":
    sys.exit(main())
