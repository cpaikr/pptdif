//! Prints each slide shape's effective geometry and each text run's
//! effective formatting as JSON lines, for `scripts/format_parity.py` to
//! compare with PowerPoint's own.
//!
//! Usage: `cargo run -p pptdif --example formats -- DECK`

use pptdif::model::RunKind;
use serde_json::json;

fn main() {
    let path = std::env::args().nth(1).expect("usage: formats DECK");
    let deck = pptdif::read_path(std::path::Path::new(&path)).expect("the deck reads");
    for (s, slide) in deck.slides.iter().enumerate() {
        for shape in slide.all_shapes() {
            if let Some(g) = shape.effective_geometry {
                let pt = |emu: i64| emu as f64 / 12_700.0;
                let line = json!({"slide": s + 1, "shape": shape.id, "kind": "geometry",
                    "left": pt(g.x), "top": pt(g.y), "width": pt(g.cx), "height": pt(g.cy)});
                println!("{line}");
            }
            for (p, para) in shape.text.iter().flatten().enumerate() {
                let runs = para.runs.iter().filter(|r| r.kind != RunKind::Break);
                for (r, run) in runs.enumerate() {
                    let f = &run.effective;
                    let line = json!({"slide": s + 1, "shape": shape.id, "kind": "run", "paragraph": p + 1,
                        "run": r + 1, "text": run.text, "size": f.get("size"), "bold": f.get("bold"),
                        "italic": f.get("italic"), "font": f.get("font"), "align": para.effective.get("align")});
                    println!("{line}");
                }
            }
        }
    }
}
