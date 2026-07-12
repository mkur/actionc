use std::env;
use std::fs;
use std::io::{self, Write};
use std::process;

use atrcopy_rs::atascii_to_ascii;

fn main() {
    let mut args = env::args().skip(1);
    let Some(input) = args.next() else {
        eprintln!("usage: atascii-to-ascii <input> [output]");
        process::exit(2);
    };
    let output = args.next();
    if args.next().is_some() {
        eprintln!("usage: atascii-to-ascii <input> [output]");
        process::exit(2);
    }

    let bytes = match fs::read(&input) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("failed to read {input}: {err}");
            process::exit(1);
        }
    };
    let converted = atascii_to_ascii(&bytes);

    if let Some(output) = output {
        if let Err(err) = fs::write(&output, converted) {
            eprintln!("failed to write {output}: {err}");
            process::exit(1);
        }
    } else {
        let mut stdout = io::stdout().lock();
        if let Err(err) = stdout.write_all(converted.as_bytes()) {
            eprintln!("failed to write stdout: {err}");
            process::exit(1);
        }
    }
}
