# pptdif

`pptdif` compares two PowerPoint presentations (`.pptx`, `.pptm`) and
reports what changed: slides, text, shapes, and formatting. Slides are
matched by content, so an inserted or reordered slide is one change, not
every slide after it. Agents read its JSON; people will read its
side-by-side HTML report. It needs no PowerPoint.

**Status:** milestones M0 to M5 are done; no release is published yet.
`inspect`, `text`, and `compare` (JSON, text, and a self-contained HTML
report) cover slides, shapes, text, notes, geometry, formatting (inherited
through layouts, masters, and themes, and checked against PowerPoint),
tables, pictures, charts, comments, sections, layouts, masters, and themes.
The [`pptdif` agent skill](skills/pptdif/SKILL.md) teaches agents to drive
the CLI. See [PLAN.md](PLAN.md).

## Commands

- `pptdif inspect DECK` shows the slide size, each slide's id, visibility,
  layout, and section, and counts of shapes, groups, placeholders,
  pictures, tables, charts, notes, and comments, plus sections and the
  masters with their layouts. It prints no slide text.
- `pptdif text DECK [--slide N]` lists every shape's paragraphs (with
  outline level) and the speaker notes.
- `pptdif compare BASE OTHER [--slide N] [--kind KIND,...] [--limit N]
  [--all] [--tolerance PT] [--align content|position]` reports what
  changed.
  Slides pair by title and text, so an inserted or reordered slide is one
  change and a slide moved and edited is still one slide; shapes pair by
  placeholder, id, name, and content; paragraphs align, so a new bullet is
  one change. Each change has a stable id and both sides' slide, shape,
  paragraph, and text. Fields (slide numbers, dates) compare by kind, so
  renumbered footers are not changes. Beyond text it reports geometry
  (beyond a tolerance, half a point by default; a placeholder that writes
  no position of its own is not compared until inheritance is resolved), formatting as written
  (runs and paragraphs of unchanged text, a shape's outline, fill, and
  line), z-order and grouping, tables (columns and rows aligned, then
  cells), pictures by image content, charts' cached series, comments with
  their authors and replies, and the deck's sections. Formatting and
  geometry show the values in effect, inherited through the layout,
  master, theme, and default style; a slide reports them only where it
  writes something different, and a changed layout, master, or theme is
  one change under `objects`. A slide that moves to another layout reports
  what it now inherits as well. Table styles, a paragraph's own `a:defRPr`,
  and fills and lines a shape inherits are not read (see [TODO.md](TODO.md)).
  `--format html -o FILE` writes a self-contained report: each slide pair
  side by side with changed shapes outlined, the changes listed under it
  with changed words marked, and next and previous navigation. Slides are
  drawn schematically; `--images` has PowerPoint (Windows) render the
  changed slides instead, which takes about a minute. The report never
  replaces a compared deck, and replaces another file only with `--force`.

All accept `--format text`. Run `pptdif <command> --help` for details.

| Path | Purpose |
|---|---|
| [crates/pptdif](crates/pptdif/) | The library: reader, model, alignment, and comparison |
| [crates/pptdif-cli](crates/pptdif-cli/) | The `pptdif` executable |
| [crates/pptdif-fixtures](crates/pptdif-fixtures/) | Synthetic test decks |
| [fixtures/powerpoint](fixtures/powerpoint/) | A deck PowerPoint saved, with invented content |
| [skills/pptdif](skills/pptdif/SKILL.md) | The agent skill: routing, [comparing](skills/pptdif/references/comparing.md), [installation](skills/pptdif/references/installation.md) |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Boundaries, invariants, and the process contract |
| [PLAN.md](PLAN.md) | Design decisions and milestones |
| [TODO.md](TODO.md) | Open items and facts still to verify |
| `fixtures/private/` | Git-ignored real decks for local checks only |

## Installing

Download the Windows release archive (once one is published), or build from
source on any platform; see
[installation](skills/pptdif/references/installation.md).

## Building

```bash
cargo build --release
```

The executable is `target/release/pptdif` (`pptdif.exe` on Windows).

## Output

Every command prints one JSON object to stdout,
`{ok, command, result, warnings, error, metadata}`, and exits 0 on success,
1 on failure, and 2 on an invalid invocation. Help, `--version`, and
`--format text` views are plain text.

## Checks

```bash
python scripts/check.py
```

The reader parity check needs `python-pptx` (`pip install python-pptx`).
