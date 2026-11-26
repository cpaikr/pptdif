//! `pptdif`: the process boundary. Parses arguments, runs one command, and
//! writes exactly one stdout document: a JSON envelope, or plain text for
//! help, `--version`, and `--format text` views. stderr stays empty, even
//! when a command panics.

mod cli;
mod commands;
mod envelope;
mod powerpoint;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::ExitCode;
use std::sync::Mutex;

use clap::Parser as _;
use clap::error::ErrorKind;

use crate::cli::Cli;
use crate::envelope::{Envelope, Failure};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Writes the one stdout document. A closed pipe is fine (the reader
/// stopped listening); any other failure means the document was lost, which
/// only the exit status can still say.
fn write_stdout(text: &str) -> Result<(), ()> {
    write_document(&mut std::io::stdout().lock(), text)
}

fn write_document(out: &mut impl std::io::Write, text: &str) -> Result<(), ()> {
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => Err(()),
        _ => Ok(()),
    }
}

/// The exit status, or 1 when stdout could not take the document.
fn finish(text: &str, code: ExitCode) -> ExitCode {
    if write_stdout(text).is_ok() { code } else { ExitCode::from(1) }
}

fn command_name(argv: &[String]) -> String {
    argv.iter().skip(1).find(|a| !a.starts_with('-')).cloned().unwrap_or_default()
}

/// Runs a command with a panic turned into an `internal_error` failure, so
/// a bug still ends in one JSON document, exit status 1, and an empty
/// stderr.
fn guarded(run: impl FnOnce() -> Result<commands::Output, Failure>) -> Result<commands::Output, Failure> {
    static PANIC: Mutex<Option<String>> = Mutex::new(None);
    // The hook replaces the default one, which writes to stderr, while the
    // command runs; it keeps the message and source location for the
    // envelope instead.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|info| {
        if let Ok(mut slot) = PANIC.lock() {
            *slot = Some(info.to_string());
        }
    }));
    // Nothing the command touched is used again after a panic.
    let outcome = catch_unwind(AssertUnwindSafe(run));
    std::panic::set_hook(default_hook);
    outcome.unwrap_or_else(|_| {
        let detail = PANIC.lock().ok().and_then(|mut slot| slot.take()).unwrap_or_default();
        Err(Failure::internal(&detail.split_whitespace().collect::<Vec<_>>().join(" ")))
    })
}

fn main() -> ExitCode {
    // Paths stay OS strings: a deck's name need not be valid Unicode. The
    // lossy form only names the command in messages.
    let argv: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let lossy: Vec<String> = argv.iter().map(|a| a.to_string_lossy().into_owned()).collect();
    let pretty = lossy.iter().any(|a| a == "--pretty");
    // Bare `pptdif` is exactly `pptdif --help`: the same parser renders both.
    let parse_args = if argv.len() == 1 { vec![argv[0].clone(), "--help".into()] } else { argv.clone() };
    let cli = match Cli::try_parse_from(&parse_args) {
        Ok(cli) => cli,
        Err(e) if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) => {
            return finish(&e.to_string(), ExitCode::SUCCESS);
        }
        Err(e) => {
            let rendered = e.to_string();
            let message =
                rendered.lines().next().unwrap_or("invalid arguments").trim_start_matches("error: ").to_owned();
            let command = command_name(&lossy);
            let hint = if command.is_empty() || e.kind() == ErrorKind::InvalidSubcommand {
                "run `pptdif --help` for commands".to_owned()
            } else {
                format!("run `pptdif {command} --help` for options and examples")
            };
            let failure = Failure::usage(message, hint);
            return finish(&Envelope::failure(&command, &failure).render(pretty), ExitCode::from(failure.exit_code()));
        }
    };
    let name = cli.command.name();
    match guarded(|| commands::run(cli.command)) {
        Ok(commands::Output::Text(text)) => finish(&text, ExitCode::SUCCESS),
        Ok(commands::Output::Json { result, warnings }) => {
            finish(&Envelope::success(name, result, &warnings).render(cli.pretty), ExitCode::SUCCESS)
        }
        Err(failure) => {
            finish(&Envelope::failure(name, &failure).render(cli.pretty), ExitCode::from(failure.exit_code()))
        }
    }
}

#[cfg(test)]
mod tests {
    /// A stdout that fails every write with `kind`.
    struct Failing(std::io::ErrorKind);

    impl std::io::Write for Failing {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(self.0.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_closed_pipe_is_fine_and_other_write_failures_are_not() {
        assert_eq!(super::write_document(&mut Failing(std::io::ErrorKind::BrokenPipe), "{}\n"), Ok(()));
        assert_eq!(super::write_document(&mut Failing(std::io::ErrorKind::StorageFull), "{}\n"), Err(()));
        assert_eq!(super::write_document(&mut Vec::new(), "{}\n"), Ok(()));
    }

    #[test]
    fn a_panic_becomes_an_internal_error() {
        let failure = super::guarded(|| panic!("boom")).err().expect("a failure");
        assert_eq!((failure.code, failure.exit_code()), ("internal_error", 1));
        assert!(failure.message.contains("boom") && failure.message.contains("main.rs"), "{}", failure.message);
    }
}
