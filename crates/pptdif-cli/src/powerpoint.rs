//! The one place pptdif starts another program: having PowerPoint export
//! slide images for the HTML report (`--images`). PowerPoint draws; it
//! never reads a deck for the comparison, which stays the same with or
//! without it.
//!
//! A hidden PowerPoint opens each deck read-only and without a window,
//! exports the slides of changed pairs as PNG into a scratch folder that is
//! removed before returning, and closes the deck. PowerPoint runs one
//! instance, which may hold the user's own presentations: only the deck it
//! opened is closed, and PowerPoint is quit only when nothing else is open.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use pptdif::diff::{Comparison, Status};
use pptdif::html::Images;
use pptdif::model::Presentation;

/// Slide images are this many pixels wide.
const WIDTH: i64 = 1280;

// Both decks in one PowerPoint session: starting PowerPoint takes most of
// the time (45 seconds cold, observed 2026-11-26), so it is paid once.
const SCRIPT: &str = r"
$ErrorActionPreference = 'Stop'
$app = New-Object -ComObject PowerPoint.Application
try {
  foreach ($k in 1, 2) {
    $path = [Environment]::GetEnvironmentVariable('PPTDIF_DECK' + $k)
    $slides = [Environment]::GetEnvironmentVariable('PPTDIF_SLIDES' + $k)
    if (-not $path -or -not $slides) { continue }
    $out = [Environment]::GetEnvironmentVariable('PPTDIF_OUT' + $k)
    $width = [int][Environment]::GetEnvironmentVariable('PPTDIF_WIDTH' + $k)
    $height = [int][Environment]::GetEnvironmentVariable('PPTDIF_HEIGHT' + $k)
    # ReadOnly, not Untitled, no window.
    $deck = $app.Presentations.Open($path, -1, 0, 0)
    try {
      foreach ($n in $slides.Split(',')) {
        if ($n) { $deck.Slides.Item([int]$n).Export((Join-Path $out ($n + '.png')), 'PNG', $width, $height) }
      }
    } finally { $deck.Close() }
  }
} finally {
  if ($app.Presentations.Count -eq 0) { $app.Quit() }
}
";

/// A scratch folder removed when dropped.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch() -> Result<Scratch, String> {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("pptdif-images-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot make a scratch folder: {e}"))?;
    Ok(Scratch(dir))
}

/// One deck's export: its path as COM takes it, the slides, and where.
struct Job {
    deck: String,
    slides: Vec<usize>,
    out: Scratch,
    width: i64,
    height: i64,
}

fn job(deck: &Path, presentation: &Presentation, slides: Vec<usize>) -> Result<Job, String> {
    let deck = std::fs::canonicalize(deck).map_err(|e| format!("cannot resolve {}: {e}", deck.display()))?;
    // `canonicalize` gives a `\\?\` path on Windows, which COM refuses.
    let deck = deck.display().to_string();
    let deck = deck.strip_prefix(r"\\?\").unwrap_or(&deck).to_owned();
    let size = presentation.slide_size.unwrap_or(pptdif::model::Size { cx: 12_192_000, cy: 6_858_000 });
    Ok(Job { deck, slides, out: scratch()?, width: WIDTH, height: (WIDTH * size.cy / size.cx.max(1)).max(1) })
}

/// Runs PowerPoint once over both jobs; the images as `data:` URIs.
fn export(jobs: [&Job; 2]) -> Result<[HashMap<usize, String>; 2], String> {
    if !cfg!(windows) {
        return Err("slide images need PowerPoint, which runs on Windows only".to_owned());
    }
    let mut command = Command::new("powershell");
    command.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", SCRIPT]);
    for (k, job) in (1..).zip(jobs) {
        let list: Vec<String> = job.slides.iter().map(ToString::to_string).collect();
        command
            .env(format!("PPTDIF_DECK{k}"), &job.deck)
            .env(format!("PPTDIF_SLIDES{k}"), list.join(","))
            .env(format!("PPTDIF_OUT{k}"), &job.out.0)
            .env(format!("PPTDIF_WIDTH{k}"), job.width.to_string())
            .env(format!("PPTDIF_HEIGHT{k}"), job.height.to_string());
    }
    let run = command.output().map_err(|e| format!("cannot start PowerShell: {e}"))?;
    if !run.status.success() {
        let why = String::from_utf8_lossy(&run.stderr);
        let first = why.lines().find(|l| !l.trim().is_empty()).unwrap_or("no reason given");
        return Err(format!("PowerPoint did not export the slides ({})", first.trim()));
    }
    let read = |job: &Job| -> Result<HashMap<usize, String>, String> {
        let mut out = HashMap::new();
        for &n in &job.slides {
            let bytes = std::fs::read(job.out.0.join(format!("{n}.png")))
                .map_err(|e| format!("slide {n} was not exported: {e}"))?;
            out.insert(n, format!("data:image/png;base64,{}", pptdif::html::base64(&bytes)));
        }
        Ok(out)
    };
    Ok([read(jobs[0])?, read(jobs[1])?])
}

/// Images of the slides on both sides of every pair that changed.
pub fn slide_images(
    base: &Path,
    other: &Path,
    comparison: &Comparison,
    a: &Presentation,
    b: &Presentation,
) -> Result<Images, String> {
    let mut wanted_a = Vec::new();
    let mut wanted_b = Vec::new();
    for pair in comparison.slides.iter().filter(|p| p.status != Status::Same) {
        wanted_a.extend(pair.base.as_ref().map(|r| r.index));
        wanted_b.extend(pair.other.as_ref().map(|r| r.index));
    }
    if wanted_a.is_empty() && wanted_b.is_empty() {
        return Ok(Images::default());
    }
    let (ja, jb) = (job(base, a, wanted_a)?, job(other, b, wanted_b)?);
    let [base, other] = export([&ja, &jb])?;
    Ok(Images { base, other })
}
