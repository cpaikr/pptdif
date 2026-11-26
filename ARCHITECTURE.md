# Architecture

`pptdif` is one Rust implementation of PowerPoint presentation reading and
comparison, shipped as a standalone executable. [README.md](README.md) owns
commands and building; [PLAN.md](PLAN.md) owns the design decisions and
milestones. This file describes what exists now and the boundaries new code
must respect.

## Boundaries

```text
skills/pptdif ──(pptdif …; JSON on stdout)──► crates/pptdif-cli ──► crates/pptdif
  routing, judgment (planned)                   process contract     deck rules
```

- [`crates/pptdif`](crates/pptdif/src/lib.rs) owns every presentation rule
  and has no process I/O:
  - [package.rs](crates/pptdif/src/package.rs): the bounded ZIP archive,
    member lookup, content types, and relationships;
  - [xml.rs](crates/pptdif/src/xml.rs): a pull parser with local names,
    resolved references, and a depth cap; it also tells prefixed attributes
    from plain ones (`r:id` beside `id`);
  - [reader.rs](crates/pptdif/src/reader.rs): `presentation.xml`, masters,
    layouts, and each slide's shape tree, notes, and comments, into the
    model;
  - [model.rs](crates/pptdif/src/model.rs): the immutable presentation
    model;
  - [views.rs](crates/pptdif/src/views.rs): the `inspect` and `text`
    results;
  - [align.rs](crates/pptdif/src/align.rs): sequence alignment (anchors on
    unique content, similarity in the gaps, moves), for slides and
    paragraphs;
  - [diff.rs](crates/pptdif/src/diff.rs): the comparison: slide pairing
    (alignment, then order-free pairing of what is left), shape pairing,
    paragraph changes, ids, and the `compare` listing;
  - [error.rs](crates/pptdif/src/error.rs): stable error categories.

  `package.rs`, `xml.rs`, `align.rs`, and `error.rs` were copied from
  xldif at `8b272fd` and say so in their headers (PLAN.md, "Relation to
  xldif").
- [`crates/pptdif-cli`](crates/pptdif-cli/src/main.rs) parses arguments
  (clap), runs one command, and writes one stdout document
  ([envelope.rs](crates/pptdif-cli/src/envelope.rs)).
- [`crates/pptdif-fixtures`](crates/pptdif-fixtures/src/lib.rs) assembles
  synthetic decks from hand-written XML parts for tests. No shipped crate
  depends on it.

## Invariants

- **No writes to inputs.** pptdif only reads presentations.
- **Bounds.** File size, member count, each member's decompressed size
  (measured while reading), total decompressed size, XML depth, and the
  number of slides, shapes, and comments are capped
  ([package.rs](crates/pptdif/src/package.rs),
  [xml.rs](crates/pptdif/src/xml.rs),
  [reader.rs](crates/pptdif/src/reader.rs)). Hostile input fails with
  `invalid_archive` instead of exhausting memory or the stack.
- **Say what is not read.** Content pptdif does not compare (SmartArt, OLE
  objects, video and audio, ink, other graphic frames, animations, custom
  shows, VBA) is counted while reading and named in one `not_compared`
  warning per deck.
- **One shape per element.** A markup-compatibility block contributes its
  first choice; its fallback, which repeats the content for older readers,
  is skipped. A shape element inside a non-group shape (an OLE frame's
  preview picture) is part of that shape.
- **No slide text in `inspect`.** Text appears only in commands that ask
  for it.
- **Swapped files mirror.** Slides, and each slide pair, are compared in a
  fixed orientation (by a content hash) and the result turned round when
  that is not the order given, so `A B` reports what `B A` does with the
  sides exchanged. Which of two crossing slide pairs counts as moved is
  settled in that orientation too.
- **Stable ids.** A change id is built from its slide pair and both sides'
  shape ids and paragraph numbers, never from list order, and no two
  changes share one.
- **Fields are not edits.** A field compares by its kind, not the text
  PowerPoint saved for it.

## Process contract

Contract version 1 (`metadata.contract`), the same shape as `dsd` and
`xldif`:

- stdout carries exactly one newline-terminated document: the JSON envelope
  `{ok, command, result, warnings, error, metadata}`, or plain text for
  help, `--version`, and successful `--format text` views. Failures are
  always JSON. stderr stays empty.
- Exit 0 means `ok: true`; 1 is an execution failure with a stable
  `error.code` (`file_not_found`, `invalid_archive`, `unsupported_format`,
  `invalid_xml`, `invalid_argument`, `output_exists`, `io_error`, and
  `internal_error` for a panic, which is a bug); 2 is an invalid invocation
  (`invalid_invocation`).
- Bare `pptdif` prints exactly `pptdif --help`.
- `metadata` is `{tool: "pptdif", version, contract}`.

## Verification

- [`crates/pptdif-cli/tests/cli.rs`](crates/pptdif-cli/tests/cli.rs) runs
  the executable as an external process and checks the process contract.
- [`crates/pptdif/tests/reader.rs`](crates/pptdif/tests/reader.rs) reads
  synthetic decks, including shapes writer libraries never produce
  (markup-compatibility blocks, OLE previews, nested groups).
- [`crates/pptdif/tests/powerpoint_saved.rs`](crates/pptdif/tests/powerpoint_saved.rs)
  reads `fixtures/powerpoint/`: one invented deck that PowerPoint built and
  saved ([`scripts/powerpoint_fixture.py`](scripts/powerpoint_fixture.py)).
  Its assertions are the file-format facts observed in PowerPoint's own
  output.
- [`crates/pptdif/tests/compare.rs`](crates/pptdif/tests/compare.rs)
  covers inserted, deleted, moved, and moved-and-edited slides, bullets
  edited and added, renumbered fields, shape pairing, notes, visibility,
  the listing, and two properties over random decks: a deck against itself
  shows nothing, and swapping the files mirrors the result.
- [`scripts/parity.py`](scripts/parity.py) compares each slide's id,
  layout, visibility, notes, and shape counts with python-pptx, an
  independent reader.
- [`scripts/check.py`](scripts/check.py) runs fmt, clippy, tests, a CLI
  smoke run, and parity over the public fixtures and, when present, the
  private corpus, plus a compare smoke run there: self-compares and
  swapped pairs (counts only).
