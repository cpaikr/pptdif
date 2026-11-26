//! The CLI as an external process: the process contract.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn pptdif<const N: usize>(args: [&str; N]) -> Run {
    let out = Command::new(env!("CARGO_BIN_EXE_pptdif")).args(args).output().unwrap();
    Run {
        code: out.status.code().unwrap(),
        stdout: String::from_utf8(out.stdout).unwrap(),
        stderr: String::from_utf8(out.stderr).unwrap(),
    }
}

/// A per-test scratch directory under the system temp dir, removed on drop.
struct Work(PathBuf);

impl Work {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pptdif-cli-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn basic(&self) -> PathBuf {
        self.file("basic.pptx", &pptdif_fixtures::basic().to_bytes())
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn envelope(run: &Run) -> Value {
    assert!(run.stdout.ends_with('\n') && run.stdout.matches('\n').count() == 1, "one line: {:?}", run.stdout);
    serde_json::from_str(&run.stdout).unwrap()
}

#[test]
fn bare_invocation_is_help() {
    let bare = pptdif([]);
    let help = pptdif(["--help"]);
    assert_eq!((bare.code, bare.stderr.as_str()), (0, ""));
    assert_eq!(bare.stdout, help.stdout);
    assert!(bare.stdout.starts_with("Compare PowerPoint presentations"));
}

#[test]
fn version_is_plain_text() {
    let run = pptdif(["--version"]);
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout, format!("pptdif {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn an_invalid_invocation_is_a_json_failure_with_status_2() {
    let run = pptdif(["no-such-command"]);
    assert_eq!((run.code, run.stderr.as_str()), (2, ""));
    let env = envelope(&run);
    assert_eq!(env["ok"], false);
    assert_eq!(env["error"]["code"], "invalid_invocation");
    assert_eq!(env["metadata"]["tool"], "pptdif");
    assert_eq!(env["metadata"]["contract"], "1");
}

#[test]
fn inspect_reports_structure_in_the_envelope() {
    let work = Work::new();
    let deck = work.basic();
    let run = pptdif(["inspect", s(&deck)]);
    assert_eq!((run.code, run.stderr.as_str()), (0, ""));
    let env = envelope(&run);
    assert_eq!((env["ok"].clone(), env["command"].clone()), (Value::Bool(true), Value::from("inspect")));
    let result = &env["result"];
    assert_eq!(result["file"], "basic.pptx");
    assert_eq!(result["format"], "pptx");
    assert_eq!(result["slideSize"]["widthPt"], 960.0);
    let second = &result["slides"][1];
    assert_eq!(second["index"], 2);
    assert_eq!(second["layout"], "Title and Content");
    assert_eq!((second["shapes"].clone(), second["placeholders"].clone()), (Value::from(3), Value::from(2)));
    assert_eq!(second["pictures"], 1);
    assert_eq!(env["warnings"], Value::Array(Vec::new()));
}

#[test]
fn inspect_never_prints_slide_text() {
    let work = Work::new();
    let deck = work.basic();
    for format in ["json", "text"] {
        let run = pptdif(["inspect", s(&deck), "--format", format]);
        assert_eq!(run.code, 0);
        assert!(!run.stdout.contains("Quarterly review") && !run.stdout.contains("Revenue grew"), "{}", run.stdout);
    }
}

#[test]
fn inspect_text_is_one_line_per_record() {
    let work = Work::new();
    let deck = work.basic();
    let run = pptdif(["inspect", s(&deck), "--format", "text"]);
    assert_eq!(run.code, 0);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines[0], "basic.pptx (Pptx, 960 x 540 pt, 2 slides)");
    assert_eq!(lines[2], "2\tid 257\tshown\tTitle and Content\tshapes 3 pictures 1");
}

#[test]
fn compare_reports_changes_in_the_envelope() {
    let work = Work::new();
    let base = work.basic();
    let edited = pptdif_fixtures::basic().slide(pptdif_fixtures::content_slide("Next steps", &["Order railings"]));
    let other = work.file("edited.pptx", &edited.to_bytes());
    let run = pptdif(["compare", s(&base), s(&other)]);
    assert_eq!((run.code, run.stderr.as_str()), (0, ""));
    let env = envelope(&run);
    let result = &env["result"];
    assert_eq!(result["summary"]["slidesOnlyOther"], 1);
    assert_eq!(result["listed"], 1);
    let change = &result["slides"][0]["changes"][0];
    assert_eq!((change["kind"].clone(), change["status"].clone()), (Value::from("slide"), Value::from("onlyOther")));
    assert_eq!(change["other"]["text"], "Next steps");

    let text = pptdif(["compare", s(&base), s(&other), "--format", "text"]);
    assert_eq!(text.code, 0);
    assert!(
        text.stdout.lines().nth(1).unwrap().starts_with("p3:slide:-:slide\tslide\tadded\tslide 3"),
        "{}",
        text.stdout
    );

    let same = pptdif(["compare", s(&base), s(&base)]);
    assert_eq!(envelope(&same)["result"]["listed"], 0);

    // `--all` lists unchanged slide pairs in the text listing too.
    let all = pptdif(["compare", s(&base), s(&base), "--all", "--format", "text"]);
    let lines: Vec<&str> = all.stdout.lines().collect();
    assert_eq!(&lines[1..3], ["p1\tslide\tsame\tslide 1 -> slide 1", "p2\tslide\tsame\tslide 2 -> slide 2"]);
}

#[test]
fn text_lists_paragraphs_by_shape() {
    let work = Work::new();
    let deck = work.basic();
    let run = pptdif(["text", s(&deck), "--slide", "2"]);
    assert_eq!(run.code, 0);
    let env = envelope(&run);
    let slides = env["result"]["slides"].as_array().unwrap();
    assert_eq!(slides.len(), 1);
    assert_eq!(slides[0]["shapes"][1]["paragraphs"][0]["text"], "Revenue grew");
    assert_eq!(slides[0]["shapes"][1]["placeholder"], "body:1");

    let bad = pptdif(["text", s(&deck), "--slide", "9"]);
    assert_eq!((bad.code, envelope(&bad)["error"]["code"].clone()), (1, Value::from("invalid_argument")));
}

#[test]
fn failures_carry_a_stable_code() {
    let work = Work::new();
    let missing = work.0.join("missing.pptx");
    let run = pptdif(["inspect", s(&missing)]);
    assert_eq!((run.code, run.stderr.as_str()), (1, ""));
    assert_eq!(envelope(&run)["error"]["code"], "file_not_found");

    let junk = work.file("junk.pptx", b"plain text");
    let run = pptdif(["inspect", s(&junk), "--format", "text"]);
    assert_eq!(run.code, 1);
    // Failures stay JSON even when text was asked for.
    assert_eq!(envelope(&run)["error"]["code"], "invalid_archive");
}
