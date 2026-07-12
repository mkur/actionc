use std::env;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process;

use actionc::diagnostic::Diagnostic;
use actionc::includes::load_program_with_expanded_source;
use actionc::nir;
use actionc::semantic::{analyze, ir};

#[derive(Debug)]
struct Config {
    roots: Vec<PathBuf>,
    verbose: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Ok,
    LoadFailed,
    SemFailed,
    NirFailed,
}

#[derive(Debug)]
struct SweepResult {
    path: PathBuf,
    outcome: Outcome,
    detail: String,
}

#[derive(Debug, Default)]
struct SweepCounts {
    ok: usize,
    load_failed: usize,
    sem_failed: usize,
    nir_failed: usize,
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
        eprintln!("actionc-nir-sweep: no Action sources found");
        process::exit(2);
    }

    let mut results = Vec::new();
    for file in files {
        let result = sweep_file(&file);
        print_result(&result, config.verbose);
        results.push(result);
    }

    let counts = count_results(&results);
    println!(
        "NIR sweep summary: ok={} load_failed={} sem_failed={} nir_failed={}",
        counts.ok, counts.load_failed, counts.sem_failed, counts.nir_failed
    );

    if counts.sem_failed + counts.nir_failed > 0 {
        process::exit(1);
    }
}

fn parse_args() -> Config {
    let mut roots = Vec::new();
    let mut verbose = false;

    for arg in env::args().skip(1) {
        match arg.as_str() {
            "-v" | "--verbose" => verbose = true,
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
        roots.push(PathBuf::from("fixtures/nir"));
    }

    Config { roots, verbose }
}

fn sweep_file(path: &Path) -> SweepResult {
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
                outcome: Outcome::SemFailed,
                detail: diagnostic_summary(&diagnostics),
            };
        }
    };

    match catch_unwind(AssertUnwindSafe(|| {
        let semir = ir::lower_program(&loaded.program, &model);
        let program = nir::lower_program(&semir);
        nir::verify_program(&program)
    })) {
        Ok(Ok(())) => SweepResult {
            path: path.to_path_buf(),
            outcome: Outcome::Ok,
            detail: "verified".to_string(),
        },
        Ok(Err(diagnostics)) => SweepResult {
            path: path.to_path_buf(),
            outcome: Outcome::NirFailed,
            detail: nir_diagnostic_summary(&diagnostics),
        },
        Err(payload) => SweepResult {
            path: path.to_path_buf(),
            outcome: Outcome::NirFailed,
            detail: format!("panic: {}", panic_payload_summary(payload)),
        },
    }
}

fn print_result(result: &SweepResult, verbose: bool) {
    let label = match result.outcome {
        Outcome::Ok => "OK",
        Outcome::LoadFailed => "LOADFAIL",
        Outcome::SemFailed => "SEMFAIL",
        Outcome::NirFailed => "NIRFAIL",
    };
    if result.outcome == Outcome::Ok && !verbose {
        println!("{label:<8} {}", result.path.display());
    } else {
        println!("{label:<8} {:<56} {}", result.path.display(), result.detail);
    }
}

fn count_results(results: &[SweepResult]) -> SweepCounts {
    let mut counts = SweepCounts::default();
    for result in results {
        match result.outcome {
            Outcome::Ok => counts.ok += 1,
            Outcome::LoadFailed => counts.load_failed += 1,
            Outcome::SemFailed => counts.sem_failed += 1,
            Outcome::NirFailed => counts.nir_failed += 1,
        }
    }
    counts
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

fn nir_diagnostic_summary(diagnostics: &[nir::NirDiagnostic]) -> String {
    diagnostics
        .iter()
        .take(3)
        .map(|diagnostic| {
            let location = match (&diagnostic.routine, &diagnostic.block) {
                (Some(routine), Some(block)) => format!("{routine}:{block}"),
                (Some(routine), None) => routine.clone(),
                (None, _) => "program".to_string(),
            };
            format!("{location}: {}", diagnostic.message)
        })
        .collect::<Vec<_>>()
        .join("; ")
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

fn usage_and_exit(message: &str) -> ! {
    eprintln!("actionc-nir-sweep: {message}");
    print_usage();
    process::exit(2);
}

fn print_usage() {
    eprintln!("usage: actionc-nir-sweep [--verbose] [path ...]");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_results() {
        let results = vec![
            SweepResult {
                path: PathBuf::from("ok.act"),
                outcome: Outcome::Ok,
                detail: String::new(),
            },
            SweepResult {
                path: PathBuf::from("nirfail.act"),
                outcome: Outcome::NirFailed,
                detail: String::new(),
            },
        ];

        let counts = count_results(&results);
        assert_eq!(counts.ok, 1);
        assert_eq!(counts.nir_failed, 1);
        assert_eq!(counts.load_failed, 0);
        assert_eq!(counts.sem_failed, 0);
    }
}
