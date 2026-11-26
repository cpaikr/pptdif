# pptdif build plan

`pptdif` compares two PowerPoint presentations and reports what changed:
slides, text, tables, pictures, charts, notes, comments, layout, and
formatting. Slides are matched by content, so an inserted or reordered
slide is one change, not every slide after it. It ships as a standalone
CLI with two outputs: JSON for agents, and a self-contained side-by-side
HTML report for people. An agent skill teaches agents to drive it.

It is a sibling of `../xldif` (Excel workbooks): the same process
contract, report style, and repository conventions, and xldif's
container reader and alignment engine as its starting point. What is new
is the presentation model, shape pairing, text-style inheritance, and a
report that shows slides rather than grids.

Status:
- The plan was drafted on 2026-11-26 from a reading of xldif at `8b272fd`.
- M0 is done: the survey (below), the workspace with the copied xldif
  modules, the fixture builder, the PowerPoint-saved fixture and its
  script, python-pptx parity, `scripts/check.py`, and CI.
- M1 has started: the reader takes slide order, sections, masters and
  layouts, and each slide's shape tree (kinds, ids, names, placeholders,
  groups), notes, and comments; `inspect` reports them. Parity with
  python-pptx is clean on the public fixtures.
- Next action: text bodies, then `text` and `compare` ([TODO.md](TODO.md)).

## Decisions

| Decision | Choice | Why |
|---|---|---|
| Build or reuse | Build. Study sugatoray/pptxdiff (Apache-2.0) as prior art for slide similarity and coverage | The survey (2026-11-26) found no tool that pairs slides by content and gives agents a change list without Office or a browser. sugatoray/pptxdiff comes closest: content-aligned slides, word marks, formatting and geometry, but it is an early single-author browser app whose headless mode drives the page through Playwright and whose JSON is shaped for its UI. Aspose.Slides answers only equal or not; GroupDocs, Draftable, and Litera produce redlines; PowerPoint's own Compare is interactive and being retired; the git-oriented tools compare text or rendered images only |
| Relation to xldif | Copy `package`, `xml`, `error`, `align`, and `textdiff` from xldif `8b272fd`, each with a header naming its origin. No path or git dependency on xldif | A `../xldif` path breaks CI and ties releases together. `align.rs` (`Item { key, tokens, shape }`) has nothing Excel-specific in it. Extract a shared crate once a fix has had to land in both copies |
| Language and layout | Rust workspace: `crates/pptdif` (library, no process I/O), `crates/pptdif-cli` (clap, process contract), `crates/pptdif-fixtures` (test decks) | Same boundaries, lints (clippy pedantic, `unsafe_code = "forbid"`), and release profile as xldif |
| Presentation reader | Own read-only OOXML reader on `zip` + `quick-xml` | A diff needs exact control over placeholder and style inheritance and over hostile-input bounds; no Rust library offers both |
| File types | `.pptx` and `.pptm` (macros not diffed), read as saved. `.ppt` and password-protected decks through a copy PowerPoint saves as `.pptx`, where PowerPoint is installed; elsewhere `unsupported_format` | xldif's rule: the converter's format knowledge is not duplicated, and the copy goes through the same reader |
| PowerPoint's role | A converter for inputs the reader rejects, the renderer of slide images for the HTML report, and a test oracle. Never the reader | JSON results must not depend on PowerPoint being installed or on how it renders. The report degrades to a schematic without it |
| Scope | Two-file comparison. A `series` command (which saved versions to keep), as in xldif, waits until compare is proven | Merging and three-way comparison are out of v1 |
| Text first | v1 compares text, structure, and formatting written on the text itself. Effective (inherited) formatting comes in M3 | Most deck edits are wording. The inheritance chain is the largest correctness risk and should not block a useful tool |
| Computed content | Field text (slide numbers, dates), autofit's `fontScale` and `lnSpcReduction`, and other values PowerPoint recomputes are not reported as edits; they are counted apart, as xldif counts recalculated values | Inserting one slide renumbers every later footer; that is noise, not an edit |
| JSON style | camelCase fields and the shared status words (`same`, `changed`, `onlyBase`, `onlyOther`, plus `moved`) | One vocabulary across the user's tools |
| Fixtures | Synthetic decks assembled from hand-written XML parts, plus PowerPoint-saved decks with invented content made by a COM script | Hand-written parts give exact control over what the reader sees, including shapes no writer produces. `ppt-rs` (Apache-2.0) is the closest Rust writer; revisit it if fixture XML grows unwieldy. Inheritance, modern comments, and autofit values only come from PowerPoint. The COM script needs a document window: charts and SmartArt fail without one |
| Confidentiality | Same rule as xldif: real decks only under git-ignored `fixtures/private/`. Their contents, file names, and derived values never appear in tests, docs, or commits | Real inputs are client and audit material |

## Architecture

```text
skills/pptdif ──(pptdif …; JSON on stdout)──► crates/pptdif-cli ──► crates/pptdif
  routing, judgment                             process contract     deck rules
```

`crates/pptdif` modules, planned:

- **`package`, `xml`, `error`** (from xldif): the bounded ZIP archive,
  relationships, content types, and a pull parser with a depth cap.
- **`reader`:** `presentation.xml` (slide order from `sldIdLst`, slide size,
  sections, default text style), each slide and its notes and comments,
  layouts, masters, and the theme, into the model. Content with no place in
  the model is counted for a `not_compared` warning.
- **`model`:** an immutable `Presentation` → `Slide` → shape tree.
- **`inherit`** (M3): placeholder and text-style inheritance, resolved into
  comparable properties.
- **`align`** (from xldif): sequence alignment for slides, paragraphs, and
  table rows and columns.
- **`pair`:** shape pairing within a slide pair (a set, not a sequence).
- **`textdiff`** (from xldif): word-level change marks.
- **`diff`, `diff_objects`:** the comparison's result types, slide and
  shape changes, layouts, masters, theme, and the `compare` listing.
- **`html`:** the self-contained report.
- **`views`:** `inspect` and `text` results.

The CLI owns argument parsing, the envelope, text listings, exit codes,
refusing to overwrite files, and the one place pptdif starts another
program (`powerpoint.rs` with its PowerPoint script): converting a rejected
input and exporting slide images.

## Model

- **Presentation:** slide size (EMU), slide order, sections (name and the
  slides in each), the default text style, the list of masters and layouts,
  and a theme fingerprint.
- **Slide:** its `sldId` and part name, layout (by layout name), hidden flag
  (`show="0"`), section, notes text, comments, transition kind (presence and
  type only), and the shape tree.
- **Shape:** kind (`sp`, `grpSp`, `pic`, `graphicFrame` holding a table,
  chart, diagram, or OLE object, `cxnSp`), `cNvPr` id, name, alt text,
  hidden flag, placeholder type and index, geometry (offset, extent,
  rotation, flips; inherited from the layout placeholder when the slide
  writes none), preset geometry, and the content below.
- **Text body:** paragraphs (level, bullet, alignment, spacing) of runs
  (text, line breaks, tabs, fields by type, and the run's own properties).
- **Table:** a grid of cells, each with a text body, spans, and the table
  style id.
- **Picture:** the image part's content hash, crop, and the picture's
  geometry. Same bytes under another part name is the same picture.
- **Chart:** chart type, series names, and the cached categories and
  values (`c:strCache`, `c:numCache`). The embedded workbook is not read;
  the skill may hand it to xldif.
- **Not compared (v1), named in the warning:** SmartArt (`dgm`), OLE
  objects, media, ink, other graphic frames, animations (`p:timing`),
  custom shows, and VBA. Extension lists are not named: PowerPoint writes
  them in every deck, and the ones that matter (sections) are read.

## Alignment

1. **Slides.** Each slide becomes an `align::Item`:
   - key: the normalized text of its title or centered-title placeholder
     (whitespace, case, and a leading outline marker dropped, as xldif
     normalizes row labels);
   - tokens: hashes of each normalized paragraph, table cell text, picture
     hash, and chart fingerprint;
   - shape: a hash of the layout name.

   Decks are reordered far more often than sheets are, and reordered slides
   are often edited too. xldif's move pass needs equal content, so after
   alignment the unpaired slides on both sides are paired by similarity
   regardless of order (at least `MIN_SIMILARITY`, best pair first, ties by
   index) and reported as moved and changed. Duplicated template slides
   share a key; `align` already keeps a shared label from pairing ahead of
   content.
2. **Shapes within a slide pair.** Shapes are a set, so pairing is in
   steps, each over what is still unpaired:
   1. placeholders with the same type and index;
   2. the same `cNvPr` id and the same name (stable within one file's save
      history, not across decks assembled by copy-and-paste);
   3. content similarity (text tokens, picture hash, table content) with
      geometry as a tie-break, best pair first, ties by index.

   The rest are `onlyBase` or `onlyOther`. Group shapes pair first, then
   their children. A changed z-order is a property of the shape.
3. **Inside a shape.** Paragraphs align as a sequence (tokens: words), so a
   new bullet is one change. Table columns then rows align as xldif aligns
   a sheet.
4. **Determinism and symmetry.** As in xldif: fixed hashes (FNV-1a), ties by
   index, each pair aligned in a fixed order and turned round when needed,
   so swapping the files mirrors the result. Property tests hold this and
   "a deck against itself shows nothing".
5. **`--align position`** pairs slides by index and shapes by id.

## Comparison

- **Kinds:** `slide`, `shape`, `text`, `table`, `picture`, `chart`,
  `notes`, `comment`, `geometry`, `format`, `layout`, `master`, `theme`,
  `section`, `transition`. One vocabulary for `--kind` and the output.
- **Text:** compared exactly after paragraph alignment; a changed paragraph
  carries both texts and word marks. Fields compare by type, not by their
  cached text.
- **Geometry:** offset, extent, rotation, and flips in EMU, with a tolerance
  (default to be tuned in M2; 1 pt is 12,700 EMU) so a nudge is not noise.
  Reported in points.
- **Formatting:** v1 compares run and paragraph properties as written
  (font, size, bold, italic, underline, strike, colour kept symbolic,
  highlight; alignment, level, bullet, spacing) and shape fill and line.
  M3 resolves the effective value through the chain: run, paragraph, the
  shape's list styles, the layout placeholder, the master placeholder, the
  master's text styles, the presentation's default text style, and theme
  fonts and colours.
- **Layouts, masters, theme:** each slide's layout is a property of the
  slide. A changed layout or master is one change, not one per slide using
  it; any theme change is one `theme` change.
- **Ids:** built from location (slide pair, kind, base and other shape),
  never from list order; slide titles never go inside an id.
- **Limits:** `--limit` caps listed changes; `omitted` counts what was cut
  per slide and kind.

### JSON result shape (sketch)

```jsonc
{
  "base": {"file": "a.pptx", "slides": 24},
  "other": {"file": "b.pptx", "slides": 25},
  "summary": {"slidesChanged": 4, "onlyOther": 1, "moved": 2, "text": 11,
              "geometry": 3, "picture": 1, "computed": 25, "listed": 19},
  "slides": [{
    "id": "p7",
    "status": "changed",
    "base": {"index": 7, "title": "Revenue bridge"},
    "other": {"index": 9, "title": "Revenue bridge"},
    "moved": true,
    "changes": [
      {"id": "p7:text:sp4:sp4:2", "kind": "text", "status": "changed",
       "shape": {"name": "Content Placeholder 2", "placeholder": "body"},
       "base": {"paragraph": 2, "text": "Growth of 12% driven by pricing"},
       "other": {"paragraph": 2, "text": "Growth of 14% driven by volume"}},
      {"id": "p7:geometry:pic5:pic6", "kind": "geometry", "status": "changed",
       "properties": [{"name": "x", "base": 72, "other": 96, "unit": "pt"}]}
    ]
  }],
  "objects": [{"id": "layout:Title and Content", "kind": "layout",
               "status": "changed"}]
}
```

## Commands

- `pptdif inspect DECK`: slide count, hidden slides, sections, layout and
  master names, and per slide the counts of shapes, pictures, tables,
  charts, notes, and comments, plus reader warnings. It prints no slide
  text.
- `pptdif text DECK [--slide N]`: each shape's text with its slide, name,
  and placeholder, as xldif's `cells` lists cells.
- `pptdif compare BASE OTHER [--format json|text|html] [-o FILE]
  [--slide N] [--kind KIND,...] [--limit N] [--all] [--tolerance PT]
  [--align content|position] [--force]`. HTML requires `-o`; it never
  overwrites an input.

## Process contract

xldif's contract version 1: one newline-terminated stdout document (the
envelope `{ok, command, result, warnings, error, metadata}`, or plain text
for help, `--version`, and `--format text`), stderr empty, exit 0/1/2, and
the same error codes, with `powerpoint_failed` in place of `excel_failed`.
`metadata` is `{tool: "pptdif", version, contract}`.

## HTML report

- **Layout:** xldif's: a top bar with summary chips and warnings,
  previous and next change (`N`/`P`/`J`/`K`), a "changes only" toggle, a
  slide list with a change count per slide, and two panes side by side
  that scroll together.
- **Slides:** each pane shows its slides in aligned order; an inserted or
  deleted slide has a hatched placeholder opposite it, and moved slides
  are marked.
- **With PowerPoint:** changed slides are exported as images and changed
  shapes are outlined over them, placed from their geometry scaled by the
  slide size. Outlines need the resolved geometry: inherited placeholder
  positions and group transforms (`chOff`, `chExt`) applied.
- **Without PowerPoint:** a schematic slide, shape outlines at their
  positions with their text, with a warning naming the fallback.
- **Details:** clicking a shape shows each side's text with word marks and
  its changed properties.
- **Size:** only changed slides carry images; unchanged ones are listed.
  Image size and export time are measured in M4 before limits are set.

## Milestones

Each milestone ends with `python scripts/check.py` green: fmt, clippy
pedantic, tests, and a CLI smoke run.

- **M0 Scaffold and survey.**
  - Survey existing tools (GitHub, commercial libraries, PowerPoint's own
    Compare) and record the result under "Build or reuse".
  - Workspace, the copied xldif modules with their origin, `AGENTS.md`,
    `ARCHITECTURE.md`, `README.md`, `TODO.md`, `.gitignore` covering
    `fixtures/private/`, and `scripts/check.py`.
  - The fixture builder (XML parts into a minimal deck) and a PowerPoint
    COM script that saves invented decks for `fixtures/powerpoint/`.
- **M1 Reader, slide alignment, and text.**
  - `reader`, `model`, `inspect`, `text`, and `compare` over slides and
    text, JSON and `--format text`.
  - Done when: text parity with python-pptx on every fixture and the
    private corpus; fixtures with inserted, deleted, moved, moved-and-edited,
    and duplicated slides give exactly the expected alignment; a deck
    against itself shows nothing; swapping the files mirrors.
- **M2 Shapes and objects.**
  - Shape pairing, geometry with tolerance, tables, pictures, charts'
    cached data, notes, comments, sections, hidden slides, direct
    formatting, computed content counted apart.
  - Done when: a variant fixture for each kind gives exactly the expected
    changes, and a private-corpus smoke run passes printing counts only.
- **M3 Inheritance.**
  - Effective formatting and geometry through layouts, masters, and the
    theme; layout, master, and theme changes.
  - Done when: a PowerPoint COM oracle script (`TextRange.Font`,
    `Shape.Left/Top/Width/Height`) agrees with the resolved values on the
    PowerPoint-saved fixtures and the private corpus.
- **M4 HTML report.**
  - Schematic rendering first, then PowerPoint slide images with outlines.
  - Done when: reports for the fixtures are checked in a browser
    (navigation, synced scrolling, details, mobile width), and a structure
    test asserts no external URLs and the size limits.
- **M5 Skill and release.**
  - The `pptdif` agent skill with evals, and a Windows x64 release
    workflow, as in xldif.

## Risks to watch

- **Reordered and edited slides.** The second pairing pass must not pair
  unrelated slides that merely share a layout and boilerplate (footers,
  logos, disclaimers). Boilerplate that repeats on most slides may need to
  be dropped from tokens.
- **Shape identity across decks.** Slides copied between decks get new
  `cNvPr` ids; pairing then rests on placeholders and content.
- **Inheritance correctness.** Seven levels, each optional; prove each on
  PowerPoint-saved files before relying on it.
- **Overlay accuracy.** Rotated shapes, nested groups, and inherited
  positions all move outlines; check them against exported images.
- **PowerPoint automation.** Protected View, modal dialogs, a deck already
  open, and export speed. Each failure is `powerpoint_failed` with a hint,
  never a hang.
- **Report size.** Slide images embedded as data URIs can make a large
  deck's report heavy.

## Unverified facts

A file-format fact counts as verified only after it is observed in a
PowerPoint-saved file:

- a placeholder with no `xfrm` takes its geometry from the layout, then the
  master;
- where modern (threaded) comments are stored and how legacy comments sit
  beside them;
- `show="0"` as the only hidden-slide mark;
- which values autofit writes (`fontScale`, `lnSpcReduction`) and when
  PowerPoint rewrites them;
- the field types PowerPoint writes for slide numbers and dates;
- that slide export through COM renders hidden slides and honours the
  slide size.
