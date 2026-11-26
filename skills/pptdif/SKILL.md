---
name: pptdif
description: "Compare two PowerPoint presentations (.pptx, .pptm) and explain what changed — slides added, removed, or moved, edited text and bullets, speaker notes, shapes moved or resized, formatting, tables, pictures, chart data, comments, sections, layouts, masters, themes — using the pptdif CLI, and hand people a side-by-side HTML report. Use when a task asks what changed between two versions of a deck (a proposal, a board pack, a client presentation, a report deck), to review a revised presentation, or to check a slide, figure, or wording across versions. Not for editing or creating decks, converting them, comparing PDFs or images, .ppt files without PowerPoint, or Google Slides."
---

# pptdif

`pptdif` reads two presentations without PowerPoint and reports how the
second differs from the first. Slides are matched by their titles and text,
so an inserted or reordered slide is one change, and a slide that moved and
was edited is still one slide. Shapes pair within each slide, paragraphs
align so a new bullet is one change, and formatting compares as it takes
effect through layouts, masters, and themes. All mechanics go through the
`pptdif` command-line tool.

## Treat decks as confidential

Decks are usually client or internal files. Process them locally; do not
upload them, their contents, or reports built from them. `pptdif inspect`
shows structure without slide text when only structure matters. Write HTML
reports where the user asked, or beside the decks.

## Start with the available executable

Run `pptdif --version` once per execution context. If it is missing or
fails, read [installation](references/installation.md). Use
`pptdif <command> --help` for exact options; do not assume a feature this
skill mentions is in the installed version.

## Interpret the output

Each command prints one JSON object: `ok`, `command`, `result`, `warnings`,
`error`, `metadata` — except with `--format text`, which prints a plain
listing to read as text (failures are still JSON), and `-o FILE`, whose
document goes to the file while stdout carries a summary envelope. Exit 0
means `ok: true`; 1 is a failure with `error.code`, `error.message`, and
often `error.hint`; 2 is an invalid invocation. Read `warnings` before
reporting: `not_compared` names content pptdif does not compare (SmartArt,
embedded objects, media, ink, animations, custom shows, macros), and
`alignment_fallback` says a pairing ran out of work and may be imperfect.

## Route the task

- What changed between two decks, a slide or figure across versions, or a
  report for a person: read [comparing](references/comparing.md).
- Reading a slide's text around a change: `pptdif text DECK --slide N
  --format text`.
- Only the shape of one deck (slides, layouts, sections, counts):
  `pptdif inspect DECK`.

## Universal rules

- Base is the earlier or reference deck, other the later one; say which is
  which when reporting, since every status (`onlyBase`, `onlyOther`) depends
  on it.
- Quote wording, figures, and slide numbers from the tool's output, never
  from memory. Slide numbers are positions in each deck: a slide can be 4 in
  one and 5 in the other.
- A summary count is exhaustive; a listing is not. When `omitted` is
  non-empty, narrow with `--slide` and `--kind` (or `--limit 0`) before
  claiming nothing else changed.
- pptdif never writes over a compared deck. Do not edit decks with other
  tools as part of a comparison.

## Finish

A comparison is complete when the user has the changes that matter for their
question, each with its slide (in both decks when it moved), shape, and base
and other wording or values, the warnings that affect what was compared,
and, when a person will review it, the path of an HTML report.
