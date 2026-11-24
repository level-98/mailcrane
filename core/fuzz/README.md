# Fuzzing

Fuzz targets for the parsers that read hostile input. They need the nightly
toolchain and [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz), so this
crate is excluded from the stable workspace and `cargo xtask ci` never builds
it. The nightly [Fuzz workflow](../../.github/workflows/fuzz.yml) runs each
target for 10 minutes and keeps the corpus between runs.

| Target | What it fuzzes | Seed corpus |
|---|---|---|
| `mime_parse` | `mailcrane_mime::parse_message` | `tests/fixtures/mime/*.eml` |
| `imap_response` | `mailcrane_imap::parse_response`, over a stream of responses | `tests/fixtures/imap/*.imap` |
| `html_sanitize` | `mailcrane_mime::sanitize_html`, on input wrapped in a `text/html` message | `tests/fixtures/html/*.html` |

## Running a target

```bash
rustup toolchain install nightly
cargo install cargo-fuzz
cd core
cargo xtask fuzz mime_parse --seconds 60
```

`cargo xtask fuzz` copies the seeds into `core/fuzz/corpus/<target>/` (ignored
by git; add new seeds to `tests/fixtures/` instead), then runs the target.

**Windows:** libFuzzer only links with AddressSanitizer there, so the MSVC ASan
runtime has to be on `PATH`. Add the folder that holds
`clang_rt.asan_dynamic-x86_64.dll`, usually
`<Visual Studio>\VC\Tools\MSVC\<version>\bin\Hostx64\x64`. If rustup's default
host is `x86_64-pc-windows-gnu`, also run
`rustup target add x86_64-pc-windows-msvc --toolchain nightly`.

## When a target crashes

Any panic, hang (one input taking over 10 seconds) or runaway allocation is a
finding. The input is saved in `core/fuzz/artifacts/<target>/` (the nightly
workflow uploads it as the `fuzz-crash-<target>` artifact).

1. Reproduce it: `cargo +nightly fuzz run <target> artifacts/<target>/<file>`
   from `core/fuzz/`.
2. Shrink it: `cargo +nightly fuzz tmin <target> artifacts/<target>/<file>`.
3. Make it a regression test in the parser crate, using the shrunk bytes (or a
   synthetic fixture in `tests/fixtures/` that triggers the same bug). The test
   must fail before the fix.
4. Fix the parser in its own PR, following the "Never crash" rules in
   [AGENTS.md](../../AGENTS.md). Look for other places with the same root cause.

Crash inputs can contain anything the fuzzer made up, but never paste one from
a real mailbox into an issue.
