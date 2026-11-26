# pptdif

`pptdif` compares two PowerPoint presentations (`.pptx`, `.pptm`) and
reports what changed: slides, text, shapes, and formatting. Slides are
matched by content, so an inserted or reordered slide is one change, not
every slide after it. Agents read its JSON; people will read its
side-by-side HTML report. It needs no PowerPoint.

**Status:** M0 and M1 are done: `inspect`, `text`, and `compare` over
slides, shapes, text, and notes. Geometry, tables, pictures, charts, and
formatting (M2), inherited formatting (M3), and the HTML report (M4) are
next. See [PLAN.md](PLAN.md).

## Commands

- `pptdif inspect DECK` shows the slide size, each slide's id, visibility,
  layout, and section, and counts of shapes, groups, placeholders,
  pictures, tables, charts, notes, and comments, plus sections and the
  masters with their layouts. It prints no slide text.
- `pptdif text DECK [--slide N]` lists every shape's paragraphs (with
  outline level) and the speaker notes.
- `pptdif compare BASE OTHER [--slide N] [--kind slide,shape,text,notes]
  [--limit N] [--all] [--align content|position]` reports what changed.
  Slides pair by title and text, so an inserted or reordered slide is one
  change and a slide moved and edited is still one slide; shapes pair by
  placeholder, id, name, and content; paragraphs align, so a new bullet is
  one change. Each change has a stable id and both sides' slide, shape,
  paragraph, and text. Fields (slide numbers, dates) compare by kind, so
  renumbered footers are not changes.

All accept `--format text`. Run `pptdif <command> --help` for details.

| Path | Purpose |
|---|---|
| [crates/pptdif](crates/pptdif/) | The library: reader, model, alignment, and comparison |
| [crates/pptdif-cli](crates/pptdif-cli/) | The `pptdif` executable |
| [crates/pptdif-fixtures](crates/pptdif-fixtures/) | Synthetic test decks |
| [fixtures/powerpoint](fixtures/powerpoint/) | A deck PowerPoint saved, with invented content |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Boundaries, invariants, and the process contract |
| [PLAN.md](PLAN.md) | Design decisions and milestones |
| [TODO.md](TODO.md) | Open items and facts still to verify |
| `fixtures/private/` | Git-ignored real decks for local checks only |

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
