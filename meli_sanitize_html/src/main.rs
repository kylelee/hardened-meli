// SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later
use std::io::{Read, Write};

const USAGE: &str = "\
Usage: meli_sanitize_html

Reads HTML from stdin, sanitizes it with an allowlist, writes the
result to stdout.

Options:
  -h, --help     Print this help and exit
  -V, --version  Print version and exit
";

fn main() {
    // `args_os()` (not `args()`): must not panic on non-UTF-8 arguments.
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();

    // Decision order is fixed: help > version > error.
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print!("{USAGE}");
        std::process::exit(0);
    } else if args.iter().any(|arg| arg == "-V" || arg == "--version") {
        println!("meli_sanitize_html {}", env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    } else if let Some(arg) = args.first() {
        eprintln!("meli_sanitize_html: unexpected argument: {arg}");
        eprint!("{USAGE}");
        std::process::exit(2);
    }

    let mut input = Vec::new();
    if let Err(err) = std::io::stdin().read_to_end(&mut input) {
        eprintln!("meli_sanitize_html: failed to read stdin: {err}");
        std::process::exit(1);
    }
    let sanitized = meli_sanitize_html::sanitize(&String::from_utf8_lossy(&input));
    if let Err(err) = std::io::stdout().write_all(sanitized.as_bytes()) {
        eprintln!("meli_sanitize_html: failed to write stdout: {err}");
        std::process::exit(1);
    }
}
