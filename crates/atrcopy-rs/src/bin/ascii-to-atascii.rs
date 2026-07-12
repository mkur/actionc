use std::env;
use std::fs;
use std::io::{self, Write};
use std::process;

use atrcopy_rs::ascii_to_atascii;

fn main() {
    let mut args = env::args().skip(1);
    let Some(input) = args.next() else {
        eprintln!("usage: ascii-to-atascii <input> [output]");
        process::exit(2);
    };
    let output = args.next();
    if args.next().is_some() {
        eprintln!("usage: ascii-to-atascii <input> [output]");
        process::exit(2);
    }

    let text = match fs::read_to_string(&input) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("failed to read {input}: {err}");
            process::exit(1);
        }
    };
    let converted = match ascii_to_atascii(&text) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("{input}: {err}");
            process::exit(1);
        }
    };

    if let Some(output) = output {
        if let Err(err) = fs::write(&output, converted) {
            eprintln!("failed to write {output}: {err}");
            process::exit(1);
        }
    } else {
        let mut stdout = io::stdout().lock();
        if let Err(err) = stdout.write_all(&converted) {
            eprintln!("failed to write stdout: {err}");
            process::exit(1);
        }
    }
}
