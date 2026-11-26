#!/usr/bin/env python3
"""Repository checks: skill package rules, Rust format/lint/tests, a CLI
smoke run of the built executable, and reader parity against python-pptx.

Usage: python scripts/check.py

Standard library only, except that the parity check runs scripts/parity.py,
which needs python-pptx, and the format check (Windows with PowerPoint and
pywin32 only) runs scripts/format_parity.py. Output never names private decks or prints their
contents.
"""

from __future__ import annotations

import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SKILLS = ROOT / "skills"
PRIVATE = ROOT / "fixtures" / "private"
SAVED = ROOT / "fixtures" / "powerpoint"
ALLOWED_FIELDS = {"name", "description", "license", "compatibility", "metadata", "allowed-tools"}
EXEMPT_DIRS = {"agents", "evals"}

errors: list[str] = []


def fail(msg: str) -> None:
    errors.append(msg)
    print(f"FAIL {msg}")


def check_skill(skill: Path) -> None:
    entry = skill / "SKILL.md"
    if not entry.is_file():
        fail(f"{skill.name}: missing SKILL.md")
        return
    text = entry.read_text(encoding="utf-8")
    m = re.match(r"---\n(.*?)\n---\n", text, re.S)
    if not m:
        fail(f"{skill.name}: SKILL.md lacks YAML frontmatter")
        return
    fields = {}
    for line in m.group(1).splitlines():
        key, _, value = line.partition(":")
        fields[key.strip()] = value.strip()
    if set(fields) - ALLOWED_FIELDS:
        fail(f"{skill.name}: unsupported frontmatter fields {sorted(set(fields) - ALLOWED_FIELDS)}")
    if fields.get("name") != skill.name:
        fail(f"{skill.name}: frontmatter name {fields.get('name')!r} must match directory")
    if not fields.get("description"):
        fail(f"{skill.name}: missing description")
    body = text[m.end():]
    for link in re.findall(r"\]\(<?([^)>]+)>?\)", body):
        if re.match(r"[a-z]+://", link) or link.startswith("#"):
            continue
        target = (skill / link).resolve()
        if not target.is_relative_to(skill.resolve()):
            fail(f"{skill.name}: link escapes the skill directory: {link}")
        elif not target.exists():
            fail(f"{skill.name}: broken link {link}")
    for path in skill.rglob("*"):
        rel = path.relative_to(skill)
        if path.is_symlink():
            fail(f"{skill.name}: symlink {rel}")
        if path.is_file() and rel.parts[0] not in EXEMPT_DIRS and rel.name != "SKILL.md" and "__pycache__" not in rel.parts:
            if rel.as_posix() not in body:
                fail(f"{skill.name}: runtime file {rel.as_posix()} is not named in SKILL.md")
    print(f"ok   package {skill.name}")


def run_cargo() -> None:
    for args in (
        ["fmt", "--all", "--check"],
        ["clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"],
        ["test", "--workspace", "--locked"],
    ):
        proc = subprocess.run(["cargo", *args], cwd=ROOT)
        if proc.returncode != 0:
            fail(f"cargo {args[0]}")
        else:
            print(f"ok   cargo {args[0]}")


def target_dir() -> Path:
    """Cargo's target directory, honoring CARGO_TARGET_DIR."""
    proc = subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=ROOT, capture_output=True)
    return Path(json.loads(proc.stdout)["target_directory"]) if proc.returncode == 0 else ROOT / "target"


TARGET = target_dir()
EXE = TARGET / "debug" / ("pptdif.exe" if sys.platform == "win32" else "pptdif")


def cli_smoke() -> None:
    """The built executable honors the process contract outside the test harness."""
    if not EXE.is_file():
        fail(f"{EXE} is missing; cargo test builds it")
        return
    before = len(errors)
    help_run = subprocess.run([str(EXE)], capture_output=True)
    if help_run.returncode != 0 or not help_run.stdout.startswith(b"Compare PowerPoint presentations"):
        fail("smoke: bare pptdif does not print help")
    bad = subprocess.run([str(EXE), "no-such-command"], capture_output=True)
    try:
        env = json.loads(bad.stdout.decode("utf-8"))
    except ValueError:
        env = {}
    if bad.returncode != 2 or env.get("error", {}).get("code") != "invalid_invocation":
        fail("smoke: invalid invocation is not a JSON failure with exit 2")
    # The HTML report of the PowerPoint-saved deck renders self-contained.
    deck = next(iter(sorted(SAVED.glob("*.pptx"))), None)
    if deck is not None:
        report = TARGET / "pptdif-smoke-report.html"
        run = subprocess.run(
            [str(EXE), "compare", str(deck), str(deck), "--format", "html", "-o", str(report), "--force"],
            capture_output=True,
        )
        html = report.read_text(encoding="utf-8") if report.is_file() else ""
        if run.returncode != 0 or not html.startswith("<!doctype html>") or re.search(r"(src|href)=\"https?:", html):
            fail("smoke: the HTML report did not render self-contained")
        report.unlink(missing_ok=True)
    if len(errors) == before:
        print("ok   cli smoke")


def private_decks() -> list[Path]:
    if not PRIVATE.is_dir():
        return []
    return sorted(p for p in PRIVATE.rglob("*") if p.suffix.lower() in {".pptx", ".pptm"} and not p.name.startswith("~$"))


def parity() -> None:
    """The reader agrees with python-pptx on the public fixtures, the
    PowerPoint-saved deck, and the private corpus. The private run prints
    counts only (see scripts/parity.py)."""
    have_pptx = subprocess.run([sys.executable, "-I", "-c", "import pptx"], capture_output=True).returncode == 0
    if not have_pptx:
        fail("parity: python-pptx is not installed (pip install python-pptx)")
        return
    # A directory of pptdif's own, emptied first, so no stale or foreign
    # deck from a shared target directory joins the run.
    fixtures = TARGET / "pptdif-parity-fixtures"
    shutil.rmtree(fixtures, ignore_errors=True)
    proc = subprocess.run(
        ["cargo", "run", "-q", "--locked", "-p", "pptdif-fixtures", "--example", "write_fixtures", "--", str(fixtures)],
        cwd=ROOT,
    )
    if proc.returncode != 0:
        fail("parity: writing the fixtures failed")
        return
    script = str(ROOT / "scripts" / "parity.py")
    public = sorted(str(p) for p in [*fixtures.glob("*.pptx"), *SAVED.glob("*.pptx")])
    # Public decks may show details; the private corpus reports counts only.
    runs = [("public fixtures", ["--show"], public)]
    private = [str(p) for p in private_decks()]
    if private:
        runs.append((f"private corpus ({len(private)} decks)", [], private))
    for label, flags, decks in runs:
        proc = subprocess.run([sys.executable, "-I", script, *flags, str(EXE), *decks])
        if proc.returncode != 0:
            fail(f"parity: {label}")
        else:
            print(f"ok   parity {label}")


def compare_smoke() -> None:
    """On the private corpus: every deck compared with itself shows no
    changes, and each pair of decks in one folder compares both ways without
    error, with the slide counts mirrored. Prints counts only."""
    decks = private_decks()
    if not decks:
        return

    def run(a: Path, b: Path) -> dict | None:
        proc = subprocess.run([str(EXE), "compare", str(a), str(b), "--limit", "1"], capture_output=True)
        try:
            env = json.loads(proc.stdout.decode("utf-8"))
        except ValueError:
            return None
        return env["result"] if env.get("ok") else None

    before = len(errors)
    differ = sum(1 for d in decks if (r := run(d, d)) is None or r["listed"] or r["summary"]["slidesChanged"])
    if differ:
        fail(f"compare smoke: {differ} deck(s) differ from themselves or failed")
    by_folder: dict[Path, list[Path]] = {}
    for d in decks:
        by_folder.setdefault(d.parent, []).append(d)
    pairs = [(s[i], s[i + 1]) for s in by_folder.values() for i in range(len(s) - 1)]
    failed = asymmetric = 0
    mirror = {"slidesOnlyBase": "slidesOnlyOther", "slidesOnlyOther": "slidesOnlyBase", "slidesMoved": "slidesMoved"}
    for a, b in pairs:
        ab, ba = run(a, b), run(b, a)
        if ab is None or ba is None:
            failed += 1
        elif any(ab["summary"][k] != ba["summary"][v] for k, v in mirror.items()):
            asymmetric += 1
    if failed:
        fail(f"compare smoke: {failed} of {len(pairs)} deck pair(s) failed to compare")
    if asymmetric:
        fail(f"compare smoke: {asymmetric} of {len(pairs)} deck pair(s) do not mirror when swapped")
    if len(errors) == before:
        print(f"ok   compare smoke ({len(decks)} self-compares, {len(pairs)} pairs)")


_powerpoint_from_python: bool | None = None


def powerpoint_from_python() -> bool:
    """Whether Python can drive PowerPoint here: Windows, PowerPoint, and
    pywin32. Starts PowerPoint once to find out."""
    global _powerpoint_from_python
    if _powerpoint_from_python is None:
        probe = (
            "import win32com.client as w; a = w.DispatchEx('PowerPoint.Application')\n"
            "if a.Presentations.Count == 0: a.Quit()"
        )
        ran = sys.platform == "win32" and subprocess.run([sys.executable, "-I", "-c", probe], capture_output=True)
        _powerpoint_from_python = bool(ran) and ran.returncode == 0
    return _powerpoint_from_python


def format_parity() -> None:
    """Geometry and text formatting resolved through layouts, masters, and
    themes agree with PowerPoint's own on the PowerPoint-saved fixture and,
    when present, the private corpus (counts only). Needs Windows,
    PowerPoint, and pywin32; skipped with a note when they are absent."""
    if not powerpoint_from_python():
        print("skip format parity: needs Windows, PowerPoint, and pywin32")
        return
    proc = subprocess.run(["cargo", "build", "-q", "--locked", "-p", "pptdif", "--example", "formats"], cwd=ROOT)
    if proc.returncode != 0:
        fail("format parity: building the formats example failed")
        return
    exe = str(TARGET / "debug" / "examples" / ("formats.exe" if sys.platform == "win32" else "formats"))
    script = str(ROOT / "scripts" / "format_parity.py")
    runs = [("PowerPoint-saved fixture", ["--show"], [str(p) for p in sorted(SAVED.glob("*.pptx"))])]
    private = [str(p) for p in private_decks()]
    if private:
        runs.append((f"private corpus ({len(private)} decks)", [], private))
    for label, flags, decks in runs:
        proc = subprocess.run([sys.executable, "-I", script, *flags, exe, *decks])
        if proc.returncode != 0:
            fail(f"format parity: {label}")
        else:
            print(f"ok   format parity {label}")


def main() -> int:
    # The published skill is part of the repository; a missing one fails.
    if not (SKILLS / "pptdif" / "SKILL.md").is_file():
        fail("skills/pptdif/SKILL.md is missing")
    for skill in sorted(p for p in SKILLS.iterdir() if p.is_dir()) if SKILLS.is_dir() else []:
        check_skill(skill)
    run_cargo()
    cli_smoke()
    parity()
    compare_smoke()
    format_parity()
    if errors:
        print(f"\n{len(errors)} check(s) failed")
        return 1
    print("\nall checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
