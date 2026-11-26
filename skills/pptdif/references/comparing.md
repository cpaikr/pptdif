# Comparing presentations

## Run the comparison

Start broad, then narrow:

1. `pptdif compare BASE OTHER --format text` for an overview: the header
   line counts changed, removed, added, and moved slides, then each change
   is one line with its id, kind, status, where it is in each deck, and the
   wording or properties on each side.
2. `pptdif compare BASE OTHER --slide N --kind text,notes` (JSON) to work
   with one slide's wording. `--limit 0` lists everything; the default lists
   500 changes and `omitted` counts the rest by slide pair and kind.
3. `pptdif text OTHER --slide N --format text` to read a slide's full text.
4. `pptdif compare BASE OTHER --format html -o REPORT.html` when a person
   will review the result. The report shows every change regardless of
   `--limit` and never replaces a compared deck; `--force` replaces another
   existing file. Add `--images` on Windows with PowerPoint installed to draw
   the changed slides as PowerPoint renders them; it takes about a minute,
   so use it when the person needs to see the slides, not by default.

`--tolerance PT` (default 0.5) ignores smaller nudges of position and size.
`--align position` pairs slide n with slide n when the user explicitly wants
that instead of matching by content.

## Read a change

Each slide pair has an `id` (`p3`), a `status` (`same`, `changed`,
`onlyBase`, `onlyOther`, `moved`; a moved slide may also carry edits), and
the slide's index, id, and title on each side. Each change has a stable
`id` (`p3:text:s3.2:s3.2` = pair 3, shape id 3 paragraph 2 on both sides), a
`kind`, a `status`, `base` and `other` sides naming the slide, shape (`id`,
`name`, `placeholder`), paragraph or table cell, and text, and often
`properties` (each with `base` and `other`). A changed paragraph may carry
`marks`: the changed parts of each side's text as UTF-16 ranges.

| kind | meaning |
|---|---|
| `slide` | a slide added, removed, or moved, or its visibility or layout changed |
| `shape` | a shape added or removed, or its kind, name, visibility, z-order, or group changed |
| `text` | a paragraph of a shape's text changed, added, removed, or moved |
| `notes` | a paragraph of the speaker notes |
| `geometry` | position, size (points), rotation, or flip changed beyond `--tolerance` |
| `format` | formatting of unchanged text (bold, size, font, colour, alignment, bullets, spacing), or a shape's outline, fill, or line |
| `table` | a table's row or column added, removed, or moved, a cell's text or merge state, or a cell's formatting |
| `picture` | a picture's image (compared by content) or crop |
| `chart` | a chart's kinds or title, or a series added, removed, reordered, or its categories or values (as the chart last drew them) |
| `comment` | a comment added, removed, or edited, or its author or replies |

Deck-level changes are under `objects`, not slides: `section` (renamed,
added, removed, or holding other slides), `layout` and `master`
(placeholders, text styles, decorations such as a logo, and the
`defaultTextStyle`), and `theme` (fonts, colours).

## Judge a change

- Formatting and geometry show the values in effect, but a slide reports
  them only where what the slide itself writes differs. A change to a
  layout, master, or theme is reported once under `objects`, not on every
  slide; tell the user it affects every slide using that layout or master.
- A slide moved to a layout of another name reports what it now inherits
  (positions, sizes) as well as the layout change.
- Slide numbers and dates in fields compare by kind, so renumbered footers
  are not changes.
- Chart values are what the chart last drew; the chart's own workbook is not
  compared. When the figures behind a chart matter, the user can compare
  that workbook with xldif.
- Pairing is by content: two slides that share a title and most text pair
  even when one was rewritten. When a pairing looks wrong, compare with
  `--align position` or read both slides with `pptdif text`.
- `not_compared` content (SmartArt, embedded objects, media, ink,
  animations) can change without a change being listed; say so when it is
  present and matters.
