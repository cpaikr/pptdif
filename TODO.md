# TODO

Open work. [PLAN.md](PLAN.md) holds the decisions and milestones; this file
holds what is left. Remove an item when it lands.

## Next

- **Inheritance not read:** table styles, a paragraph's own `a:defRPr`,
  and shape fills and lines inherited from the layout.
- **Alignment on real decks.** Review slide pairing on real version pairs
  once `fixtures/private/` has some; tune boilerplate (footers, logos)
  that repeats on most slides out of the slide tokens if it misleads.

## Verification

Facts the code relies on that need a PowerPoint-saved deck before they
count as verified:

- markup-compatibility blocks in shape trees, read from the first choice
  (PowerPoint wrote none in the current fixture);
- legacy comment lists (`p:cmLst`, one `p:cm` each, a reply marked by
  `p15:threadingInfo/p15:parentCm`); only modern comments were observed;
- video and audio pictures (`videoFile`, `audioFile`, `p14:media`);
- `.pptm`, template, and show content types, and the `vbaProject`
  relationship;
- `custShowLst` for custom shows;
- a placeholder with no `xfrm` taking its geometry from the layout, then
  the master;
- which autofit values PowerPoint writes (`fontScale`, `lnSpcReduction`)
  and when it rewrites them, and the field types for slide numbers and
  dates.

Observed in `fixtures/powerpoint/powerpoint-saved.pptx` (2026-11-26) and
asserted in `crates/pptdif/tests/powerpoint_saved.rs`: `show="0"` on hidden
slides, `<p:timing>` only on animated slides, sections in the `p14`
extension listing `sldId` ids, modern comments, and the `graphicData` URIs
for tables, charts, and SmartArt.
