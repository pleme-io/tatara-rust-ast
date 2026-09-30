//! `tatara-rust-emit` — a typed Rust AST, as JSON on stdin, rendered to Rust
//! source on stdout.
//!
//! The process-boundary door into the emitter for a compiler that is not
//! written in Rust (blue's native backend builds a [`File`] as data). The JSON
//! is deserialized into the typed AST first, so an unknown node kind or a
//! missing field is refused at the border with serde's message and exit 1;
//! nothing reaches the renderer as text.

use std::io::{Read, Write};
use std::process::ExitCode;

use tatara_rust_ast::File;
use tatara_rust_emit::emit_file;

fn main() -> ExitCode {
    let mut input = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut input) {
        eprintln!("tatara-rust-emit: cannot read stdin: {e}");
        return ExitCode::FAILURE;
    }
    let file: File = match serde_json::from_str(&input) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("tatara-rust-emit: not a typed Rust file: {e}");
            return ExitCode::FAILURE;
        }
    };
    match emit_file(&file) {
        Ok(text) => {
            if std::io::stdout().write_all(text.as_bytes()).is_err() {
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("tatara-rust-emit: {e}");
            ExitCode::FAILURE
        }
    }
}
