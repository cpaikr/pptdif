#!/usr/bin/env python3
"""Has PowerPoint write the public fixture under fixtures/powerpoint/.

Usage:
  python -I scripts/powerpoint_fixture.py

Every word in it is invented. PowerPoint builds the deck from nothing and
saves it as `powerpoint-saved.pptx`. It exists because a file-format fact
counts as verified only once it has been seen in a file PowerPoint saved:
`crates/pptdif/tests/powerpoint_saved.rs` reads it and states what was seen.

The file is committed, so this script runs only to change it. It needs
Windows, PowerPoint, and pywin32. PowerPoint stamps a deck with its author
and the folder it was saved in; the script removes the personal
information, saves through a neutral folder, and refuses to finish if the
user's name, account, or home folder appears anywhere in the file.
"""

from __future__ import annotations

import getpass
import io
import os
import re
import shutil
import struct
import sys
import tempfile
import zipfile
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "fixtures" / "powerpoint"

PP_SAVE_AS_OPEN_XML = 24
PP_ALERTS_NONE = 1
# ppRDIAll would also delete the comments the deck exists to show.
PP_RDI_PERSONAL = 4
PP_RDI_PROPERTIES = 8
XL_COLUMN_CLUSTERED = 51
MSO_ANIM_EFFECT_FADE = 10
MSO_TRUE = -1


def png(path: Path) -> None:
    """A 4x4 single-colour PNG, written without any imaging library."""
    width = height = 4
    raw = b"".join(b"\x00" + b"\x2a\x6f\xb4" * width for _ in range(height))

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def build(app, saved: Path, image: Path, signers: list[str]) -> list[str]:
    """Fills a new deck and saves it, adding to `signers` the names Office
    signs with. Returns notes on features this PowerPoint could not write."""
    # Charts and SmartArt fail without a document window (observed
    # 2026-11-26), so the deck shows on screen while it is built.
    pres = app.Presentations.Add(WithWindow=True)
    try:
        return fill(app, pres, saved, image, signers)
    finally:
        # Close only the deck this script made: PowerPoint runs one
        # instance, which may hold the user's own presentations.
        pres.Close()


def fill(app, pres, saved: Path, image: Path, signers: list[str]) -> list[str]:
    """Builds the slides of `pres` and saves it."""
    notes = []
    layouts = pres.SlideMaster.CustomLayouts
    title_layout, content_layout = layouts(1), layouts(2)

    # 1: a title slide with notes.
    s1 = pres.Slides.AddSlide(1, title_layout)
    s1.Shapes.Placeholders(1).TextFrame.TextRange.Text = "Harbour project review"
    s1.Shapes.Placeholders(2).TextFrame.TextRange.Text = "Planning team"
    s1.NotesPage.Shapes.Placeholders(2).TextFrame.TextRange.Text = "Open with the timeline."

    # 2: bullets at two levels, a picture, and a group of two boxes.
    s2 = pres.Slides.AddSlide(2, content_layout)
    s2.Shapes.Placeholders(1).TextFrame.TextRange.Text = "Progress"
    body = s2.Shapes.Placeholders(2).TextFrame.TextRange
    body.Text = "Pier foundations done\rDeck framing started\rRailings ordered"
    body.Paragraphs(3).IndentLevel = 2
    s2.Shapes.AddPicture(str(image), 0, MSO_TRUE, 600, 300, 80, 80)
    a = s2.Shapes.AddTextbox(1, 40, 380, 120, 40)
    a.TextFrame.TextRange.Text = "North"
    b = s2.Shapes.AddTextbox(1, 180, 380, 120, 40)
    b.TextFrame.TextRange.Text = "South"
    s2.Shapes.Range([a.Name, b.Name]).Group()
    s2.TimeLine.MainSequence.AddEffect(s2.Shapes.Placeholders(2), MSO_ANIM_EFFECT_FADE)

    # 3: a table and a chart.
    s3 = pres.Slides.AddSlide(3, content_layout)
    s3.Shapes.Placeholders(1).TextFrame.TextRange.Text = "Budget"
    s3.Shapes.Placeholders(2).Delete()
    table = s3.Shapes.AddTable(3, 2, 40, 120, 400, 120).Table
    for r, row in enumerate([("Item", "Cost"), ("Timber", "120"), ("Steel", "80")], start=1):
        for c, text in enumerate(row, start=1):
            table.Cell(r, c).Shape.TextFrame.TextRange.Text = text
    try:
        chart = s3.Shapes.AddChart2(-1, XL_COLUMN_CLUSTERED, 480, 120, 400, 300).Chart
        chart.ChartData.Activate()
        # The chart's data is a workbook embedded in the deck, which Excel
        # signs with its own user name; PowerPoint's removal does not reach
        # it.
        workbook = chart.ChartData.Workbook
        signers.append(str(workbook.Application.UserName))
        workbook.RemovePersonalInformation = True
        workbook.Close()
    except Exception as e:  # noqa: BLE001 - a missing feature is a note, not a failure
        notes.append(f"no chart: {e}")

    # 4: a hidden slide with a SmartArt diagram and a comment.
    s4 = pres.Slides.AddSlide(4, content_layout)
    s4.Shapes.Placeholders(1).TextFrame.TextRange.Text = "Backup"
    s4.SlideShowTransition.Hidden = MSO_TRUE
    try:
        s4.Shapes.AddSmartArt(app.SmartArtLayouts(1), 40, 120, 400, 250)
    except Exception as e:  # noqa: BLE001
        notes.append(f"no SmartArt: {e}")
    try:
        s4.Comments.Add2(20, 20, "Reviewer", "RV", "Check the totals.", "None", "Reviewer")
    except Exception as e:  # noqa: BLE001
        notes.append(f"no comment: {e}")

    # Two sections.
    props = pres.SectionProperties
    props.AddBeforeSlide(1, "Opening")
    props.AddBeforeSlide(3, "Details")

    pres.RemoveDocumentInformation(PP_RDI_PERSONAL)
    pres.RemoveDocumentInformation(PP_RDI_PROPERTIES)
    pres.SaveAs(str(saved), PP_SAVE_AS_OPEN_XML)
    return notes


def office_user_name() -> str:
    """The name Office signs documents and comments with."""
    import winreg

    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, r"Software\Microsoft\Office\Common\UserInfo") as key:
            return str(winreg.QueryValueEx(key, "UserName")[0])
    except OSError:
        return ""


def personal_strings(user_names: list[str]) -> list[str]:
    """The names a file must not carry: the account, its home folder, the
    machine, and the names Office signs with (whole and in parts)."""
    home = Path.home()
    found = {getpass.getuser(), home.name, str(home), os.environ.get("USERDOMAIN", ""), os.environ.get("COMPUTERNAME", "")}
    for name in user_names:
        found |= {name, *name.replace(",", " ").split()}
    # "user" as an account name would match `userId` in every file. Names
    # in scripts without spaces between words (Korean, Japanese) are short.
    return sorted(s for s in found if len(s) >= 2 and s.lower() not in {"user", "users"})


# Who wrote or last saved a package, in docProps/core.xml.
SIGNATURE = re.compile(rb"<(?:\w+:)?(creator|lastModifiedBy)>\s*([^<\s][^<]*)</")


def leaks(label: str, data: bytes, needles: list[str]) -> list[str]:
    """Personal names in a package and in every package embedded in it (a
    chart's workbook), and any author or last editor recorded at all: Office
    may sign with a name none of the needles spell."""
    out = []
    with zipfile.ZipFile(io.BytesIO(data)) as z:
        for name in z.namelist():
            member = z.read(name)
            where = f"{label}:{name}"
            if member.startswith(b"PK\x03\x04"):
                out += leaks(where, member, needles)
                continue
            for form in ("utf-8", "utf-16-le"):
                text = member.decode(form, errors="ignore").lower()
                out += [f"{where} holds {n!r}" for n in needles if n.lower() in text]
            # Only the field's name is printed, never its value.
            out += [f"{where} records a {m.group(1).decode()}" for m in SIGNATURE.finditer(member)]
    return sorted(set(out))


def main() -> int:
    import win32com.client as win32

    # A neutral folder: PowerPoint records the folder a deck was saved in.
    work = Path(tempfile.mkdtemp(prefix="p", dir=os.environ.get("SystemDrive", "C:") + "\\"))
    try:
        return write(win32, work)
    finally:
        shutil.rmtree(work, ignore_errors=True)


def write(win32, work: Path) -> int:
    saved, image = work / "powerpoint-saved.pptx", work / "dot.png"
    png(image)
    app = win32.DispatchEx("PowerPoint.Application")
    signers = [office_user_name()]
    try:
        app.DisplayAlerts = PP_ALERTS_NONE
        notes = build(app, saved, image, signers)
    finally:
        # Quit only an instance this script alone was using.
        if app.Presentations.Count == 0:
            app.Quit()
    needles = personal_strings(signers)
    problems = leaks(saved.name, saved.read_bytes(), needles)
    if problems:
        print("\n".join(problems))
        print("not written: the file would carry personal information")
        return 1
    OUT.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(saved, OUT / saved.name)
    print(f"wrote {OUT / saved.name} ({saved.stat().st_size} bytes)")
    for n in notes:
        print(f"note: {n}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
