//! BR-TOK-01R demo: load the pinned tokenizer.json from a caller-supplied
//! path and print ONLY the contract facts: byte length, SHA match, vocab,
//! merges, added-token counts. The path is never hardcoded in library code;
//! the operator passes it on the command line.

use lfm3_brain::tokenizer::{load_tokenizer, TokenLimits, TOKENIZER_BYTES, TOKENIZER_SHA256};
use std::path::Path;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: tokenizer_load_demo <path-to-tokenizer.json>");
        std::process::exit(2);
    };
    match run(Path::new(&path)) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("rejected: {e}");
            std::process::exit(1);
        }
    }
}

fn run(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let (report, tables) = load_tokenizer(
        path,
        TOKENIZER_BYTES,
        TOKENIZER_SHA256,
        &TokenLimits::default(),
    )?;
    let sha_match = if report.sha256 == TOKENIZER_SHA256 {
        "match"
    } else {
        "MISMATCH"
    };
    println!("length {}", report.byte_length);
    println!("sha256 {sha_match}");
    println!("vocab {}", tables.vocab.len());
    println!("merges {}", tables.merges.len());
    println!("added {}", tables.added_tokens.len());
    Ok(())
}
