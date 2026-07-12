use std::env;
use std::fs;
use std::process;

use actionc::codegen::format_listing_with_origin;

const INITAD: u16 = 0x02E0;
const RUNAD: u16 = 0x02E2;

#[derive(Debug)]
struct Options {
    paths: Vec<String>,
    disasm: Option<AddressRange>,
}

#[derive(Clone, Copy, Debug)]
struct AddressRange {
    start: u16,
    end: u16,
}

#[derive(Debug)]
struct Segment {
    index: usize,
    header_offset: usize,
    data_offset: usize,
    start: u16,
    end: u16,
    data: Vec<u8>,
}

fn main() {
    let options = parse_options();

    let multiple = options.paths.len() > 1;
    for path in options.paths {
        if multiple {
            println!("{path}:");
        }
        let bytes = fs::read(&path).unwrap_or_else(|err| {
            eprintln!("read {path}: {err}");
            process::exit(1);
        });
        match parse_load_file(&bytes) {
            Ok(segments) => print_segments(&path, &bytes, &segments, multiple, options.disasm),
            Err(err) => {
                eprintln!("{path}: {err}");
                process::exit(1);
            }
        }
    }
}

fn parse_options() -> Options {
    let mut args = env::args().skip(1);
    let mut paths = Vec::new();
    let mut disasm = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                process::exit(0);
            }
            "--disasm" => {
                let Some(value) = args.next() else {
                    eprintln!("--disasm needs a range like $5A00-$5B53");
                    print_help();
                    process::exit(2);
                };
                disasm = Some(parse_address_range(&value).unwrap_or_else(|err| {
                    eprintln!("{err}");
                    print_help();
                    process::exit(2);
                }));
            }
            _ if arg.starts_with('-') => {
                eprintln!("unknown option `{arg}`");
                print_help();
                process::exit(2);
            }
            _ => paths.push(arg),
        }
    }

    if paths.is_empty() {
        print_help();
        process::exit(2);
    }

    Options { paths, disasm }
}

fn print_help() {
    eprintln!("usage: atari-load-info [--disasm <start-end>] <file.com|file.xex> [more files...]");
}

fn parse_load_file(bytes: &[u8]) -> Result<Vec<Segment>, String> {
    let mut offset = 0usize;
    let mut index = 0usize;
    let mut segments = Vec::new();

    while offset < bytes.len() {
        if bytes.len().saturating_sub(offset) >= 2 && read_word(bytes, offset) == 0xFFFF {
            offset += 2;
        }
        if offset == bytes.len() {
            break;
        }
        if bytes.len().saturating_sub(offset) < 4 {
            return Err(format!(
                "truncated segment header at file offset {offset} (${offset:04X})"
            ));
        }

        let header_offset = offset;
        let start = read_word(bytes, offset);
        let end = read_word(bytes, offset + 2);
        offset += 4;
        if end < start {
            return Err(format!(
                "segment {index} has invalid range ${start:04X}-${end:04X}"
            ));
        }

        let len = usize::from(end.wrapping_sub(start).wrapping_add(1));
        if bytes.len().saturating_sub(offset) < len {
            return Err(format!(
                "segment {index} ${start:04X}-${end:04X} needs {len} data bytes, only {} remain",
                bytes.len().saturating_sub(offset)
            ));
        }

        let data_offset = offset;
        let data = bytes[offset..offset + len].to_vec();
        offset += len;
        segments.push(Segment {
            index,
            header_offset,
            data_offset,
            start,
            end,
            data,
        });
        index += 1;
    }

    Ok(segments)
}

fn read_word(bytes: &[u8], offset: usize) -> u16 {
    u16::from(bytes[offset]) | (u16::from(bytes[offset + 1]) << 8)
}

fn print_segments(
    path: &str,
    bytes: &[u8],
    segments: &[Segment],
    indent: bool,
    disasm: Option<AddressRange>,
) {
    let prefix = if indent { "  " } else { "" };
    println!("{prefix}file_size: {}", bytes.len());
    println!("{prefix}segments: {}", segments.len());

    let mut previous: Option<&Segment> = None;
    for segment in segments {
        let len = segment.data.len();
        println!(
            "{prefix}seg {:02}: ${:04X}-${:04X} len {:5} header_off {:5} data_off {:5}",
            segment.index,
            segment.start,
            segment.end,
            len,
            segment.header_offset,
            segment.data_offset
        );

        if let Some(previous) = previous {
            if previous.end < segment.start {
                let gap = segment.start.wrapping_sub(previous.end).wrapping_sub(1);
                println!(
                    "{prefix}        load gap after seg {:02}: ${:04X}-${:04X} len {}",
                    previous.index,
                    previous.end.wrapping_add(1),
                    segment.start.wrapping_sub(1),
                    gap
                );
            } else if segment.start <= previous.end {
                println!(
                    "{prefix}        overlaps/touches previous load range ending at ${:04X}",
                    previous.end
                );
            }
        }

        if let Some(vector) = vector_value(segment, INITAD) {
            println!("{prefix}        INITAD ${INITAD:04X} = ${vector:04X}");
        }
        if let Some(vector) = vector_value(segment, RUNAD) {
            println!("{prefix}        RUNAD  ${RUNAD:04X} = ${vector:04X}");
        }

        println!(
            "{prefix}        first: {}",
            format_bytes(&segment.data[..segment.data.len().min(16)])
        );
        let tail_start = segment.data.len().saturating_sub(16);
        println!(
            "{prefix}        last : {}",
            format_bytes(&segment.data[tail_start..])
        );

        if let Some(range) = disasm {
            print_disasm(segment, range, prefix);
        }
        previous = Some(segment);
    }

    if segments.is_empty() {
        println!("{prefix}warning: no segments parsed from {path}");
    }
}

fn print_disasm(segment: &Segment, range: AddressRange, prefix: &str) {
    if segment.end < range.start || segment.start > range.end {
        return;
    }

    let start = segment.start.max(range.start);
    let end = segment.end.min(range.end);
    let offset = usize::from(start.wrapping_sub(segment.start));
    let len = usize::from(end.wrapping_sub(start).wrapping_add(1));

    println!("{prefix}        disasm ${start:04X}-${end:04X}:");
    for line in format_listing_with_origin(&segment.data[offset..offset + len], start).lines() {
        println!("{prefix}          {line}");
    }
}

fn vector_value(segment: &Segment, address: u16) -> Option<u16> {
    if segment.start > address || segment.end < address.wrapping_add(1) {
        return None;
    }
    let offset = usize::from(address.wrapping_sub(segment.start));
    Some(u16::from(segment.data[offset]) | (u16::from(segment.data[offset + 1]) << 8))
}

fn parse_address_range(value: &str) -> Result<AddressRange, String> {
    let Some((start, end)) = value.split_once('-') else {
        return Err(format!("invalid disasm range `{value}`"));
    };
    let start = parse_address(start)?;
    let end = parse_address(end)?;
    if end < start {
        return Err(format!("invalid disasm range `{value}`"));
    }
    Ok(AddressRange { start, end })
}

fn parse_address(value: &str) -> Result<u16, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("empty address".to_string());
    }

    let (radix, digits) = if let Some(hex) = value.strip_prefix('$') {
        (16, hex)
    } else if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        (16, hex)
    } else {
        (10, value)
    };

    u16::from_str_radix(digits, radix).map_err(|_| format!("invalid address `{value}`"))
}

fn format_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}
