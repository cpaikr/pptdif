#!/usr/bin/env python3
"""Compares what `pptdif inspect` reads with python-pptx, an independent
reader: per slide, its id, layout, hidden flag, notes, and the counts of
shapes, groups, placeholders, pictures, tables, and charts.

Usage: python -I scripts/parity.py [--show] PPTDIF_EXE DECK...

Prints counts only unless --show is given (public fixtures only: --show
prints slide numbers and the values that differ, never slide text). Exits
1 when any deck disagrees or cannot be read by either side.

Known, intended difference: python-pptx skips shapes inside
`mc:AlternateContent`, which pptdif reads from the first choice. In a deck
with such blocks, differing shape counts are excused and counted; ids,
layouts, visibility, and notes must still agree.
"""

from __future__ import annotations

import json
import subprocess
import sys
import zipfile

from pptx import Presentation

FIELDS = ("id", "layout", "hidden", "notes", "shapes", "groups", "placeholders", "pictures", "tables", "charts")
COUNTS = {"shapes", "groups", "placeholders", "pictures", "tables", "charts"}


def theirs(path: str) -> list[dict]:
    prs = Presentation(path)
    slides = []
    for slide in prs.slides:
        counts = {"shapes": 0, "groups": 0, "placeholders": 0, "pictures": 0, "tables": 0, "charts": 0}

        def walk(shapes) -> None:
            for shape in shapes:
                # By element: shape_type refuses shapes it cannot classify.
                if shape._element.tag.endswith("}grpSp"):
                    counts["groups"] += 1
                    walk(shape.shapes)
                    continue
                counts["shapes"] += 1
                if shape.is_placeholder:
                    counts["placeholders"] += 1
                # A placeholder holding a picture is a picture too.
                if shape._element.tag.endswith("}pic"):
                    counts["pictures"] += 1
                if getattr(shape, "has_table", False) and shape.has_table:
                    counts["tables"] += 1
                if getattr(shape, "has_chart", False) and shape.has_chart:
                    counts["charts"] += 1

        walk(slide.shapes)
        name = slide.slide_layout.name
        slides.append({
            "id": slide.slide_id,
            "layout": name or None,
            "hidden": slide._element.get("show") in ("0", "false"),
            "notes": slide.has_notes_slide,
            **counts,
        })
    return slides


def ours(exe: str, path: str) -> list[dict] | None:
    proc = subprocess.run([exe, "inspect", path], capture_output=True)
    try:
        env = json.loads(proc.stdout.decode("utf-8"))
    except ValueError:
        return None
    if not env.get("ok"):
        return None
    return [{k: s[k] for k in FIELDS} for s in env["result"]["slides"]]


def has_alternate_content(path: str) -> bool:
    with zipfile.ZipFile(path) as z:
        return any(
            n.startswith("ppt/slides/slide") and n.endswith(".xml") and b"AlternateContent" in z.read(n)
            for n in z.namelist()
        )


def main() -> int:
    args = sys.argv[1:]
    show = "--show" in args
    args = [a for a in args if a != "--show"]
    exe, decks = args[0], args[1:]
    agree = differ = unreadable = skipped = 0
    for k, deck in enumerate(decks):
        label = deck if show else f"deck {k + 1}"
        mine = ours(exe, deck)
        try:
            other = theirs(deck)
        except Exception as e:  # noqa: BLE001 - python-pptx failing is a finding, not a crash
            other = None
            if show:
                print(f"{label}: python-pptx failed: {type(e).__name__}")
        if mine is None or other is None:
            unreadable += 1
            print(f"{label}: unreadable by {'pptdif' if mine is None else 'python-pptx'}")
            continue
        problems = []
        excused = 0
        alternate = has_alternate_content(deck)
        if len(mine) != len(other):
            problems.append(f"{len(mine)} slides against {len(other)}")
        for n, (a, b) in enumerate(zip(mine, other), start=1):
            for f in FIELDS:
                if a[f] == b[f]:
                    continue
                # Markup-compatibility blocks change what each reader
                # counts as a shape, never a slide's identity.
                if alternate and f in COUNTS:
                    excused += 1
                else:
                    problems.append(f"slide {n} {f}: {a[f]!r} against {b[f]!r}")
        if not problems:
            agree += 1
            skipped += excused > 0
        else:
            differ += 1
            print(f"{label}: {len(problems)} difference(s)")
            if show:
                for p in problems:
                    print(f"  {p}")
    print(f"{agree} agree ({skipped} with shape counts excused by markup-compatibility blocks), {differ} differ, {unreadable} unreadable")
    return 1 if differ or unreadable else 0


if __name__ == "__main__":
    sys.exit(main())
