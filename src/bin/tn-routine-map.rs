use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::process;

use actionc::codegen::{CODE_ORIGIN, generate_compatible_with_origin};
use actionc::includes::load_program_with_includes;
use actionc::semantic::analyze;

const RUNAD: u16 = 0x02E2;
const JSR_ABS: u8 = 0x20;
const JMP_ABS: u8 = 0x4C;
const INFER_WINDOW: u16 = 64;

#[derive(Debug)]
struct Segment {
    start: u16,
    end: u16,
    data: Vec<u8>,
}

#[derive(Debug, Clone)]
struct TargetUse {
    site: u16,
    opcode: u8,
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() != 2 {
        eprintln!("usage: tn-routine-map <TN.ACT> <original-TN.COM>");
        process::exit(2);
    }

    let program = load_program_with_includes(&args[0]).unwrap_or_else(|diagnostics| {
        print_diagnostics(diagnostics);
        process::exit(1);
    });
    if let Err(diagnostics) = analyze(&program) {
        print_diagnostics(diagnostics);
        process::exit(1);
    }
    let output =
        generate_compatible_with_origin(&program, CODE_ORIGIN).unwrap_or_else(|diagnostics| {
            print_diagnostics(diagnostics);
            process::exit(1);
        });

    let original_bytes = fs::read(&args[1]).unwrap_or_else(|err| {
        eprintln!("read {}: {err}", args[1]);
        process::exit(1);
    });
    let segments = parse_load_file(&original_bytes).unwrap_or_else(|err| {
        eprintln!("{}: {err}", args[1]);
        process::exit(1);
    });
    let Some(code_segment) = segments
        .iter()
        .filter(|segment| !(segment.start <= RUNAD && RUNAD.wrapping_add(1) <= segment.end))
        .max_by_key(|segment| segment.data.len())
    else {
        eprintln!("{}: no code segment found", args[1]);
        process::exit(1);
    };
    let original_runad = segments
        .iter()
        .find_map(|segment| vector_value(segment, RUNAD));
    let target_uses = internal_target_uses(code_segment);
    let targets: BTreeSet<u16> = target_uses.keys().copied().collect();
    let actionc_end = output
        .origin
        .wrapping_add(output.bytes.len().saturating_sub(1) as u16);

    println!("# TN Routine Map");
    println!();
    println!("Source: `{}`", args[0]);
    println!("Original: `{}`", args[1]);
    println!();
    println!("| Metric | Original | actionc | Delta |");
    println!("| --- | ---: | ---: | ---: |");
    println!(
        "| Code segment | ${:04X}-${:04X} | ${:04X}-${:04X} | {} bytes |",
        code_segment.start,
        code_segment.end,
        output.origin,
        actionc_end,
        (output.bytes.len() as isize) - (code_segment.data.len() as isize)
    );
    println!(
        "| RUNAD | {} | ${:04X} | {} |",
        original_runad
            .map(format_address)
            .unwrap_or_else(|| "?".to_string()),
        output.run_address,
        original_runad
            .map(|addr| signed_delta(output.run_address, addr).to_string())
            .unwrap_or_else(|| "?".to_string())
    );
    println!();

    let anchor_drift = original_runad.map(|addr| output.run_address.wrapping_sub(addr));
    println!(
        "Inference anchor: last routine/RUNAD drift {}.",
        anchor_drift
            .map(|drift| format!("{} bytes", drift))
            .unwrap_or_else(|| "unknown".to_string())
    );
    println!(
        "Original routine addresses below are inferred from internal `JSR`/`JMP` targets within +/- {INFER_WINDOW} bytes of the anchored actionc address."
    );
    println!();
    println!("| Routine | actionc | Expected original | Inferred original | Drift | Target uses |");
    println!("| --- | ---: | ---: | ---: | ---: | ---: |");

    let mut used_targets = BTreeSet::new();
    for routine in &output.routine_addresses {
        let expected = anchor_drift.map(|drift| routine.address.wrapping_sub(drift));
        let inferred = if Some(routine.address) == Some(output.run_address) {
            original_runad
        } else {
            expected.and_then(|expected| {
                nearest_unused_target(expected, &targets, &used_targets).inspect(|target| {
                    used_targets.insert(*target);
                })
            })
        };
        let uses = inferred
            .and_then(|addr| target_uses.get(&addr))
            .map(|uses| uses.len())
            .unwrap_or(0);
        println!(
            "| {} | ${:04X} | {} | {} | {} | {} |",
            routine.name,
            routine.address,
            expected
                .map(format_address)
                .unwrap_or_else(|| "?".to_string()),
            inferred
                .map(format_address)
                .unwrap_or_else(|| "".to_string()),
            inferred
                .map(|addr| signed_delta(routine.address, addr).to_string())
                .unwrap_or_else(|| "".to_string()),
            uses
        );
    }

    println!();
    println!("## Tail Target Uses");
    println!();
    println!("| Target | Uses | Sites |");
    println!("| ---: | ---: | --- |");
    let tail_start = original_runad
        .map(|runad| runad.saturating_sub(0x0800))
        .unwrap_or(code_segment.start);
    for (target, uses) in target_uses.range(tail_start..) {
        let sites = uses
            .iter()
            .take(8)
            .map(|use_site| format!("{}@${:04X}", opcode_name(use_site.opcode), use_site.site))
            .collect::<Vec<_>>()
            .join(", ");
        println!("| ${target:04X} | {} | {sites} |", uses.len());
    }
}

fn nearest_unused_target(
    expected: u16,
    targets: &BTreeSet<u16>,
    used: &BTreeSet<u16>,
) -> Option<u16> {
    targets
        .range(expected.saturating_sub(INFER_WINDOW)..=expected.saturating_add(INFER_WINDOW))
        .filter(|target| !used.contains(target))
        .min_by_key(|target| target.abs_diff(expected))
        .copied()
}

fn parse_load_file(bytes: &[u8]) -> Result<Vec<Segment>, String> {
    let mut offset = 0usize;
    let mut segments = Vec::new();

    while offset < bytes.len() {
        if bytes.len().saturating_sub(offset) >= 2 && read_word(bytes, offset) == 0xFFFF {
            offset += 2;
        }
        if offset == bytes.len() {
            break;
        }
        if bytes.len().saturating_sub(offset) < 4 {
            return Err(format!("truncated segment header at file offset {offset}"));
        }
        let start = read_word(bytes, offset);
        let end = read_word(bytes, offset + 2);
        offset += 4;
        if end < start {
            return Err(format!("invalid segment ${start:04X}-${end:04X}"));
        }
        let len = usize::from(end.wrapping_sub(start).wrapping_add(1));
        if bytes.len().saturating_sub(offset) < len {
            return Err(format!("segment ${start:04X}-${end:04X} is truncated"));
        }
        segments.push(Segment {
            start,
            end,
            data: bytes[offset..offset + len].to_vec(),
        });
        offset += len;
    }

    Ok(segments)
}

fn internal_target_uses(segment: &Segment) -> BTreeMap<u16, Vec<TargetUse>> {
    let mut uses: BTreeMap<u16, Vec<TargetUse>> = BTreeMap::new();
    for offset in 0..segment.data.len().saturating_sub(2) {
        let opcode = segment.data[offset];
        if !matches!(opcode, JSR_ABS | JMP_ABS) {
            continue;
        }
        let target = read_word(&segment.data, offset + 1);
        if target < segment.start || target > segment.end {
            continue;
        }
        uses.entry(target).or_default().push(TargetUse {
            site: segment.start.wrapping_add(offset as u16),
            opcode,
        });
    }
    uses
}

fn vector_value(segment: &Segment, address: u16) -> Option<u16> {
    if segment.start > address || segment.end < address.wrapping_add(1) {
        return None;
    }
    let offset = usize::from(address.wrapping_sub(segment.start));
    Some(read_word(&segment.data, offset))
}

fn read_word(bytes: &[u8], offset: usize) -> u16 {
    u16::from(bytes[offset]) | (u16::from(bytes[offset + 1]) << 8)
}

fn signed_delta(actionc: u16, original: u16) -> i32 {
    i32::from(actionc) - i32::from(original)
}

fn format_address(address: u16) -> String {
    format!("${address:04X}")
}

fn opcode_name(opcode: u8) -> &'static str {
    match opcode {
        JSR_ABS => "JSR",
        JMP_ABS => "JMP",
        _ => "???",
    }
}

fn print_diagnostics(diagnostics: Vec<actionc::diagnostic::Diagnostic>) {
    for diagnostic in diagnostics {
        eprintln!(
            "{}..{}: {}",
            diagnostic.span.start, diagnostic.span.end, diagnostic.message
        );
    }
}
