use std::env;
use std::fmt::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process;

use actionc::codegen::{
    CODE_ORIGIN, CodegenOutput, CodegenProfile, format_hex, generate_profile_with_origin,
    generate_semir_profile_with_origin,
};
use actionc::diagnostic::Diagnostic;
use actionc::includes::load_program_with_expanded_source;
use actionc::semantic::{analyze, ir};

#[derive(Debug, Clone)]
struct Config {
    roots: Vec<PathBuf>,
    profile: CodegenProfile,
    origin: u16,
    verbose: bool,
    report: ReportFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReportFormat {
    Text,
    Markdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Match,
    Mismatch,
    AstFailed,
    SemIrFailed,
    LoadFailed,
}

#[derive(Debug)]
struct SweepResult {
    path: PathBuf,
    outcome: Outcome,
    detail: String,
}

#[derive(Debug, Default)]
struct SweepCounts {
    matched: usize,
    mismatched: usize,
    ast_failed: usize,
    semir_failed: usize,
    load_failed: usize,
}

fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let config = parse_args();
    let mut files = Vec::new();
    for root in &config.roots {
        collect_action_sources(root, &mut files);
    }
    files.sort();
    files.dedup();

    if files.is_empty() {
        eprintln!("actionc-semir-sweep: no Action sources found");
        process::exit(2);
    }

    let mut results = Vec::new();
    for file in files {
        let result = sweep_file(&config, &file);
        if config.report == ReportFormat::Text {
            print_result(&result, config.verbose);
        }
        results.push(result);
    }

    let counts = count_results(&results);
    match config.report {
        ReportFormat::Text => println!(
            "SemIR bridge sweep summary: match={} mismatch={} ast_failed={} semir_failed={} load_failed={}",
            counts.matched,
            counts.mismatched,
            counts.ast_failed,
            counts.semir_failed,
            counts.load_failed
        ),
        ReportFormat::Markdown => print_markdown_report(&config, &results, &counts),
    }

    if counts.mismatched + counts.semir_failed > 0 {
        process::exit(1);
    }
}

fn parse_args() -> Config {
    let mut args = env::args().skip(1);
    let mut roots = Vec::new();
    let mut profile = CodegenProfile::Compat;
    let mut origin = CODE_ORIGIN;
    let mut verbose = false;
    let mut report = ReportFormat::Text;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--profile" => {
                let Some(value) = args.next() else {
                    usage_and_exit("--profile requires legacy or modern");
                };
                profile = parse_profile(&value).unwrap_or_else(|| {
                    usage_and_exit("--profile requires legacy or modern");
                });
            }
            value if value.starts_with("--profile=") => {
                profile = parse_profile(&value["--profile=".len()..]).unwrap_or_else(|| {
                    usage_and_exit("--profile requires legacy or modern");
                });
            }
            "--origin" => {
                let Some(value) = args.next() else {
                    usage_and_exit("--origin requires an address");
                };
                origin = parse_origin(&value).unwrap_or_else(|| {
                    usage_and_exit("--origin requires a decimal or $/0x hex address");
                });
            }
            value if value.starts_with("--origin=") => {
                origin = parse_origin(&value["--origin=".len()..]).unwrap_or_else(|| {
                    usage_and_exit("--origin requires a decimal or $/0x hex address");
                });
            }
            "-v" | "--verbose" => verbose = true,
            "--markdown" => report = ReportFormat::Markdown,
            "--report" => {
                let Some(value) = args.next() else {
                    usage_and_exit("--report requires text or markdown");
                };
                report = parse_report_format(&value).unwrap_or_else(|| {
                    usage_and_exit("--report requires text or markdown");
                });
            }
            value if value.starts_with("--report=") => {
                report = parse_report_format(&value["--report=".len()..]).unwrap_or_else(|| {
                    usage_and_exit("--report requires text or markdown");
                });
            }
            "-h" | "--help" => {
                print_usage();
                process::exit(0);
            }
            value if value.starts_with('-') => {
                usage_and_exit(&format!("unexpected argument: {value}"));
            }
            path => roots.push(PathBuf::from(path)),
        }
    }

    if roots.is_empty() {
        roots.push(PathBuf::from("surveys/probes/original-compiler"));
        roots.push(PathBuf::from("fixtures/stress"));
        roots.push(PathBuf::from("corpora/toolkit/original/extracted"));
    }

    Config {
        roots,
        profile,
        origin,
        verbose,
        report,
    }
}

fn sweep_file(config: &Config, path: &Path) -> SweepResult {
    let loaded = match load_program_with_expanded_source(path) {
        Ok(loaded) => loaded,
        Err(diagnostics) => {
            return SweepResult {
                path: path.to_path_buf(),
                outcome: Outcome::LoadFailed,
                detail: diagnostic_summary(&diagnostics),
            };
        }
    };
    let model = match analyze(&loaded.program) {
        Ok(model) => model,
        Err(diagnostics) => {
            return SweepResult {
                path: path.to_path_buf(),
                outcome: Outcome::LoadFailed,
                detail: diagnostic_summary(&diagnostics),
            };
        }
    };

    let ast_output = match catch_codegen(|| {
        generate_profile_with_origin(&loaded.program, config.origin, config.profile)
    }) {
        Ok(output) => output,
        Err(detail) => {
            return SweepResult {
                path: path.to_path_buf(),
                outcome: Outcome::AstFailed,
                detail,
            };
        }
    };

    let semir = ir::lower_program(&loaded.program, &model);
    let semir_output = match catch_codegen(|| {
        generate_semir_profile_with_origin(&semir, config.origin, config.profile)
    }) {
        Ok(output) => output,
        Err(detail) => {
            return SweepResult {
                path: path.to_path_buf(),
                outcome: Outcome::SemIrFailed,
                detail,
            };
        }
    };

    if equivalent_output(&ast_output, &semir_output) {
        SweepResult {
            path: path.to_path_buf(),
            outcome: Outcome::Match,
            detail: format!("{} bytes", ast_output.bytes.len()),
        }
    } else {
        SweepResult {
            path: path.to_path_buf(),
            outcome: Outcome::Mismatch,
            detail: mismatch_summary(&ast_output, &semir_output),
        }
    }
}

fn catch_codegen(
    generate: impl FnOnce() -> Result<CodegenOutput, Vec<Diagnostic>>,
) -> Result<CodegenOutput, String> {
    match catch_unwind(AssertUnwindSafe(generate)) {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(diagnostics)) => Err(diagnostic_summary(&diagnostics)),
        Err(payload) => Err(format!("panic: {}", panic_payload_summary(payload))),
    }
}

fn panic_payload_summary(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

fn equivalent_output(left: &CodegenOutput, right: &CodegenOutput) -> bool {
    left.origin == right.origin
        && left.run_address == right.run_address
        && left.bytes == right.bytes
        && left.skipped_ranges == right.skipped_ranges
        && left.routine_addresses == right.routine_addresses
}

fn mismatch_summary(ast: &CodegenOutput, semir: &CodegenOutput) -> String {
    let mut detail = String::new();
    let _ = write!(
        detail,
        "ast={} bytes semir={} bytes origin ${:04X}/${:04X} run ${:04X}/${:04X}",
        ast.bytes.len(),
        semir.bytes.len(),
        ast.origin,
        semir.origin,
        ast.run_address,
        semir.run_address
    );

    if ast.bytes != semir.bytes {
        if let Some(index) = ast
            .bytes
            .iter()
            .zip(semir.bytes.iter())
            .position(|(left, right)| left != right)
        {
            let _ = write!(
                detail,
                " first_byte_diff={} ast={:02X} semir={:02X}",
                index, ast.bytes[index], semir.bytes[index]
            );
        } else {
            let _ = write!(
                detail,
                " common_prefix={} ast_tail={} semir_tail={}",
                ast.bytes.len().min(semir.bytes.len()),
                hex_tail(&ast.bytes, semir.bytes.len()),
                hex_tail(&semir.bytes, ast.bytes.len())
            );
        }
    }

    detail
}

fn hex_tail(bytes: &[u8], other_len: usize) -> String {
    if bytes.len() <= other_len {
        return "-".to_string();
    }
    format_hex(&bytes[other_len..bytes.len().min(other_len + 8)])
}

fn print_result(result: &SweepResult, verbose: bool) {
    let label = outcome_label(result.outcome);
    if result.outcome == Outcome::Match && !verbose {
        println!("{label:<8} {}", result.path.display());
    } else {
        println!("{label:<8} {:<56} {}", result.path.display(), result.detail);
    }
}

fn count_results(results: &[SweepResult]) -> SweepCounts {
    let mut counts = SweepCounts::default();
    for result in results {
        match result.outcome {
            Outcome::Match => counts.matched += 1,
            Outcome::Mismatch => counts.mismatched += 1,
            Outcome::AstFailed => counts.ast_failed += 1,
            Outcome::SemIrFailed => counts.semir_failed += 1,
            Outcome::LoadFailed => counts.load_failed += 1,
        }
    }
    counts
}

fn print_markdown_report(config: &Config, results: &[SweepResult], counts: &SweepCounts) {
    println!("# SemIR Bridge Sweep Report");
    println!();
    println!("- Profile: `{}`", profile_label(config.profile));
    println!("- Origin: `${:04X}`", config.origin);
    println!("- Validation policy: `exact`");
    println!("- Files: {}", results.len());
    println!();
    println!("## Summary");
    println!();
    println!("| Outcome | Count |");
    println!("| --- | ---: |");
    println!("| `MATCH` | {} |", counts.matched);
    println!("| `MISMATCH` | {} |", counts.mismatched);
    println!("| `ASTFAIL` | {} |", counts.ast_failed);
    println!("| `SEMFAIL` | {} |", counts.semir_failed);
    println!("| `LOADFAIL` | {} |", counts.load_failed);
    println!();
    println!("## Results");
    println!();
    println!("| Outcome | Source | Detail |");
    println!("| --- | --- | --- |");
    for result in results {
        println!(
            "| `{}` | `{}` | {} |",
            outcome_label(result.outcome),
            escape_markdown_cell(&result.path.display().to_string()),
            escape_markdown_cell(&result.detail)
        );
    }
}

fn outcome_label(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Match => "MATCH",
        Outcome::Mismatch => "MISMATCH",
        Outcome::AstFailed => "ASTFAIL",
        Outcome::SemIrFailed => "SEMFAIL",
        Outcome::LoadFailed => "LOADFAIL",
    }
}

fn profile_label(profile: CodegenProfile) -> &'static str {
    match profile {
        CodegenProfile::Compat => "legacy",
        CodegenProfile::Modern => "modern",
    }
}

fn escape_markdown_cell(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('\n', "<br>")
}

fn collect_action_sources(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        if is_action_source(path) {
            out.push(path.to_path_buf());
        }
        return;
    }

    let Ok(entries) = std::fs::read_dir(path) else {
        eprintln!("warning: cannot read {}", path.display());
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if is_generated_output_dir(&path) {
                continue;
            }
            collect_action_sources(&path, out);
        } else if is_action_source(&path) {
            out.push(path);
        }
    }
}

fn is_generated_output_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, "outputs" | "target" | ".git"))
}

fn is_action_source(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("act"))
}

fn diagnostic_summary(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .take(3)
        .map(|diagnostic| {
            format!(
                "{}..{}: {}",
                diagnostic.span.start, diagnostic.span.end, diagnostic.message
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn parse_profile(value: &str) -> Option<CodegenProfile> {
    match value {
        "legacy" | "compat" => Some(CodegenProfile::Compat),
        "modern" => Some(CodegenProfile::Modern),
        _ => None,
    }
}

fn parse_report_format(value: &str) -> Option<ReportFormat> {
    match value {
        "text" => Some(ReportFormat::Text),
        "markdown" | "md" => Some(ReportFormat::Markdown),
        _ => None,
    }
}

fn parse_origin(value: &str) -> Option<u16> {
    if let Some(hex) = value.strip_prefix('$') {
        u16::from_str_radix(hex, 16).ok()
    } else if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u16::from_str_radix(hex, 16).ok()
    } else {
        value.parse::<u16>().ok()
    }
}

fn usage_and_exit(message: &str) -> ! {
    eprintln!("actionc-semir-sweep: {message}");
    print_usage();
    process::exit(2);
}

fn print_usage() {
    eprintln!(
        "usage: actionc-semir-sweep [--profile legacy|modern] [--origin <addr>] [--report text|markdown] [--verbose] [path ...]"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_report_format_aliases() {
        assert_eq!(parse_report_format("text"), Some(ReportFormat::Text));
        assert_eq!(
            parse_report_format("markdown"),
            Some(ReportFormat::Markdown)
        );
        assert_eq!(parse_report_format("md"), Some(ReportFormat::Markdown));
        assert_eq!(parse_report_format("other"), None);
    }
}
