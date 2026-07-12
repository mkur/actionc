use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use actionc::codegen::{
    AddressingMode, CODE_ORIGIN, CodegenMap, CodegenOptimization, CodegenOptimizationKind,
    CodegenOutput, CodegenProfile, CodegenSourceRange, CodegenSourceRangeKind,
    CodegenStorageSymbol, CodegenSymbolScope, DisassembledInstruction, RoutineRange, SkippedRange,
    disassemble_with_origin, format_load_file, generate_profile_with_origin,
};
use actionc::diagnostic::Diagnostic;
use actionc::includes::load_program_with_includes;
use actionc::map_query::{MapQuery, StorageOwner, source_location};
use actionc::semantic::analyze;
use actionc::source::decode_source;

const RUNAD: u16 = 0x02E2;

#[derive(Debug)]
struct Options {
    source: PathBuf,
    source_text: String,
    original: Option<PathBuf>,
    original_symbols: Option<PathBuf>,
    original_symbol_snapshots: Option<PathBuf>,
    disassemble_original: bool,
    origin: Option<u16>,
    max_diffs: usize,
    mode: CompareMode,
}

#[derive(Debug)]
struct Artifact {
    name: String,
    load_bytes: Vec<u8>,
    code_origin: u16,
    code_bytes: Vec<u8>,
    segments: Vec<Segment>,
    map: Option<CodegenMap>,
    optimizations: Vec<CodegenOptimization>,
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

#[derive(Debug, Clone)]
struct OriginalSymbolSet {
    labels: BTreeMap<u16, Vec<OriginalSymbol>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OriginalRoutineRange {
    name: String,
    start: u16,
    end: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OriginalSymbol {
    name: String,
    class: String,
    size: u16,
    source: OriginalSymbolSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OriginalSymbolSource {
    Final,
    Snapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SourceRangeKey {
    kind: SourceRangeKindKey,
    name: Option<String>,
    span_start: usize,
    span_end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SourceRangeKindKey {
    Routine,
    Statement,
    Expression,
    Declaration,
    StorageInitializer,
    MachineBlock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompareMode {
    All,
    Compat,
    Modern,
    Profiles,
}

impl CompareMode {
    fn includes_compat(self) -> bool {
        matches!(self, Self::All | Self::Compat)
    }

    fn includes_modern(self) -> bool {
        matches!(self, Self::All | Self::Modern)
    }

    fn includes_profiles(self) -> bool {
        matches!(self, Self::All | Self::Profiles)
    }
}

struct SourceRangeDelta {
    key: SourceRangeKey,
    left_range: Option<CodegenSourceRange>,
    right_range: Option<CodegenSourceRange>,
    diff_count: usize,
}

fn main() {
    let options = parse_options();
    let program = load_program_with_includes(&options.source)
        .and_then(|program| analyze(&program).map(|_| program))
        .unwrap_or_else(|diagnostics| {
            print_diagnostics(diagnostics);
            process::exit(1);
        });

    let original = options.original.as_ref().map(read_artifact);
    let origin = options
        .origin
        .or_else(|| original.as_ref().map(|artifact| artifact.code_origin))
        .unwrap_or(CODE_ORIGIN);
    let compat = (options.mode.includes_compat() || options.mode.includes_profiles())
        .then(|| compile_artifact(&program, CodegenProfile::Compat, "legacy", origin));
    let modern = (options.mode.includes_modern() || options.mode.includes_profiles())
        .then(|| compile_artifact(&program, CodegenProfile::Modern, "modern", origin));
    let original_symbols = load_original_symbols(
        options.original_symbols.as_ref(),
        options.original_symbol_snapshots.as_ref(),
    );

    println!("source: {}", format_compact_path(&options.source));
    if let Some(original) = &original {
        print_artifact(original);
        if options.disassemble_original {
            print_original_disassembly(original, original_symbols.as_ref());
        }
    }
    if options.mode.includes_compat() || options.mode.includes_profiles() {
        print_artifact(compat.as_ref().expect("compat artifact"));
    }
    if options.mode.includes_modern() || options.mode.includes_profiles() {
        print_artifact(modern.as_ref().expect("modern artifact"));
    }

    if options.mode.includes_compat() || options.mode.includes_profiles() {
        print_map_summary(compat.as_ref().expect("compat artifact"));
    }
    if options.mode.includes_modern() || options.mode.includes_profiles() {
        print_map_summary(modern.as_ref().expect("modern artifact"));
    }
    if let (Some(original), Some(original_symbols)) = (&original, &original_symbols) {
        if options.mode.includes_compat() {
            print_original_routine_comparison(
                original,
                original_symbols,
                compat.as_ref().expect("compat artifact"),
                &options.source,
                &options.source_text,
                options.max_diffs,
            );
        }
        if options.mode.includes_modern() {
            print_original_routine_comparison(
                original,
                original_symbols,
                modern.as_ref().expect("modern artifact"),
                &options.source,
                &options.source_text,
                options.max_diffs,
            );
        }
    }
    if options.mode.includes_profiles() {
        let compat = compat.as_ref().expect("compat artifact");
        let modern = modern.as_ref().expect("modern artifact");
        print_expected_modern_wins(modern);
        print_routine_range_comparison(
            compat,
            modern,
            &options.source,
            &options.source_text,
            options.max_diffs,
        );
    }

    if let Some(original) = &original {
        if options.mode.includes_compat() {
            let compat = compat.as_ref().expect("compat artifact");
            print_byte_comparison(original, compat, options.max_diffs);
            print_instruction_diff(original, compat, options.max_diffs);
        }
        if options.mode.includes_modern() {
            let modern = modern.as_ref().expect("modern artifact");
            print_byte_comparison(original, modern, options.max_diffs);
            print_instruction_diff(original, modern, options.max_diffs);
        }
    }
    if options.mode.includes_profiles() {
        let compat = compat.as_ref().expect("compat artifact");
        let modern = modern.as_ref().expect("modern artifact");
        print_byte_comparison(compat, modern, options.max_diffs);
        print_instruction_diff(compat, modern, options.max_diffs);
    }
}

fn parse_options() -> Options {
    let mut args = env::args().skip(1);
    let mut original = None;
    let mut original_symbols = None;
    let mut original_symbol_snapshots = None;
    let mut disassemble_original = false;
    let mut origin = None;
    let mut max_diffs = 12usize;
    let mut mode = CompareMode::All;
    let mut source = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                process::exit(0);
            }
            "--original" => {
                let Some(path) = args.next() else {
                    eprintln!("--original requires a .COM/.XEX path");
                    print_help();
                    process::exit(2);
                };
                original = Some(PathBuf::from(path));
            }
            "--original-symbols" => {
                let Some(path) = args.next() else {
                    eprintln!("--original-symbols requires a .symbols.json path");
                    print_help();
                    process::exit(2);
                };
                original_symbols = Some(PathBuf::from(path));
            }
            "--original-symbol-snapshots" => {
                let Some(path) = args.next() else {
                    eprintln!("--original-symbol-snapshots requires a .symbol-snapshots.json path");
                    print_help();
                    process::exit(2);
                };
                original_symbol_snapshots = Some(PathBuf::from(path));
            }
            "--disassemble-original" => {
                disassemble_original = true;
            }
            "--origin" => {
                let Some(value) = args.next() else {
                    eprintln!("--origin requires an address");
                    print_help();
                    process::exit(2);
                };
                origin = Some(parse_address(&value).unwrap_or_else(|err| {
                    eprintln!("{err}");
                    process::exit(2);
                }));
            }
            "--max-diffs" => {
                let Some(value) = args.next() else {
                    eprintln!("--max-diffs requires a number");
                    print_help();
                    process::exit(2);
                };
                max_diffs = value.parse().unwrap_or_else(|_| {
                    eprintln!("invalid --max-diffs value: {value}");
                    process::exit(2);
                });
            }
            "--mode" => {
                let Some(value) = args.next() else {
                    eprintln!("--mode requires one of: all, legacy, modern, profiles");
                    print_help();
                    process::exit(2);
                };
                mode = parse_compare_mode(&value).unwrap_or_else(|| {
                    eprintln!("invalid --mode value: {value}");
                    print_help();
                    process::exit(2);
                });
            }
            _ if arg.starts_with('-') => {
                eprintln!("unknown option: {arg}");
                print_help();
                process::exit(2);
            }
            _ if source.is_none() => source = Some(PathBuf::from(arg)),
            _ => {
                eprintln!("unexpected argument: {arg}");
                print_help();
                process::exit(2);
            }
        }
    }

    let Some(source) = source else {
        print_help();
        process::exit(2);
    };
    let source_bytes = fs::read(&source).unwrap_or_else(|err| {
        eprintln!("read {}: {err}", source.display());
        process::exit(1);
    });
    let source_text = decode_source(&source_bytes);

    Options {
        source,
        source_text,
        original,
        original_symbols,
        original_symbol_snapshots,
        disassemble_original,
        origin,
        max_diffs,
        mode,
    }
}

fn print_help() {
    eprintln!(
        "usage: actionc-compare [--origin <addr>] [--original <file.com>] [--original-symbols <file.symbols.json>] [--original-symbol-snapshots <file.symbol-snapshots.json>] [--disassemble-original] [--max-diffs <n>] [--mode all|legacy|modern|profiles] <source.act>"
    );
}

fn parse_compare_mode(value: &str) -> Option<CompareMode> {
    match value {
        "all" => Some(CompareMode::All),
        "legacy" | "compat" => Some(CompareMode::Compat),
        "modern" => Some(CompareMode::Modern),
        "profiles" | "profile" => Some(CompareMode::Profiles),
        _ => None,
    }
}

fn compile_artifact(
    program: &actionc::ast::Program,
    profile: CodegenProfile,
    name: &str,
    origin: u16,
) -> Artifact {
    let output = generate_profile_with_origin(program, origin, profile).unwrap_or_else(|err| {
        print_diagnostics(err);
        process::exit(1);
    });
    artifact_from_output(name.to_string(), &output)
}

fn artifact_from_output(name: String, output: &CodegenOutput) -> Artifact {
    let load_bytes = format_load_file(output);
    let segments = parse_load_file(&load_bytes).unwrap_or_else(|err| {
        eprintln!("{name}: generated invalid Atari load file: {err}");
        process::exit(1);
    });
    Artifact {
        name,
        load_bytes,
        code_origin: output.origin,
        code_bytes: output.bytes.clone(),
        segments,
        map: Some(output.map.clone()),
        optimizations: output.optimizations.clone(),
    }
}

fn read_artifact(path: &PathBuf) -> Artifact {
    let load_bytes = fs::read(path).unwrap_or_else(|err| {
        eprintln!("read {}: {err}", path.display());
        process::exit(1);
    });
    let segments = parse_load_file(&load_bytes).unwrap_or_else(|err| {
        eprintln!("{}: {err}", path.display());
        process::exit(1);
    });
    let code_segment = segments
        .iter()
        .find(|segment| !is_vector_segment(segment))
        .unwrap_or_else(|| {
            eprintln!("{}: no code/data segment found", path.display());
            process::exit(1);
        });
    Artifact {
        name: format!("original:{}", format_compact_path(path)),
        load_bytes,
        code_origin: code_segment.start,
        code_bytes: code_segment.data.clone(),
        segments,
        map: None,
        optimizations: Vec::new(),
    }
}

fn print_expected_modern_wins(artifact: &Artifact) {
    if artifact.optimizations.is_empty() {
        return;
    }

    println!();
    println!("== expected modern wins ==");
    let mut by_kind = BTreeMap::<&'static str, (usize, i16)>::new();
    for optimization in &artifact.optimizations {
        let entry = by_kind
            .entry(format_optimization_kind(optimization.kind))
            .or_insert((0, 0));
        entry.0 += 1;
        entry.1 += optimization.bytes_saved;
    }
    for (kind, (count, bytes_saved)) in by_kind {
        println!("  {kind:<24} count {count:<4} saved {bytes_saved:>4} bytes");
    }

    println!("details:");
    for optimization in &artifact.optimizations {
        let address = optimization
            .address
            .map(|address| format!("${address:04X}"))
            .unwrap_or_else(|| "-".to_string());
        let routine = optimization.routine.as_deref().unwrap_or("program");
        println!(
            "  {:<24} saved {:>3} bytes  {:<7} {:<16} {}",
            format_optimization_kind(optimization.kind),
            optimization.bytes_saved,
            address,
            routine,
            optimization.message
        );
    }
}

fn format_optimization_kind(kind: CodegenOptimizationKind) -> &'static str {
    match kind {
        CodegenOptimizationKind::TrampolineElided => "trampoline elided",
        CodegenOptimizationKind::FinalRtsRemoved => "final RTS removed",
        CodegenOptimizationKind::RegisterReloadRemoved => "register reload removed",
        CodegenOptimizationKind::ConstantStoreReusedRegister => "constant store reused register",
        CodegenOptimizationKind::CallResultMaterializationRemoved => {
            "call result materialization removed"
        }
        CodegenOptimizationKind::PointerReloadRemoved => "pointer reload removed",
        CodegenOptimizationKind::EffectiveAddressLowered => "effective address lowered",
        CodegenOptimizationKind::EffectiveAddressReused => "effective address reused",
        CodegenOptimizationKind::ArgumentStoreRemoved => "argument store removed",
        CodegenOptimizationKind::ArgumentStackForwarded => "argument stack forwarded",
        CodegenOptimizationKind::BranchInverted => "branch inverted",
        CodegenOptimizationKind::TailCall => "tail call",
        CodegenOptimizationKind::JumpToRtsRemoved => "jump to RTS removed",
        CodegenOptimizationKind::CallFactPreserved => "call fact preserved",
    }
}

fn print_artifact(artifact: &Artifact) {
    println!();
    println!("== {} ==", artifact.name);
    println!(
        "load bytes: {}  code bytes: {}  code origin: ${:04X}",
        artifact.load_bytes.len(),
        artifact.code_bytes.len(),
        artifact.code_origin
    );
    println!("segments:");
    for segment in &artifact.segments {
        println!(
            "  {:02}: ${:04X}-${:04X} len {:5} header_off {:5} data_off {:5}",
            segment.index,
            segment.start,
            segment.end,
            segment.data.len(),
            segment.header_offset,
            segment.data_offset
        );
        if let Some(runad) = vector_value(segment, RUNAD) {
            println!("      RUNAD ${RUNAD:04X} = ${runad:04X}");
        }
    }
    println!("opcode counts:");
    for (opcode, count) in opcode_counts(artifact) {
        println!("  {opcode:<12} {count}");
    }
}

fn print_byte_comparison(left: &Artifact, right: &Artifact, max_diffs: usize) {
    println!();
    println!("== bytes: {} vs {} ==", left.name, right.name);
    println!(
        "exact: {}  sizes: {} vs {}",
        left.load_bytes == right.load_bytes,
        left.load_bytes.len(),
        right.load_bytes.len()
    );
    let diffs = byte_diffs(&left.load_bytes, &right.load_bytes, max_diffs);
    if diffs.is_empty() {
        println!("first diffs: none");
    } else {
        println!("first diffs:");
        for diff in diffs {
            println!("  {diff}");
        }
    }
}

fn print_instruction_diff(left: &Artifact, right: &Artifact, max_diffs: usize) {
    let left_disasm = disassemble_with_origin(&left.code_bytes, left.code_origin);
    let right_disasm = disassemble_with_origin(&right.code_bytes, right.code_origin);
    println!();
    println!("== instructions: {} vs {} ==", left.name, right.name);
    println!("counts: {} vs {}", left_disasm.len(), right_disasm.len());

    let mut shown = 0usize;
    let max_len = left_disasm.len().max(right_disasm.len());
    for index in 0..max_len {
        let left_item = left_disasm.get(index);
        let right_item = right_disasm.get(index);
        if normalized(left_item) == normalized(right_item) {
            continue;
        }
        println!("  #{index:04}");
        println!("    left : {}", format_disasm_item(left_item));
        println!("    right: {}", format_disasm_item(right_item));
        shown += 1;
        if shown >= max_diffs {
            break;
        }
    }
    if shown == 0 {
        println!("first diffs: none");
    }
}

fn print_map_summary(artifact: &Artifact) {
    let Some(map) = &artifact.map else {
        return;
    };

    println!();
    println!("== map: {} ==", artifact.name);
    println!(
        "origin ${:04X}  run ${:04X}  routines {}  symbols {}  skipped {}",
        map.origin,
        map.run_address,
        map.routine_ranges.len(),
        map.storage_symbols.len(),
        map.skipped_ranges.len()
    );
    if !map.routine_ranges.is_empty() {
        println!("routines:");
        for routine in &map.routine_ranges {
            println!(
                "  {:<24} ${:04X}-${:04X} len {:5}",
                routine.name,
                routine.start,
                routine.end.saturating_sub(1),
                routine_range_len(routine)
            );
        }
    }
    if !map.skipped_ranges.is_empty() {
        println!("skipped ranges:");
        for range in map.skipped_ranges.iter().take(32) {
            println!("  {}", format_skipped_range(artifact, range));
        }
        if map.skipped_ranges.len() > 32 {
            println!("  ... {} more", map.skipped_ranges.len() - 32);
        }
    }
    if !map.storage_symbols.is_empty() {
        println!("storage symbols:");
        let mut symbols = map.storage_symbols.iter().collect::<Vec<_>>();
        symbols.sort_by(|left, right| {
            left.address
                .cmp(&right.address)
                .then_with(|| left.size.cmp(&right.size))
                .then_with(|| left.name.cmp(&right.name))
        });
        for symbol in symbols.into_iter().take(32) {
            println!("  {}", format_storage_symbol(symbol));
        }
        if map.storage_symbols.len() > 32 {
            println!("  ... {} more", map.storage_symbols.len() - 32);
        }
    }
    if !map.routine_effects.is_empty() {
        println!("trusted effects:");
        for effect in map.routine_effects.iter().take(32) {
            println!("  {:<24} {}", effect.routine, effect.summary);
        }
        if map.routine_effects.len() > 32 {
            println!("  ... {} more", map.routine_effects.len() - 32);
        }
    }
    if !map.machine_blocks.is_empty() {
        println!("machine-block advisory:");
        for analysis in map.machine_blocks.iter().take(32) {
            let routine = analysis.routine.as_deref().unwrap_or("program");
            println!(
                "  {:<24} ${:04X} {}",
                routine, analysis.address, analysis.summary
            );
        }
        if map.machine_blocks.len() > 32 {
            println!("  ... {} more", map.machine_blocks.len() - 32);
        }
    }
    print_effect_gaps(map);
}

fn print_effect_gaps(map: &CodegenMap) {
    let trusted = map
        .routine_effects
        .iter()
        .map(|effect| (effect.routine.as_str(), effect.summary.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut gaps = BTreeSet::new();

    for analysis in &map.machine_blocks {
        let routine = analysis.routine.as_deref().unwrap_or("program");
        let advisory = analysis.summary.as_str();
        let trusted_summary = trusted.get(routine).copied();

        if trusted_summary.is_none() && advisory_reports_writes(advisory) {
            gaps.insert(format!(
                "{routine:<24} advisory writes but no trusted effect: {advisory}"
            ));
            continue;
        }
        if let Some(trusted_summary) = trusted_summary {
            if advisory_is_opaque(advisory) {
                gaps.insert(format!(
                    "{routine:<24} trusted effect exists but advisory is opaque: {trusted_summary}"
                ));
            } else if trusted_is_broad(trusted_summary)
                && advisory_reports_specific_writes(advisory)
            {
                gaps.insert(format!(
                    "{routine:<24} trusted effect may be broad: trusted `{trusted_summary}` vs {advisory}"
                ));
            }
        }
    }

    if gaps.is_empty() {
        return;
    }
    println!("effect gaps:");
    for gap in gaps.iter().take(32) {
        println!("  {gap}");
    }
    if gaps.len() > 32 {
        println!("  ... {} more", gaps.len() - 32);
    }
}

fn advisory_reports_writes(summary: &str) -> bool {
    summary.contains("writes") || summary.contains("unknown absolute write")
}

fn advisory_reports_specific_writes(summary: &str) -> bool {
    summary.contains("zp writes") || summary.contains("absolute writes")
}

fn advisory_is_opaque(summary: &str) -> bool {
    summary.contains("opaque/unsupported")
}

fn trusted_is_broad(summary: &str) -> bool {
    summary.contains("$0100-$FFFF")
        || summary.contains("unknown-abs")
        || summary.contains("$0000-$FFFF")
}

fn print_routine_range_comparison(
    left: &Artifact,
    right: &Artifact,
    source_path: &Path,
    source_text: &str,
    max_diffs: usize,
) {
    let (Some(left_map), Some(right_map)) = (&left.map, &right.map) else {
        return;
    };

    println!();
    println!("== routines: {} vs {} ==", left.name, right.name);
    let right_by_name = right_map
        .routine_ranges
        .iter()
        .map(|routine| (routine.name.as_str(), routine))
        .collect::<BTreeMap<_, _>>();
    let mut shown = 0usize;
    for left_routine in &left_map.routine_ranges {
        let Some(right_routine) = right_by_name.get(left_routine.name.as_str()).copied() else {
            println!(
                "  {:<24} left ${:04X}-${:04X} missing on right",
                left_routine.name,
                left_routine.start,
                left_routine.end.saturating_sub(1)
            );
            shown += 1;
            if shown >= max_diffs {
                break;
            }
            continue;
        };
        let left_len = routine_range_len(left_routine);
        let right_len = routine_range_len(right_routine);
        let same_bytes = routine_bytes(left, left_routine) == routine_bytes(right, right_routine);
        if left_len == right_len && same_bytes {
            continue;
        }
        println!(
            "  {:<24} len {} -> {} ({:+})  addr ${:04X} -> ${:04X}",
            left_routine.name,
            left_len,
            right_len,
            right_len as isize - left_len as isize,
            left_routine.start,
            right_routine.start
        );
        print_source_range_deltas(
            left,
            left_routine,
            right,
            right_routine,
            source_path,
            source_text,
            max_diffs,
        );
        print_routine_instruction_diff(
            left,
            left_routine,
            right,
            right_routine,
            source_path,
            source_text,
            max_diffs,
        );
        shown += 1;
        if shown >= max_diffs {
            break;
        }
    }
    if shown == 0 {
        println!("range diffs: none");
    }
}

fn print_original_routine_comparison(
    original: &Artifact,
    original_symbols: &OriginalSymbolSet,
    generated: &Artifact,
    source_path: &Path,
    source_text: &str,
    max_diffs: usize,
) {
    let Some(generated_map) = &generated.map else {
        return;
    };
    let original_ranges = original_symbols.routine_ranges(original);
    if original_ranges.is_empty() {
        return;
    }

    println!();
    println!("== routines: {} vs {} ==", original.name, generated.name);
    let generated_by_name = generated_map
        .routine_ranges
        .iter()
        .map(|routine| (routine.name.to_ascii_uppercase(), routine))
        .collect::<BTreeMap<_, _>>();

    let mut shown = 0usize;
    for original_range in &original_ranges {
        let Some(generated_range) = generated_by_name
            .get(&original_range.name.to_ascii_uppercase())
            .copied()
        else {
            println!(
                "  {:<24} original ${:04X}-${:04X} missing on {}",
                original_range.name,
                original_range.start,
                original_range.end.saturating_sub(1),
                generated.name
            );
            shown += 1;
            if shown >= max_diffs {
                break;
            }
            continue;
        };

        let original_len = original_routine_range_len(original_range);
        let generated_len = routine_range_len(generated_range);
        let same_bytes = original_routine_bytes(original, original_range)
            == routine_bytes(generated, generated_range);
        if original_len == generated_len && same_bytes {
            continue;
        }

        println!(
            "  {:<24} len {} -> {} ({:+})  addr ${:04X} -> ${:04X}",
            original_range.name,
            original_len,
            generated_len,
            generated_len as isize - original_len as isize,
            original_range.start,
            generated_range.start
        );
        print_original_routine_instruction_diff(
            original,
            original_symbols,
            original_range,
            generated,
            generated_range,
            source_path,
            source_text,
            max_diffs,
        );
        shown += 1;
        if shown >= max_diffs {
            break;
        }
    }
    if shown == 0 {
        println!("range diffs: none");
    }
}

fn print_original_routine_instruction_diff(
    original: &Artifact,
    original_symbols: &OriginalSymbolSet,
    original_range: &OriginalRoutineRange,
    generated: &Artifact,
    generated_range: &RoutineRange,
    source_path: &Path,
    source_text: &str,
    max_diffs: usize,
) {
    let original_disasm = disassemble_with_origin(
        original_routine_bytes(original, original_range),
        original_range.start,
    );
    let generated_disasm = disassemble_with_origin(
        routine_bytes(generated, generated_range),
        generated_range.start,
    );
    let mut shown = 0usize;
    for index in 0..original_disasm.len().max(generated_disasm.len()) {
        let original_item = original_disasm.get(index);
        let generated_item = generated_disasm.get(index);
        if normalized(original_item) == normalized(generated_item) {
            continue;
        }
        println!(
            "    {} #{index:04}",
            format_original_routine_offset(original_range, original_item, generated_item)
        );
        println!(
            "      original : {}",
            format_original_diff_item(original_item, original_symbols)
        );
        println!("      generated: {}", format_disasm_item(generated_item));
        if let Some(item) = original_item
            && let Some(owner) = original_symbol_owner_for_address(original_symbols, item.address)
        {
            println!("      orig owner: {owner}");
        }
        if let Some(item) = original_item
            && let Some(owner) = original_operand_owner(item, original_symbols)
        {
            println!("      orig operand: {owner}");
        }
        if let Some(item) = generated_item
            && let Some(source) =
                source_range_for_address(generated, item.address, source_path, source_text)
        {
            println!("      source   : {source}");
        }
        if let Some(item) = generated_item
            && let Some(owner) = storage_owner_for_address(generated, item.address)
        {
            println!("      owner    : {}", format_storage_owner(&owner));
        }
        shown += 1;
        if shown >= max_diffs {
            break;
        }
    }
    if shown == 0 {
        println!("    instruction diffs: none");
    }
}

fn format_original_diff_item(
    item: Option<&DisassembledInstruction>,
    symbols: &OriginalSymbolSet,
) -> String {
    let Some(item) = item else {
        return "<missing>".to_string();
    };
    let empty_labels = BTreeMap::new();
    format_labeled_disasm_item(item, Some(symbols), &empty_labels)
}

fn original_symbol_owner_for_address(symbols: &OriginalSymbolSet, address: u16) -> Option<String> {
    let (start, symbol) = symbols.symbol_containing(address)?;
    Some(format_original_operand_symbol(symbol, start, address))
}

fn original_operand_owner(
    item: &DisassembledInstruction,
    symbols: &OriginalSymbolSet,
) -> Option<String> {
    let address = instruction_operand_address(item)?;
    original_symbol_owner_for_address(symbols, address)
}

fn format_original_routine_offset(
    range: &OriginalRoutineRange,
    original_item: Option<&DisassembledInstruction>,
    _generated_item: Option<&DisassembledInstruction>,
) -> String {
    match original_item.map(|item| item.address) {
        Some(address) => format!("{}+${:04X}", range.name, address.wrapping_sub(range.start)),
        None => format!("{}+<missing>", range.name),
    }
}

fn print_routine_instruction_diff(
    left: &Artifact,
    left_range: &RoutineRange,
    right: &Artifact,
    right_range: &RoutineRange,
    source_path: &Path,
    source_text: &str,
    max_diffs: usize,
) {
    let left_disasm = disassemble_with_origin(routine_bytes(left, left_range), left_range.start);
    let right_disasm =
        disassemble_with_origin(routine_bytes(right, right_range), right_range.start);
    let mut shown = 0usize;
    for index in 0..left_disasm.len().max(right_disasm.len()) {
        let left_item = left_disasm.get(index);
        let right_item = right_disasm.get(index);
        if normalized(left_item) == normalized(right_item) {
            continue;
        }
        println!("    #{index:04}");
        println!("      left : {}", format_disasm_item(left_item));
        println!("      right: {}", format_disasm_item(right_item));
        if let Some(item) = left_item
            && let Some(source) =
                source_range_for_address(left, item.address, source_path, source_text)
        {
            println!("      left source : {source}");
        }
        if let Some(item) = left_item
            && let Some(owner) = storage_owner_for_address(left, item.address)
        {
            println!("      left owner  : {}", format_storage_owner(&owner));
        }
        if let Some(item) = right_item
            && let Some(source) =
                source_range_for_address(right, item.address, source_path, source_text)
        {
            println!("      right source: {source}");
        }
        if let Some(item) = right_item
            && let Some(owner) = storage_owner_for_address(right, item.address)
        {
            println!("      right owner : {}", format_storage_owner(&owner));
        }
        shown += 1;
        if shown >= max_diffs {
            break;
        }
    }
    if shown == 0 {
        println!("    instruction diffs: none");
    }
}

fn print_source_range_deltas(
    left: &Artifact,
    left_range: &RoutineRange,
    right: &Artifact,
    right_range: &RoutineRange,
    source_path: &Path,
    source_text: &str,
    max_diffs: usize,
) {
    let deltas = source_range_deltas(left, left_range, right, right_range, max_diffs);
    if deltas.is_empty() {
        return;
    }

    println!("    source-range deltas:");
    for delta in deltas {
        println!(
            "      {}  diffs {}",
            format_source_range_key(&delta.key, source_path, source_text),
            delta.diff_count
        );
        println!(
            "        left : {}",
            format_optional_source_code_range(left, delta.left_range.as_ref())
        );
        println!(
            "        right: {}",
            format_optional_source_code_range(right, delta.right_range.as_ref())
        );
    }
}

fn source_range_deltas(
    left: &Artifact,
    left_range: &RoutineRange,
    right: &Artifact,
    right_range: &RoutineRange,
    max_diffs: usize,
) -> Vec<SourceRangeDelta> {
    let left_disasm = disassemble_with_origin(routine_bytes(left, left_range), left_range.start);
    let right_disasm =
        disassemble_with_origin(routine_bytes(right, right_range), right_range.start);
    let mut deltas: BTreeMap<SourceRangeKey, SourceRangeDelta> = BTreeMap::new();
    for index in 0..left_disasm.len().max(right_disasm.len()) {
        let left_item = left_disasm.get(index);
        let right_item = right_disasm.get(index);
        if normalized(left_item) == normalized(right_item) {
            continue;
        }
        if let Some(item) = left_item {
            insert_source_range_delta(&mut deltas, left, right, item.address, true);
        }
        if let Some(item) = right_item {
            insert_source_range_delta(&mut deltas, right, left, item.address, false);
        }
        if deltas.len() >= max_diffs {
            break;
        }
    }
    deltas.into_values().take(max_diffs).collect()
}

fn insert_source_range_delta(
    deltas: &mut BTreeMap<SourceRangeKey, SourceRangeDelta>,
    primary: &Artifact,
    secondary: &Artifact,
    address: u16,
    primary_is_left: bool,
) {
    let Some(primary_range) = source_range_for_address_ref(primary, address).cloned() else {
        return;
    };
    let key = SourceRangeKey::from(&primary_range);
    let secondary_range = matching_source_range(secondary, &key).cloned();
    let entry = deltas
        .entry(key.clone())
        .or_insert_with(|| SourceRangeDelta {
            key,
            left_range: None,
            right_range: None,
            diff_count: 0,
        });
    entry.diff_count += 1;
    if primary_is_left {
        entry.left_range.get_or_insert(primary_range);
        if let Some(secondary_range) = secondary_range {
            entry.right_range.get_or_insert(secondary_range);
        }
    } else {
        entry.right_range.get_or_insert(primary_range);
        if let Some(secondary_range) = secondary_range {
            entry.left_range.get_or_insert(secondary_range);
        }
    }
}

fn normalized(
    item: Option<&DisassembledInstruction>,
) -> Option<(&'static str, Option<String>, Vec<u8>)> {
    item.map(|item| {
        (
            item.mnemonic,
            item.mode.map(|mode| format!("{mode:?}")),
            item.operands.clone(),
        )
    })
}

fn format_disasm_item(item: Option<&DisassembledInstruction>) -> String {
    match item {
        Some(item) => format!(
            "${:04X} {:<8} {}",
            item.address,
            format_bytes(&item.bytes),
            item.text
        ),
        None => "<missing>".to_string(),
    }
}

fn opcode_counts(artifact: &Artifact) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for item in disassemble_with_origin(&artifact.code_bytes, artifact.code_origin) {
        let opcode = item.bytes.first().copied().unwrap_or(0);
        let key = format!("${opcode:02X} {}", item.mnemonic);
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}

fn byte_diffs(left: &[u8], right: &[u8], max_diffs: usize) -> Vec<String> {
    let mut diffs = Vec::new();
    let max_len = left.len().max(right.len());
    for offset in 0..max_len {
        let left_byte = left.get(offset);
        let right_byte = right.get(offset);
        if left_byte == right_byte {
            continue;
        }
        diffs.push(format!(
            "@{offset:04X}: {} -> {}",
            format_optional_byte(left_byte),
            format_optional_byte(right_byte)
        ));
        if diffs.len() >= max_diffs {
            break;
        }
    }
    diffs
}

fn routine_range_len(range: &RoutineRange) -> usize {
    usize::from(range.end.saturating_sub(range.start))
}

fn original_routine_range_len(range: &OriginalRoutineRange) -> usize {
    usize::from(range.end.saturating_sub(range.start))
}

fn routine_bytes<'a>(artifact: &'a Artifact, range: &RoutineRange) -> &'a [u8] {
    let start = usize::from(range.start.saturating_sub(artifact.code_origin));
    let end = usize::from(range.end.saturating_sub(artifact.code_origin));
    let start = start.min(artifact.code_bytes.len());
    let end = end.min(artifact.code_bytes.len()).max(start);
    &artifact.code_bytes[start..end]
}

fn original_routine_bytes<'a>(artifact: &'a Artifact, range: &OriginalRoutineRange) -> &'a [u8] {
    let start = usize::from(range.start.saturating_sub(artifact.code_origin));
    let end = usize::from(range.end.saturating_sub(artifact.code_origin));
    let start = start.min(artifact.code_bytes.len());
    let end = end.min(artifact.code_bytes.len()).max(start);
    &artifact.code_bytes[start..end]
}

fn source_range_for_address(
    artifact: &Artifact,
    address: u16,
    source_path: &Path,
    source_text: &str,
) -> Option<String> {
    let source = map_query(artifact, Some(source_text))?.source_at(address)?;
    let range = source.range;
    let name = range
        .name
        .as_ref()
        .map(|name| format!(" {name}"))
        .unwrap_or_default();
    let location = source.location?;
    Some(format!(
        "{:?}{name} {}:{}:{} | {} | source {}..{} code ${:04X}-${:04X}",
        range.kind,
        format_compact_path(source_path),
        location.line,
        location.column,
        location.excerpt,
        range.source_span.start,
        range.source_span.end,
        range.start,
        range.end.saturating_sub(1)
    ))
}

fn source_range_for_address_ref(artifact: &Artifact, address: u16) -> Option<&CodegenSourceRange> {
    map_query(artifact, None)?
        .source_at(address)
        .map(|source| source.range)
}

fn matching_source_range<'a>(
    artifact: &'a Artifact,
    key: &SourceRangeKey,
) -> Option<&'a CodegenSourceRange> {
    artifact
        .map
        .as_ref()?
        .source_ranges
        .iter()
        .find(|range| SourceRangeKey::from(*range) == *key)
}

impl From<&CodegenSourceRange> for SourceRangeKey {
    fn from(range: &CodegenSourceRange) -> Self {
        Self {
            kind: SourceRangeKindKey::from(range.kind),
            name: range.name.clone(),
            span_start: range.source_span.start,
            span_end: range.source_span.end,
        }
    }
}

impl From<CodegenSourceRangeKind> for SourceRangeKindKey {
    fn from(kind: CodegenSourceRangeKind) -> Self {
        match kind {
            CodegenSourceRangeKind::Routine => Self::Routine,
            CodegenSourceRangeKind::Statement => Self::Statement,
            CodegenSourceRangeKind::Expression => Self::Expression,
            CodegenSourceRangeKind::Declaration => Self::Declaration,
            CodegenSourceRangeKind::StorageInitializer => Self::StorageInitializer,
            CodegenSourceRangeKind::MachineBlock => Self::MachineBlock,
        }
    }
}

fn format_source_range_key(key: &SourceRangeKey, source_path: &Path, source_text: &str) -> String {
    let location = source_location(source_text, key.span_start);
    let name = key
        .name
        .as_ref()
        .map(|name| format!(" {name}"))
        .unwrap_or_default();
    format!(
        "{:?}{name} {}:{}:{} | {} | source {}..{}",
        key.kind,
        format_compact_path(source_path),
        location.line,
        location.column,
        location.excerpt,
        key.span_start,
        key.span_end
    )
}

fn format_compact_path(path: &Path) -> String {
    if path.is_relative() {
        return path.display().to_string();
    }
    if let Ok(current_dir) = env::current_dir()
        && let Ok(relative) = path.strip_prefix(&current_dir)
    {
        return relative.display().to_string();
    }
    if let Some(name) = path.file_name() {
        return name.to_string_lossy().into_owned();
    }
    path.display().to_string()
}

fn format_optional_source_code_range(
    artifact: &Artifact,
    range: Option<&CodegenSourceRange>,
) -> String {
    let Some(range) = range else {
        return "<missing>".to_string();
    };
    let len = range.end.saturating_sub(range.start);
    let mut formatted = format!(
        "${:04X}-${:04X} len {}",
        range.start,
        range.end.saturating_sub(1),
        len
    );
    if let Some(owner) = storage_owner_for_address(artifact, range.start) {
        formatted.push_str(" owner ");
        formatted.push_str(&format_storage_owner(&owner));
    }
    formatted
}

fn storage_owner_for_address(artifact: &Artifact, address: u16) -> Option<StorageOwner<'_>> {
    map_query(artifact, None)?.storage_owner(address)
}

fn format_skipped_range(artifact: &Artifact, range: &SkippedRange) -> String {
    let start = range.start;
    let end = range.start.wrapping_add(range.len.saturating_sub(1));
    let mut formatted = format!("${start:04X}-${end:04X} len {}", range.len);
    if let Some(owner) = storage_owner_for_address(artifact, range.start) {
        formatted.push_str(" owner ");
        formatted.push_str(&format_storage_owner(&owner));
    }
    formatted
}

fn format_storage_owner(owner: &StorageOwner<'_>) -> String {
    format!("{}+{}", format_storage_symbol(owner.symbol), owner.offset)
}

fn format_storage_symbol(symbol: &CodegenStorageSymbol) -> String {
    let scope = match &symbol.scope {
        CodegenSymbolScope::Global => "global".to_string(),
        CodegenSymbolScope::Routine(name) => format!("routine {name}"),
    };
    let end = symbol.address.wrapping_add(symbol.size.saturating_sub(1));
    format!(
        "{} {:?} {} ${:04X}-${:04X}",
        scope, symbol.kind, symbol.name, symbol.address, end
    )
}

fn map_query<'a>(artifact: &'a Artifact, source_text: Option<&'a str>) -> Option<MapQuery<'a>> {
    let map = artifact.map.as_ref()?;
    Some(match source_text {
        Some(source_text) => MapQuery::with_source(map, source_text),
        None => MapQuery::new(map),
    })
}

fn load_original_symbols(
    symbols_path: Option<&PathBuf>,
    snapshots_path: Option<&PathBuf>,
) -> Option<OriginalSymbolSet> {
    if symbols_path.is_none() && snapshots_path.is_none() {
        return None;
    }

    let mut set = OriginalSymbolSet {
        labels: BTreeMap::new(),
    };
    if let Some(path) = symbols_path {
        let text = fs::read_to_string(path).unwrap_or_else(|err| {
            eprintln!("read {}: {err}", path.display());
            process::exit(1);
        });
        set.extend(parse_original_symbols_json(
            &text,
            OriginalSymbolSource::Final,
        ));
    }
    if let Some(path) = snapshots_path {
        let text = fs::read_to_string(path).unwrap_or_else(|err| {
            eprintln!("read {}: {err}", path.display());
            process::exit(1);
        });
        set.extend(parse_original_symbols_json(
            &text,
            OriginalSymbolSource::Snapshot,
        ));
    }
    Some(set)
}

impl OriginalSymbolSet {
    fn extend(&mut self, symbols: Vec<(u16, OriginalSymbol)>) {
        for (address, symbol) in symbols {
            let labels = self.labels.entry(address).or_default();
            if !labels.iter().any(|existing| {
                existing.name == symbol.name
                    && existing.class == symbol.class
                    && existing.size == symbol.size
            }) {
                labels.push(symbol);
            }
        }
        for labels in self.labels.values_mut() {
            labels.sort_by(|left, right| {
                original_symbol_source_rank(left.source)
                    .cmp(&original_symbol_source_rank(right.source))
                    .then_with(|| left.name.cmp(&right.name))
                    .then_with(|| left.class.cmp(&right.class))
            });
        }
    }

    fn labels_at(&self, address: u16) -> Option<&[OriginalSymbol]> {
        self.labels.get(&address).map(Vec::as_slice)
    }

    fn symbol_containing(&self, address: u16) -> Option<(u16, &OriginalSymbol)> {
        self.labels.iter().find_map(|(start, labels)| {
            labels.iter().find_map(|symbol| {
                let size = symbol.size.max(1);
                let end = start.wrapping_add(size);
                if address >= *start && address < end {
                    Some((*start, symbol))
                } else {
                    None
                }
            })
        })
    }

    fn routine_ranges(&self, artifact: &Artifact) -> Vec<OriginalRoutineRange> {
        let mut routines = self
            .labels
            .iter()
            .flat_map(|(address, labels)| {
                labels.iter().filter_map(|symbol| {
                    if symbol.is_routine() {
                        Some(OriginalRoutineRange {
                            name: symbol.name.clone(),
                            start: *address,
                            end: artifact
                                .code_origin
                                .wrapping_add(artifact.code_bytes.len() as u16),
                        })
                    } else {
                        None
                    }
                })
            })
            .collect::<Vec<_>>();
        routines.sort_by(|left, right| {
            left.start
                .cmp(&right.start)
                .then_with(|| left.name.cmp(&right.name))
        });
        let proc_starts = routines.iter().map(|range| range.start).collect::<Vec<_>>();

        let code_start = artifact.code_origin;
        let code_end = artifact
            .code_origin
            .wrapping_add(artifact.code_bytes.len() as u16);
        let data_addresses = self
            .labels
            .iter()
            .filter_map(|(address, labels)| {
                if labels.iter().any(OriginalSymbol::is_routine) {
                    None
                } else {
                    Some(*address)
                }
            })
            .collect::<Vec<_>>();

        for index in 0..routines.len() {
            let proc_start = proc_starts[index];
            let previous_proc_start = index
                .checked_sub(1)
                .and_then(|previous| proc_starts.get(previous))
                .copied()
                .unwrap_or(code_start);
            let storage_start = data_addresses
                .iter()
                .copied()
                .filter(|address| *address >= previous_proc_start && *address < proc_start)
                .min()
                .unwrap_or(proc_start);
            routines[index].start = storage_start.clamp(code_start, code_end);
        }

        for index in 0..routines.len() {
            let next_start = routines
                .get(index + 1)
                .map(|range| range.start)
                .unwrap_or(code_end);
            routines[index].end = next_start.clamp(code_start, code_end);
        }
        routines
            .into_iter()
            .filter(|range| range.end > range.start)
            .collect()
    }
}

impl OriginalSymbol {
    fn is_routine(&self) -> bool {
        let upper = self.class.to_ascii_uppercase();
        upper == "PROC" || upper.contains(" FUNC")
    }
}

fn original_symbol_source_rank(source: OriginalSymbolSource) -> u8 {
    match source {
        OriginalSymbolSource::Final => 0,
        OriginalSymbolSource::Snapshot => 1,
    }
}

fn parse_original_symbols_json(
    text: &str,
    source: OriginalSymbolSource,
) -> Vec<(u16, OriginalSymbol)> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative_start) = text[cursor..].find('{') {
        let start = cursor + relative_start;
        let Some(relative_end) = text[start..].find('}') else {
            break;
        };
        let end = start + relative_end + 1;
        let object = &text[start..end];
        if let (Some(name), Some(class), Some(address)) = (
            json_string_field(object, "name"),
            json_string_field(object, "class"),
            json_string_field(object, "address").and_then(|value| parse_json_address(&value)),
        ) {
            let size = original_symbol_size(&class);
            out.push((
                address,
                OriginalSymbol {
                    name,
                    class,
                    size,
                    source,
                },
            ));
        }
        cursor = end;
    }
    out
}

fn original_symbol_size(class: &str) -> u16 {
    let upper = class.to_ascii_uppercase();
    if upper.contains(" PROC") || upper == "PROC" || upper.contains(" FUNC") {
        1
    } else if upper.contains("CARD")
        || upper.contains("INT")
        || upper.contains("POINTER")
        || upper.contains("STRING")
    {
        2
    } else {
        1
    }
}

fn json_string_field(object: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":");
    let start = object.find(&marker)? + marker.len();
    let rest = object[start..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut value = String::new();
    let mut escaped = false;
    for ch in rest.chars() {
        if escaped {
            value.push(match ch {
                '"' => '"',
                '\\' => '\\',
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                other => other,
            });
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Some(value),
            ch => value.push(ch),
        }
    }
    None
}

fn parse_json_address(value: &str) -> Option<u16> {
    let hex = value.strip_prefix('$')?;
    u16::from_str_radix(hex, 16).ok()
}

fn parse_address(value: &str) -> Result<u16, String> {
    let trimmed = value.trim();
    let (digits, radix) = if let Some(hex) = trimmed.strip_prefix('$') {
        (hex, 16)
    } else if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        (hex, 16)
    } else {
        (trimmed, 10)
    };
    u16::from_str_radix(digits, radix).map_err(|_| format!("invalid address `{value}`"))
}

fn print_original_disassembly(artifact: &Artifact, symbols: Option<&OriginalSymbolSet>) {
    println!();
    println!("== original disassembly: {} ==", artifact.name);
    for segment in artifact
        .segments
        .iter()
        .filter(|segment| !is_vector_segment(segment))
    {
        println!(
            "segment {:02} ${:04X}-${:04X} len {}",
            segment.index,
            segment.start,
            segment.end,
            segment.data.len()
        );
        let items = disassemble_with_origin(&segment.data, segment.start);
        let generated_labels = generated_code_labels(&items);
        for item in items {
            if let Some(symbols) = symbols
                && let Some(labels) = symbols.labels_at(item.address)
            {
                for label in labels {
                    println!(
                        "{}: ; {}",
                        sanitize_label(&label.name),
                        format_original_symbol(label)
                    );
                }
            } else if let Some(label) = generated_labels.get(&item.address) {
                println!("{label}:");
            }
            println!(
                "  {}",
                format_labeled_disasm_item(&item, symbols, &generated_labels)
            );
        }
    }
}

fn generated_code_labels(items: &[DisassembledInstruction]) -> BTreeMap<u16, String> {
    let mut labels = BTreeMap::new();
    for item in items {
        if let Some(target) = instruction_target(item) {
            labels
                .entry(target)
                .or_insert_with(|| format!("L{target:04X}"));
        }
    }
    labels
}

fn instruction_target(item: &DisassembledInstruction) -> Option<u16> {
    match item.mode? {
        AddressingMode::Relative => {
            let offset = *item.operands.first()? as i8;
            Some(
                item.address
                    .wrapping_add(2)
                    .wrapping_add_signed(i16::from(offset)),
            )
        }
        AddressingMode::Absolute | AddressingMode::AbsoluteX => {
            if matches!(item.mnemonic, "JMP" | "JSR") {
                Some(le_u16_from_slice(&item.operands)?)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn instruction_operand_address(item: &DisassembledInstruction) -> Option<u16> {
    match item.mode? {
        AddressingMode::ZeroPage
        | AddressingMode::ZeroPageX
        | AddressingMode::IndexedIndirectX
        | AddressingMode::IndirectIndexedY => item.operands.first().copied().map(u16::from),
        AddressingMode::Absolute | AddressingMode::AbsoluteX => le_u16_from_slice(&item.operands),
        AddressingMode::Relative => instruction_target(item),
        _ => None,
    }
}

fn le_u16_from_slice(bytes: &[u8]) -> Option<u16> {
    Some(u16::from(*bytes.first()?) | (u16::from(*bytes.get(1)?) << 8))
}

fn format_labeled_disasm_item(
    item: &DisassembledInstruction,
    symbols: Option<&OriginalSymbolSet>,
    generated_labels: &BTreeMap<u16, String>,
) -> String {
    let mut text = item.text.clone();
    if let Some(address) = instruction_operand_address(item) {
        if let Some((start, symbol)) =
            symbols.and_then(|symbols| symbols.symbol_containing(address))
        {
            text.push_str(" ; ");
            text.push_str(&format_original_operand_symbol(symbol, start, address));
        } else if let Some(label) = generated_labels.get(&address) {
            text.push_str(" ; ");
            text.push_str(label);
        }
    }
    format!(
        "${:04X} {:<8} {}",
        item.address,
        format_bytes(&item.bytes),
        text
    )
}

fn format_original_operand_symbol(symbol: &OriginalSymbol, start: u16, address: u16) -> String {
    let offset = address.wrapping_sub(start);
    if offset == 0 {
        format!("{} ${address:04X}", symbol.name)
    } else {
        format!("{}+{} ${address:04X}", symbol.name, offset)
    }
}

fn format_original_symbol(symbol: &OriginalSymbol) -> String {
    let source = match symbol.source {
        OriginalSymbolSource::Final => "final",
        OriginalSymbolSource::Snapshot => "snapshot",
    };
    format!("{} {}", source, symbol.class)
}

fn sanitize_label(name: &str) -> String {
    let mut label = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            label.push(ch);
        } else {
            label.push('_');
        }
    }
    if label.is_empty() {
        "unnamed".to_string()
    } else if label
        .as_bytes()
        .first()
        .is_some_and(|first| first.is_ascii_digit())
    {
        format!("_{label}")
    } else {
        label
    }
}

fn format_optional_byte(byte: Option<&u8>) -> String {
    byte.map(|byte| format!("${byte:02X}"))
        .unwrap_or_else(|| "<eof>".to_string())
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
            return Err(format!("truncated segment header at file offset {offset}"));
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
                "segment {index} ${start:04X}-${end:04X} is truncated"
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

fn is_vector_segment(segment: &Segment) -> bool {
    segment.start == RUNAD && segment.end == RUNAD.wrapping_add(1)
}

fn vector_value(segment: &Segment, address: u16) -> Option<u16> {
    if segment.start > address || segment.end < address.wrapping_add(1) {
        return None;
    }
    let offset = usize::from(address.wrapping_sub(segment.start));
    Some(u16::from(segment.data[offset]) | (u16::from(segment.data[offset + 1]) << 8))
}

fn read_word(bytes: &[u8], offset: usize) -> u16 {
    u16::from(bytes[offset]) | (u16::from(bytes[offset + 1]) << 8)
}

fn format_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn print_diagnostics(diagnostics: Vec<Diagnostic>) {
    for diagnostic in diagnostics {
        eprintln!(
            "{}..{}: {}",
            diagnostic.span.start, diagnostic.span.end, diagnostic.message
        );
    }
}
