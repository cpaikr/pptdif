# Installing pptdif

`pptdif` is a single executable with no runtime dependencies; it does not
need PowerPoint (only `--images` in the HTML report does).

## From a release

Releases carry one archive, for Windows x64. When the repository has
published releases, download it from the Releases page, extract
`pptdif.exe`, and put it on `PATH`. Check with `pptdif --version`. On other
platforms, build from source.

## From source

With a Rust toolchain (1.88 or later):

```bash
cargo install --locked --git https://github.com/cpaikr/pptdif pptdif-cli
```

or, in a checkout:

```bash
cargo build --release
```

and use `target/release/pptdif` (`pptdif.exe` on Windows).

## After installing

Run `pptdif --version`, then resume the task. A comparison failure with an
`error.code` (for example `unsupported_format` for a legacy `.ppt` file) is
not an installation problem; read its `hint`.
