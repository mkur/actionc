use std::env;
use std::fmt::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process;

use actionc::codegen::{
    CODE_ORIGIN, CodegenOutput, CodegenProfile, format_hex, generate_profile_with_origin,
    generate_semir_native_profile_with_origin, generate_semir_profile_with_origin,
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
    candidate: SemIrCandidate,
    dashboard: bool,
    validation_policy: ValidationPolicy,
    report: ReportFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SemIrCandidate {
    Bridge,
    Native,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValidationPolicy {
    Exact,
    Coverage,
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReportFormat {
    Text,
    Markdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputExpectation {
    Exact,
    Coverage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Match,
    Delta,
    Mismatch,
    Unsupported,
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
    deltas: usize,
    mismatched: usize,
    unsupported: usize,
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
        ReportFormat::Text => {
            println!(
                "SemIR {} sweep summary: match={} delta={} mismatch={} unsupported={} ast_failed={} semir_failed={} load_failed={}",
                config.candidate.label(),
                counts.matched,
                counts.deltas,
                counts.mismatched,
                counts.unsupported,
                counts.ast_failed,
                counts.semir_failed,
                counts.load_failed
            );
            if config.dashboard {
                print_dashboard(&results);
            }
        }
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
    let mut candidate = SemIrCandidate::Bridge;
    let mut dashboard = false;
    let mut validation_policy = ValidationPolicy::Exact;
    let mut report = ReportFormat::Text;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--candidate" | "--semir-candidate" => {
                let Some(value) = args.next() else {
                    usage_and_exit("--candidate requires bridge or native");
                };
                candidate = parse_candidate(&value).unwrap_or_else(|| {
                    usage_and_exit("--candidate requires bridge or native");
                });
            }
            value if value.starts_with("--candidate=") => {
                candidate = parse_candidate(&value["--candidate=".len()..]).unwrap_or_else(|| {
                    usage_and_exit("--candidate requires bridge or native");
                });
            }
            value if value.starts_with("--semir-candidate=") => {
                candidate =
                    parse_candidate(&value["--semir-candidate=".len()..]).unwrap_or_else(|| {
                        usage_and_exit("--candidate requires bridge or native");
                    });
            }
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
            "--dashboard" | "--support-dashboard" => dashboard = true,
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
            "--validation-policy" | "--validation" => {
                let Some(value) = args.next() else {
                    usage_and_exit("--validation-policy requires exact, coverage, or mixed");
                };
                validation_policy = parse_validation_policy(&value).unwrap_or_else(|| {
                    usage_and_exit("--validation-policy requires exact, coverage, or mixed");
                });
            }
            value if value.starts_with("--validation-policy=") => {
                validation_policy = parse_validation_policy(&value["--validation-policy=".len()..])
                    .unwrap_or_else(|| {
                        usage_and_exit("--validation-policy requires exact, coverage, or mixed");
                    });
            }
            value if value.starts_with("--validation=") => {
                validation_policy = parse_validation_policy(&value["--validation=".len()..])
                    .unwrap_or_else(|| {
                        usage_and_exit("--validation-policy requires exact, coverage, or mixed");
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
        candidate,
        dashboard,
        validation_policy,
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
    let semir_output = match catch_codegen(|| candidate_codegen(config, &semir)) {
        Ok(output) => output,
        Err(detail) => {
            if config.dashboard
                && matches!(config.candidate, SemIrCandidate::Native)
                && let Some(reason) = native_unsupported_reason(&detail)
            {
                return SweepResult {
                    path: path.to_path_buf(),
                    outcome: Outcome::Unsupported,
                    detail: reason,
                };
            }
            return SweepResult {
                path: path.to_path_buf(),
                outcome: Outcome::SemIrFailed,
                detail,
            };
        }
    };

    let expectation = output_expectation(config, path);
    if equivalent_output(&ast_output, &semir_output) {
        SweepResult {
            path: path.to_path_buf(),
            outcome: Outcome::Match,
            detail: format!("{} bytes", ast_output.bytes.len()),
        }
    } else if expectation == OutputExpectation::Coverage {
        SweepResult {
            path: path.to_path_buf(),
            outcome: Outcome::Delta,
            detail: mismatch_summary(&ast_output, &semir_output, config.candidate.label()),
        }
    } else {
        SweepResult {
            path: path.to_path_buf(),
            outcome: Outcome::Mismatch,
            detail: mismatch_summary(&ast_output, &semir_output, config.candidate.label()),
        }
    }
}

fn output_expectation(config: &Config, path: &Path) -> OutputExpectation {
    match config.validation_policy {
        ValidationPolicy::Exact => OutputExpectation::Exact,
        ValidationPolicy::Coverage => OutputExpectation::Coverage,
        ValidationPolicy::Mixed => {
            if is_semir_exact_fixture(path) {
                OutputExpectation::Exact
            } else {
                OutputExpectation::Coverage
            }
        }
    }
}

fn is_semir_exact_fixture(path: &Path) -> bool {
    let mut previous = None;
    for component in path.components() {
        let Some(name) = component.as_os_str().to_str() else {
            previous = None;
            continue;
        };
        if previous == Some("fixtures") && name == "semir" {
            return true;
        }
        previous = Some(name);
    }
    false
}

fn candidate_codegen(
    config: &Config,
    semir: &ir::SemProgram,
) -> Result<CodegenOutput, Vec<Diagnostic>> {
    match config.candidate {
        SemIrCandidate::Bridge => {
            generate_semir_profile_with_origin(semir, config.origin, config.profile)
        }
        SemIrCandidate::Native => {
            generate_semir_native_profile_with_origin(semir, config.origin, config.profile)
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

fn mismatch_summary(ast: &CodegenOutput, semir: &CodegenOutput, candidate: &str) -> String {
    let mut detail = String::new();
    let _ = write!(
        detail,
        "ast={} bytes {candidate}={} bytes origin ${:04X}/${:04X} run ${:04X}/${:04X}",
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
            let ast_byte = ast.bytes[index];
            let semir_byte = semir.bytes[index];
            let _ = write!(
                detail,
                " first_byte_diff={} ast={:02X} semir={:02X}",
                index, ast_byte, semir_byte
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
    let label = match result.outcome {
        Outcome::Match => "MATCH",
        Outcome::Delta => "DELTA",
        Outcome::Mismatch => "MISMATCH",
        Outcome::Unsupported => "UNSUPPORTED",
        Outcome::AstFailed => "ASTFAIL",
        Outcome::SemIrFailed => "SEMFAIL",
        Outcome::LoadFailed => "LOADFAIL",
    };
    if result.outcome == Outcome::Match && !verbose {
        println!("{label:<8} {}", result.path.display());
    } else {
        println!("{label:<8} {:<56} {}", result.path.display(), result.detail);
    }
}

fn print_dashboard(results: &[SweepResult]) {
    let total = results.len();
    let supported = results
        .iter()
        .filter(|result| {
            matches!(
                result.outcome,
                Outcome::Match | Outcome::Delta | Outcome::Mismatch
            )
        })
        .count();
    let matched = results
        .iter()
        .filter(|result| matches!(result.outcome, Outcome::Match))
        .count();
    let deltas = results
        .iter()
        .filter(|result| matches!(result.outcome, Outcome::Delta))
        .count();
    let unsupported = results
        .iter()
        .filter(|result| matches!(result.outcome, Outcome::Unsupported))
        .count();
    println!(
        "SemIR support dashboard: total={total} supported={supported} matched={matched} deltas={deltas} unsupported={unsupported}"
    );

    let mut unsupported_by_reason = std::collections::BTreeMap::<String, Vec<&Path>>::new();
    for result in results {
        if result.outcome == Outcome::Unsupported {
            unsupported_by_reason
                .entry(result.detail.clone())
                .or_default()
                .push(result.path.as_path());
        }
    }
    if unsupported_by_reason.is_empty() {
        return;
    }

    println!("Unsupported blockers:");
    for (reason, paths) in unsupported_by_reason {
        println!("  {:>3} {}", paths.len(), reason);
        for path in paths.iter().take(5) {
            println!("      {}", path.display());
        }
        if paths.len() > 5 {
            println!("      ... {} more", paths.len() - 5);
        }
    }
}

fn count_results(results: &[SweepResult]) -> SweepCounts {
    let mut counts = SweepCounts::default();
    for result in results {
        match result.outcome {
            Outcome::Match => counts.matched += 1,
            Outcome::Delta => counts.deltas += 1,
            Outcome::Mismatch => counts.mismatched += 1,
            Outcome::Unsupported => counts.unsupported += 1,
            Outcome::AstFailed => counts.ast_failed += 1,
            Outcome::SemIrFailed => counts.semir_failed += 1,
            Outcome::LoadFailed => counts.load_failed += 1,
        }
    }
    counts
}

fn print_markdown_report(config: &Config, results: &[SweepResult], counts: &SweepCounts) {
    println!("# SemIR {} Sweep Report", config.candidate.label());
    println!();
    println!("- Profile: `{}`", profile_label(config.profile));
    println!("- Origin: `${:04X}`", config.origin);
    println!(
        "- Validation policy: `{}`",
        validation_policy_label(config.validation_policy)
    );
    println!("- Files: {}", results.len());
    println!();
    println!("## Summary");
    println!();
    println!("| Outcome | Count |");
    println!("| --- | ---: |");
    println!("| `MATCH` | {} |", counts.matched);
    println!("| `DELTA` | {} |", counts.deltas);
    println!("| `MISMATCH` | {} |", counts.mismatched);
    println!("| `UNSUPPORTED` | {} |", counts.unsupported);
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

    if config.dashboard {
        print_markdown_dashboard(results);
    }
}

fn print_markdown_dashboard(results: &[SweepResult]) {
    let total = results.len();
    let supported = results
        .iter()
        .filter(|result| {
            matches!(
                result.outcome,
                Outcome::Match | Outcome::Delta | Outcome::Mismatch
            )
        })
        .count();
    let matched = results
        .iter()
        .filter(|result| matches!(result.outcome, Outcome::Match))
        .count();
    let deltas = results
        .iter()
        .filter(|result| matches!(result.outcome, Outcome::Delta))
        .count();
    let unsupported = results
        .iter()
        .filter(|result| matches!(result.outcome, Outcome::Unsupported))
        .count();

    println!();
    println!("## Support Dashboard");
    println!();
    println!("- Total: {total}");
    println!("- Supported: {supported}");
    println!("- Matched: {matched}");
    println!("- Deltas: {deltas}");
    println!("- Unsupported: {unsupported}");

    let mut unsupported_by_reason = std::collections::BTreeMap::<String, Vec<&Path>>::new();
    for result in results {
        if result.outcome == Outcome::Unsupported {
            unsupported_by_reason
                .entry(result.detail.clone())
                .or_default()
                .push(result.path.as_path());
        }
    }
    if unsupported_by_reason.is_empty() {
        return;
    }

    println!();
    println!("| Unsupported reason | Count | Examples |");
    println!("| --- | ---: | --- |");
    for (reason, paths) in unsupported_by_reason {
        let examples = paths
            .iter()
            .take(5)
            .map(|path| format!("`{}`", escape_markdown_cell(&path.display().to_string())))
            .collect::<Vec<_>>()
            .join("<br>");
        let suffix = if paths.len() > 5 {
            format!("<br>... {} more", paths.len() - 5)
        } else {
            String::new()
        };
        println!(
            "| {} | {} | {}{} |",
            escape_markdown_cell(&reason),
            paths.len(),
            examples,
            suffix
        );
    }
}

fn outcome_label(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Match => "MATCH",
        Outcome::Delta => "DELTA",
        Outcome::Mismatch => "MISMATCH",
        Outcome::Unsupported => "UNSUPPORTED",
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

fn validation_policy_label(policy: ValidationPolicy) -> &'static str {
    match policy {
        ValidationPolicy::Exact => "exact",
        ValidationPolicy::Coverage => "coverage",
        ValidationPolicy::Mixed => "mixed",
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

fn native_unsupported_reason(detail: &str) -> Option<String> {
    let marker = "native SemIR codegen is not implemented yet (";
    let start = detail.find(marker)? + marker.len();
    let rest = &detail[start..];
    let end = rest
        .find("; read model:")
        .or_else(|| rest.find(')'))
        .unwrap_or(rest.len());
    Some(rest[..end].trim().to_string())
}

fn parse_profile(value: &str) -> Option<CodegenProfile> {
    match value {
        "legacy" | "compat" => Some(CodegenProfile::Compat),
        "modern" => Some(CodegenProfile::Modern),
        _ => None,
    }
}

fn parse_candidate(value: &str) -> Option<SemIrCandidate> {
    match value {
        "bridge" => Some(SemIrCandidate::Bridge),
        "native" => Some(SemIrCandidate::Native),
        _ => None,
    }
}

fn parse_validation_policy(value: &str) -> Option<ValidationPolicy> {
    match value {
        "exact" => Some(ValidationPolicy::Exact),
        "coverage" => Some(ValidationPolicy::Coverage),
        "mixed" | "by-path" => Some(ValidationPolicy::Mixed),
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

impl SemIrCandidate {
    fn label(self) -> &'static str {
        match self {
            Self::Bridge => "bridge",
            Self::Native => "native",
        }
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
        "usage: actionc-semir-sweep [--candidate bridge|native] [--profile legacy|modern] [--origin <addr>] [--validation-policy exact|coverage|mixed] [--report text|markdown] [--dashboard] [--verbose] [path ...]"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_native_unsupported_reason() {
        let detail = "29..64: native SemIR codegen is not implemented yet (dynamic indexes are not supported; read model: origin=$3000, profile=legacy)";
        assert_eq!(
            native_unsupported_reason(detail),
            Some("dynamic indexes are not supported".to_string())
        );
    }

    #[test]
    fn parses_validation_policy_aliases() {
        assert_eq!(
            parse_validation_policy("exact"),
            Some(ValidationPolicy::Exact)
        );
        assert_eq!(
            parse_validation_policy("coverage"),
            Some(ValidationPolicy::Coverage)
        );
        assert_eq!(
            parse_validation_policy("mixed"),
            Some(ValidationPolicy::Mixed)
        );
        assert_eq!(
            parse_validation_policy("by-path"),
            Some(ValidationPolicy::Mixed)
        );
        assert_eq!(parse_validation_policy("other"), None);
    }

    #[test]
    fn mixed_policy_keeps_semir_fixtures_exact() {
        let config = Config {
            roots: Vec::new(),
            profile: CodegenProfile::Compat,
            origin: CODE_ORIGIN,
            verbose: false,
            candidate: SemIrCandidate::Native,
            dashboard: false,
            validation_policy: ValidationPolicy::Mixed,
            report: ReportFormat::Text,
        };

        assert_eq!(
            output_expectation(&config, Path::new("fixtures/semir/scalar_assignments.act")),
            OutputExpectation::Exact
        );
        assert_eq!(
            output_expectation(&config, Path::new("fixtures/stress/calls.act")),
            OutputExpectation::Coverage
        );
    }

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
