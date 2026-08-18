use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;

use crate::ast::Program;
use crate::codegen::{
    CODE_ORIGIN, CodegenOptimizationKind, CodegenOutput, CodegenProfile, format_hex,
    format_load_file, generate_profile_with_origin, generate_semir_native_profile_with_origin,
    generate_semir_profile_with_origin, generate_semir_standalone_profile_at_origin,
};
use crate::compiler::{
    Backend, CodegenSource, CompileError, CompileErrorKind, CompileMode, CompileRequest,
    CompiledProgram, CompilerPhase, DiagnosticSite, Runtime,
    artifacts::{format_listing_with_boundaries, format_listing_with_source},
    compile_file_with_request, mir6502_default_origin_from_semir, mode_profile_backend,
    validation::{legacy_routine_retargeting_diagnostics, standalone_resident_diagnostics},
};
use crate::includes::{ModuleLoadOptions, SourceMap, load_compilation};
use crate::lexer::tokenize;
use crate::mir6502;
use crate::nir;
#[cfg(test)]
use crate::semantic::analyze;
use crate::semantic::{analyze_compilation, ir};
use crate::source::decode_source;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliFlavor {
    Compile,
    Emit,
}

impl CliFlavor {
    fn executable_name(self) -> &'static str {
        match self {
            Self::Compile => "actionc",
            Self::Emit => "actionc-emit",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompileOutputs {
    object: PathBuf,
    listing: Option<PathBuf>,
}

pub fn actionc_main() {
    run_main(CliFlavor::Compile);
}

pub fn emit_main() {
    run_main(CliFlavor::Emit);
}

fn run_main(flavor: CliFlavor) {
    let mut args = env::args().skip(1);
    let mut emit_tokens = false;
    let mut emit_code = false;
    let mut emit_listing = false;
    let mut emit_source_listing = false;
    let mut emit_load = false;
    let mut emit_map = false;
    let mut emit_proofs = false;
    let mut emit_proof_attempts = false;
    let mut emit_semir = false;
    let mut emit_nir = false;
    let mut emit_optimized_nir = false;
    let mut emit_nir_stats = false;
    let mut emit_mir6502 = false;
    let mut emit_materialized_mir6502 = false;
    let mut diagnostic_byte_ranges = false;
    let mut origin = CODE_ORIGIN;
    let mut origin_explicit = false;
    let mut profile = CodegenProfile::default();
    let mut profile_explicit = false;
    let mut codegen_source = CodegenSource::Ast;
    let mut backend = Backend::Classic;
    let mut backend_explicit = false;
    let mut runtime = Runtime::ActionCart;
    let mut runtime_explicit = false;
    let mut compile_mode = None;
    let mut output_path = None;
    let mut listing_path = None;
    let mut input_path = None;
    let mut module_paths = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--emit-tokens" => emit_tokens = true,
            "--emit-code" => emit_code = true,
            "--emit-listing" => emit_listing = true,
            "--emit-source-listing" | "--emit-listing-source" => emit_source_listing = true,
            "--emit-load" => emit_load = true,
            "--emit-map" => emit_map = true,
            "--emit-proofs" => emit_proofs = true,
            "--emit-proof-attempts" | "--emit-proof-debug" => emit_proof_attempts = true,
            "--emit-semir" => emit_semir = true,
            "--emit-nir" => emit_nir = true,
            "--emit-optimized-nir" => emit_optimized_nir = true,
            "--emit-nir-stats" => emit_nir_stats = true,
            "--emit-mir6502" => emit_mir6502 = true,
            "--emit-materialized-mir6502" | "--emit-mir6502-materialized" => {
                emit_materialized_mir6502 = true
            }
            "--diagnostic-byte-ranges" | "--debug-diagnostic-spans" => {
                diagnostic_byte_ranges = true
            }
            "-o" | "--output" => {
                let Some(value) = args.next() else {
                    eprintln!("{arg} requires a file path");
                    print_help_for(flavor);
                    process::exit(2);
                };
                output_path = Some(PathBuf::from(value));
            }
            _ if arg.starts_with("--output=") => {
                output_path = Some(PathBuf::from(&arg["--output=".len()..]));
            }
            "--listing" => {
                let Some(value) = args.next() else {
                    eprintln!("--listing requires a file path");
                    print_help_for(flavor);
                    process::exit(2);
                };
                listing_path = Some(PathBuf::from(value));
            }
            _ if arg.starts_with("--listing=") => {
                listing_path = Some(PathBuf::from(&arg["--listing=".len()..]));
            }
            "--module-path" => {
                let Some(value) = args.next() else {
                    eprintln!("--module-path requires a directory path");
                    print_help_for(flavor);
                    process::exit(2);
                };
                module_paths.push(PathBuf::from(value));
            }
            _ if arg.starts_with("--module-path=") => {
                module_paths.push(PathBuf::from(&arg["--module-path=".len()..]));
            }
            "--mode" => {
                let Some(value) = args.next() else {
                    eprintln!("--mode requires compatibility, optimized, or mir6502");
                    print_help_for(flavor);
                    process::exit(2);
                };
                compile_mode = Some(parse_compile_mode_or_exit(&value));
            }
            _ if arg.starts_with("--mode=") => {
                compile_mode = Some(parse_compile_mode_or_exit(&arg["--mode=".len()..]));
            }
            "--origin" => {
                let Some(value) = args.next() else {
                    eprintln!("--origin requires an address");
                    print_help_for(flavor);
                    process::exit(2);
                };
                origin = parse_origin(&value);
                origin_explicit = true;
            }
            _ if arg.starts_with("--origin=") => {
                origin = parse_origin(&arg["--origin=".len()..]);
                origin_explicit = true;
            }
            "--profile" => {
                let Some(value) = args.next() else {
                    eprintln!("--profile requires legacy or modern");
                    print_help_for(flavor);
                    process::exit(2);
                };
                profile = parse_profile_or_codegen_alias(&value, &mut codegen_source);
                profile_explicit = true;
            }
            _ if arg.starts_with("--profile=") => {
                profile =
                    parse_profile_or_codegen_alias(&arg["--profile=".len()..], &mut codegen_source);
                profile_explicit = true;
            }
            "--codegen-source" => {
                let Some(value) = args.next() else {
                    eprintln!("--codegen-source requires ast, semir, or semir-native");
                    print_help_for(flavor);
                    process::exit(2);
                };
                codegen_source = parse_codegen_source_or_exit(&value);
            }
            _ if arg.starts_with("--codegen-source=") => {
                codegen_source = parse_codegen_source_or_exit(&arg["--codegen-source=".len()..]);
            }
            "--backend" => {
                let Some(value) = args.next() else {
                    eprintln!("--backend requires classic or mir6502");
                    print_help_for(flavor);
                    process::exit(2);
                };
                backend = parse_backend_or_exit(&value);
                backend_explicit = true;
            }
            "--runtime" => {
                let Some(value) = args.next() else {
                    eprintln!("--runtime requires cart or standalone");
                    print_help_for(flavor);
                    process::exit(2);
                };
                runtime = parse_runtime_or_exit(&value);
                runtime_explicit = true;
            }
            _ if arg.starts_with("--backend=") => {
                backend = parse_backend_or_exit(&arg["--backend=".len()..]);
                backend_explicit = true;
            }
            "-h" | "--help" => {
                print_help_for(flavor);
                return;
            }
            "-V" | "--version" => {
                println!(
                    "{}",
                    crate::build_info::version_line(flavor.executable_name())
                );
                return;
            }
            _ if arg.starts_with('-') => {
                eprintln!("unexpected argument: {arg}");
                print_help_for(flavor);
                process::exit(2);
            }
            _ if input_path.is_none() => input_path = Some(arg),
            _ => {
                eprintln!("unexpected argument: {arg}");
                print_help_for(flavor);
                process::exit(2);
            }
        }
    }

    let Some(input_path) = input_path else {
        print_help_for(flavor);
        process::exit(2);
    };
    match flavor {
        CliFlavor::Compile => {
            if let Some(mode) = compile_mode {
                if profile_explicit || backend_explicit {
                    eprintln!("--mode cannot be combined with --profile or --backend");
                    process::exit(2);
                }
                (profile, backend) = mode_profile_backend(mode);
                profile_explicit = true;
                backend_explicit = true;
            }
        }
        CliFlavor::Emit => {
            if compile_mode.is_some() {
                eprintln!(
                    "--mode belongs to actionc; use --profile and --backend with actionc-emit"
                );
                process::exit(2);
            }
        }
    }
    let compile_outputs = match flavor {
        CliFlavor::Compile => {
            if emit_mode_selected(
                emit_tokens,
                emit_code,
                emit_listing,
                emit_source_listing,
                emit_load,
                emit_map,
                emit_proofs,
                emit_proof_attempts,
                emit_semir,
                emit_nir,
                emit_optimized_nir,
                emit_nir_stats,
                emit_mir6502,
                emit_materialized_mir6502,
            ) {
                eprintln!(
                    "--emit-* options belong to actionc-emit; use actionc-emit for stdout output"
                );
                process::exit(2);
            }
            Some(compile_outputs_or_exit(
                &input_path,
                output_path,
                listing_path,
            ))
        }
        CliFlavor::Emit => {
            if output_path.is_some() || listing_path.is_some() {
                eprintln!("-o, --output, and --listing belong to actionc");
                process::exit(2);
            }
            None
        }
    };
    backend = backend_for_profile_default(profile, profile_explicit, backend, backend_explicit);
    if let Some(message) = emit_mode_error(
        emit_tokens,
        emit_code,
        emit_listing,
        emit_source_listing,
        emit_load,
        emit_map,
        emit_proofs,
        emit_proof_attempts,
        emit_semir,
        emit_nir,
        emit_optimized_nir,
        emit_nir_stats,
        emit_mir6502,
        emit_materialized_mir6502,
    ) {
        eprintln!("{message}");
        process::exit(2);
    }

    if let Some(outputs) = &compile_outputs {
        let request = CompileRequest {
            profile,
            profile_explicit,
            backend,
            backend_explicit,
            runtime,
            runtime_explicit,
            codegen_source,
            origin: origin_explicit.then_some(origin),
            project_root: None,
            module_paths: module_paths.clone(),
        };
        let compiled = match compile_file_with_request(Path::new(&input_path), &request) {
            Ok(compiled) => compiled,
            Err(error) => {
                let exit_code = match error.kind() {
                    CompileErrorKind::Configuration => 2,
                    CompileErrorKind::Compilation => 1,
                };
                print_compile_error(&error, diagnostic_byte_ranges);
                process::exit(exit_code);
            }
        };
        write_compiled_program_or_exit(&compiled, outputs);
        return;
    }

    if emit_tokens {
        let source_bytes = match fs::read(&input_path) {
            Ok(source) => source,
            Err(err) => {
                eprintln!("failed to read {input_path}: {err}");
                process::exit(1);
            }
        };
        let source = decode_source(&source_bytes);
        let tokens = match tokenize(&source) {
            Ok(tokens) => tokens,
            Err(diagnostics) => {
                print_diagnostics_with_source_path(
                    diagnostics,
                    &source,
                    Some(Path::new(&input_path)),
                    None,
                    diagnostic_byte_ranges,
                );
                process::exit(1);
            }
        };

        for token in &tokens {
            println!("{:?} {:?}", token.span, token.kind);
        }
        return;
    }

    let loaded = match load_compilation(
        &input_path,
        &ModuleLoadOptions {
            project_root: None,
            module_paths,
        },
    ) {
        Ok(loaded) => loaded,
        Err(diagnostics) => {
            print_input_diagnostics(&input_path, diagnostics, diagnostic_byte_ranges);
            process::exit(1);
        }
    };
    let program = &loaded.root_module().program;
    apply_source_codegen_settings(
        &loaded.source,
        &mut profile,
        profile_explicit,
        &mut backend,
        backend_explicit,
    );
    if let Some(message) = profile_backend_error(profile, backend) {
        eprintln!("{message}");
        process::exit(2);
    }

    let model = match analyze_compilation(&loaded) {
        Ok(model) => model,
        Err(diagnostics) => {
            print_diagnostics_with_source(
                diagnostics,
                &loaded.source,
                Some(&loaded.source_map),
                diagnostic_byte_ranges,
            );
            process::exit(1);
        }
    };
    let semir = ir::lower_compilation(&loaded, &model);
    let named = matches!(program.source_kind, crate::ast::SourceUnitKind::Named(_));

    if runtime == Runtime::Standalone {
        let diagnostics = standalone_resident_diagnostics(&semir);
        if !diagnostics.is_empty() {
            print_diagnostics_with_source(
                diagnostics,
                &loaded.source,
                Some(&loaded.source_map),
                diagnostic_byte_ranges,
            );
            process::exit(1);
        }
    }

    if emit_semir {
        print!("{}", ir::format_program(&semir));
        return;
    }

    if emit_nir {
        reject_nir_unsupported_legacy_routine_retargeting_or_exit(
            program,
            &loaded.source,
            Some(&loaded.source_map),
            diagnostic_byte_ranges,
        );
        let nir = nir::lower_program(&semir);
        if let Err(diagnostics) = nir::verify_program(&nir) {
            print_nir_diagnostics(diagnostics);
            process::exit(1);
        }
        print!("{}", nir::format_program(&nir));
        return;
    }

    if emit_optimized_nir || emit_nir_stats {
        reject_nir_unsupported_legacy_routine_retargeting_or_exit(
            program,
            &loaded.source,
            Some(&loaded.source_map),
            diagnostic_byte_ranges,
        );
        let lowered = nir::lower_program(&semir);
        let optimized = optimize_nir_or_exit(lowered.clone());
        if emit_nir_stats {
            print!("{}", nir::format_stats_comparison(&lowered, &optimized));
        } else {
            print!("{}", nir::format_program(&optimized));
        }
        return;
    }

    if emit_mir6502 || emit_materialized_mir6502 {
        reject_nir_unsupported_legacy_routine_retargeting_or_exit(
            program,
            &loaded.source,
            Some(&loaded.source_map),
            diagnostic_byte_ranges,
        );
        let nir = optimize_nir_or_exit(nir::lower_program(&semir));
        let mir = match mir6502::lower_program(&nir) {
            Ok(mir) => mir,
            Err(diagnostics) => {
                print_mir6502_diagnostics(diagnostics);
                process::exit(1);
            }
        };
        if let Err(diagnostics) =
            mir6502::verify_program(&mir, mir6502::MirPhase::PreMaterialization)
        {
            print_mir6502_diagnostics(diagnostics);
            process::exit(1);
        }
        if emit_materialized_mir6502 {
            let mir = match mir6502::materialize_program_with_origin_and_runtime(
                mir,
                &mir6502::Mir6502Config::default(),
                CODE_ORIGIN,
                runtime,
            ) {
                Ok(mir) => mir,
                Err(diagnostics) => {
                    print_mir6502_diagnostics(diagnostics);
                    process::exit(1);
                }
            };
            if let Err(diagnostics) = mir6502::verify_program(&mir, mir6502::MirPhase::PreEmission)
            {
                print_mir6502_diagnostics(diagnostics);
                process::exit(1);
            }
            print!("{}", mir6502::format_program(&mir));
            return;
        }
        print!("{}", mir6502::format_program(&mir));
        return;
    }

    if should_run_codegen_backend(
        emit_code,
        emit_listing,
        emit_source_listing,
        emit_load,
        emit_map,
        emit_proofs,
        emit_proof_attempts,
        backend,
    ) {
        if matches!(backend, Backend::Mir6502) {
            reject_nir_unsupported_legacy_routine_retargeting_or_exit(
                program,
                &loaded.source,
                Some(&loaded.source_map),
                diagnostic_byte_ranges,
            );
            let nir = optimize_nir_or_exit(nir::lower_program(&semir));
            let mir_origin = if origin_explicit {
                origin
            } else {
                mir6502_default_origin_from_semir(&semir, origin)
            };
            let mir_config = if matches!(profile, CodegenProfile::Modern) {
                mir6502::Mir6502Config::optimized()
            } else {
                mir6502::Mir6502Config::default()
            };
            match mir6502::generate_output_with_config_and_runtime(
                &nir,
                mir_origin,
                &mir_config,
                runtime,
            ) {
                Ok(output) => emit_output(
                    &output,
                    &loaded.source,
                    emit_load,
                    emit_map,
                    emit_proofs,
                    emit_proof_attempts,
                    emit_listing,
                    emit_source_listing,
                ),
                Err(diagnostics) => {
                    print_mir6502_diagnostics(diagnostics);
                    process::exit(1);
                }
            }
            return;
        }

        if runtime == Runtime::Standalone {
            let standalone_origin = if origin_explicit {
                origin
            } else {
                mir6502_default_origin_from_semir(&semir, origin)
            };
            match generate_semir_standalone_profile_at_origin(&semir, standalone_origin, profile) {
                Ok(output) => emit_output(
                    &output,
                    &loaded.source,
                    emit_load,
                    emit_map,
                    emit_proofs,
                    emit_proof_attempts,
                    emit_listing,
                    emit_source_listing,
                ),
                Err(diagnostics) => {
                    print_diagnostics_with_source(
                        diagnostics,
                        &loaded.source,
                        Some(&loaded.source_map),
                        diagnostic_byte_ranges,
                    );
                    process::exit(1);
                }
            }
            return;
        }

        if matches!(
            codegen_source,
            CodegenSource::SemIr | CodegenSource::SemIrNative
        ) {
            let result = match codegen_source {
                CodegenSource::SemIr => generate_semir_profile_with_origin(&semir, origin, profile),
                CodegenSource::SemIrNative => {
                    generate_semir_native_profile_with_origin(&semir, origin, profile)
                }
                CodegenSource::Ast => unreachable!("AST codegen handled separately"),
            };
            match result {
                Ok(output) => emit_output(
                    &output,
                    &loaded.source,
                    emit_load,
                    emit_map,
                    emit_proofs,
                    emit_proof_attempts,
                    emit_listing,
                    emit_source_listing,
                ),
                Err(diagnostics) => {
                    print_diagnostics_with_source(
                        diagnostics,
                        &loaded.source,
                        Some(&loaded.source_map),
                        diagnostic_byte_ranges,
                    );
                    process::exit(1);
                }
            }
            return;
        }

        let result = if named {
            generate_semir_profile_with_origin(&semir, origin, profile)
        } else {
            generate_profile_with_origin(program, origin, profile)
        };
        match result {
            Ok(output) => emit_output(
                &output,
                &loaded.source,
                emit_load,
                emit_map,
                emit_proofs,
                emit_proof_attempts,
                emit_listing,
                emit_source_listing,
            ),
            Err(diagnostics) => {
                print_diagnostics_with_source(
                    diagnostics,
                    &loaded.source,
                    Some(&loaded.source_map),
                    diagnostic_byte_ranges,
                );
                process::exit(1);
            }
        }
        return;
    }

    println!(
        "parsed {} module(s); compiler backend is not implemented yet",
        program.modules.len()
    );
}

fn print_diagnostics(diagnostics: Vec<crate::diagnostic::Diagnostic>) {
    for diagnostic in diagnostics {
        eprintln!(
            "unknown location: {} (span {}..{})",
            diagnostic.message, diagnostic.span.start, diagnostic.span.end
        );
    }
}

fn print_nir_diagnostics(diagnostics: Vec<crate::nir::NirDiagnostic>) {
    for diagnostic in diagnostics {
        match (&diagnostic.routine, &diagnostic.block) {
            (Some(routine), Some(block)) => {
                eprintln!("nir {routine}:{block}: {}", diagnostic.message)
            }
            (Some(routine), None) => eprintln!("nir {routine}: {}", diagnostic.message),
            (None, _) => eprintln!("nir: {}", diagnostic.message),
        }
    }
}

fn print_mir6502_diagnostics(diagnostics: Vec<crate::mir6502::MirDiagnostic>) {
    for diagnostic in diagnostics {
        match (&diagnostic.routine, &diagnostic.block) {
            (Some(routine), Some(block)) => {
                eprintln!("mir6502 {routine}:{block}: {}", diagnostic.message)
            }
            (Some(routine), None) => eprintln!("mir6502 {routine}: {}", diagnostic.message),
            (None, _) => eprintln!("mir6502: {}", diagnostic.message),
        }
    }
}

fn reject_nir_unsupported_legacy_routine_retargeting_or_exit(
    program: &Program,
    source: &str,
    source_map: Option<&SourceMap>,
    diagnostic_byte_ranges: bool,
) {
    let diagnostics = legacy_routine_retargeting_diagnostics(program);
    if diagnostics.is_empty() {
        return;
    }
    print_diagnostics_with_source(diagnostics, source, source_map, diagnostic_byte_ranges);
    process::exit(1);
}

fn parse_backend_or_exit(value: &str) -> Backend {
    match value {
        "classic" | "legacy" | "default" => Backend::Classic,
        "mir6502" => Backend::Mir6502,
        _ => {
            eprintln!("unknown backend: {value}");
            process::exit(2);
        }
    }
}

fn parse_runtime_or_exit(value: &str) -> Runtime {
    match value {
        "cart" => Runtime::ActionCart,
        "standalone" => Runtime::Standalone,
        _ => {
            eprintln!("unknown runtime: {value}; expected cart or standalone");
            process::exit(2);
        }
    }
}

fn parse_compile_mode_or_exit(value: &str) -> CompileMode {
    match value {
        "compatibility" => CompileMode::Compatibility,
        "optimized" => CompileMode::Optimized,
        "mir6502" => CompileMode::Mir6502,
        _ => {
            eprintln!("unknown mode: {value}; expected compatibility, optimized, or mir6502");
            process::exit(2);
        }
    }
}

fn backend_for_profile_default(
    _profile: CodegenProfile,
    _profile_explicit: bool,
    backend: Backend,
    _backend_explicit: bool,
) -> Backend {
    backend
}

fn profile_backend_error(profile: CodegenProfile, backend: Backend) -> Option<&'static str> {
    if matches!(profile, CodegenProfile::Compat) && matches!(backend, Backend::Mir6502) {
        Some("--backend mir6502 requires --profile modern")
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_mode_error(
    emit_tokens: bool,
    emit_code: bool,
    emit_listing: bool,
    emit_source_listing: bool,
    emit_load: bool,
    emit_map: bool,
    emit_proofs: bool,
    emit_proof_attempts: bool,
    emit_semir: bool,
    emit_nir: bool,
    emit_optimized_nir: bool,
    emit_nir_stats: bool,
    emit_mir6502: bool,
    emit_materialized_mir6502: bool,
) -> Option<String> {
    let mut modes = Vec::new();
    if emit_tokens {
        modes.push("--emit-tokens");
    }
    if emit_semir {
        modes.push("--emit-semir");
    }
    if emit_nir {
        modes.push("--emit-nir");
    }
    if emit_optimized_nir {
        modes.push("--emit-optimized-nir");
    }
    if emit_nir_stats {
        modes.push("--emit-nir-stats");
    }
    if emit_mir6502 {
        modes.push("--emit-mir6502");
    }
    if emit_materialized_mir6502 {
        modes.push("--emit-materialized-mir6502");
    }
    if emit_code {
        modes.push("--emit-code");
    }
    if emit_listing {
        modes.push("--emit-listing");
    }
    if emit_source_listing {
        modes.push("--emit-source-listing");
    }
    if emit_load {
        modes.push("--emit-load");
    }
    if emit_map {
        modes.push("--emit-map");
    }
    if emit_proofs {
        modes.push("--emit-proofs");
    }
    if emit_proof_attempts {
        modes.push("--emit-proof-attempts");
    }

    (modes.len() > 1).then(|| format!("multiple emit modes selected: {}", modes.join(", ")))
}

#[allow(clippy::too_many_arguments)]
fn emit_mode_selected(
    emit_tokens: bool,
    emit_code: bool,
    emit_listing: bool,
    emit_source_listing: bool,
    emit_load: bool,
    emit_map: bool,
    emit_proofs: bool,
    emit_proof_attempts: bool,
    emit_semir: bool,
    emit_nir: bool,
    emit_optimized_nir: bool,
    emit_nir_stats: bool,
    emit_mir6502: bool,
    emit_materialized_mir6502: bool,
) -> bool {
    emit_tokens
        || emit_code
        || emit_listing
        || emit_source_listing
        || emit_load
        || emit_map
        || emit_proofs
        || emit_proof_attempts
        || emit_semir
        || emit_nir
        || emit_optimized_nir
        || emit_nir_stats
        || emit_mir6502
        || emit_materialized_mir6502
}

fn compile_outputs_or_exit(
    input_path: &str,
    output_path: Option<PathBuf>,
    listing_path: Option<PathBuf>,
) -> CompileOutputs {
    let object = output_path.unwrap_or_else(|| default_object_path(input_path));
    if object.as_os_str() == "-" {
        eprintln!("-o - is not supported; use actionc-emit --emit-load for stdout output");
        process::exit(2);
    }
    if listing_path.as_ref() == Some(&object) {
        eprintln!("object and listing output paths must be different");
        process::exit(2);
    }
    CompileOutputs {
        object,
        listing: listing_path,
    }
}

fn default_object_path(input_path: &str) -> PathBuf {
    let stem = Path::new(input_path)
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| std::ffi::OsStr::new("output"));
    PathBuf::from(stem).with_extension("com")
}

fn should_run_codegen_backend(
    emit_code: bool,
    emit_listing: bool,
    emit_source_listing: bool,
    emit_load: bool,
    emit_map: bool,
    emit_proofs: bool,
    emit_proof_attempts: bool,
    backend: Backend,
) -> bool {
    emit_code
        || emit_listing
        || emit_source_listing
        || emit_load
        || emit_map
        || emit_proofs
        || emit_proof_attempts
        || matches!(backend, Backend::Classic)
        || matches!(backend, Backend::Mir6502)
}

fn apply_source_codegen_settings(
    source_text: &str,
    profile: &mut CodegenProfile,
    profile_explicit: bool,
    backend: &mut Backend,
    backend_explicit: bool,
) {
    for line in source_text.lines() {
        let Some(annotation) = line.trim_start().strip_prefix(";@actionc") else {
            continue;
        };
        let normalized = annotation
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase();
        match normalized.as_str() {
            "profile modern" if !profile_explicit => *profile = CodegenProfile::Modern,
            "backend classic" if !backend_explicit => *backend = Backend::Classic,
            "backend mir6502" if !backend_explicit => *backend = Backend::Mir6502,
            _ => {}
        }
    }
}

fn print_diagnostics_with_source(
    diagnostics: Vec<crate::diagnostic::Diagnostic>,
    source_text: &str,
    source_map: Option<&SourceMap>,
    include_byte_ranges: bool,
) {
    print_diagnostics_with_source_path(
        diagnostics,
        source_text,
        None,
        source_map,
        include_byte_ranges,
    );
}

fn print_input_diagnostics(
    input_path: &str,
    diagnostics: Vec<crate::diagnostic::Diagnostic>,
    include_byte_ranges: bool,
) {
    let Ok(source_bytes) = fs::read(input_path) else {
        print_diagnostics(diagnostics);
        return;
    };
    let source = decode_source(&source_bytes);
    print_diagnostics_with_source_path(
        diagnostics,
        &source,
        Some(Path::new(input_path)),
        None,
        include_byte_ranges,
    );
}

fn print_diagnostics_with_source_path(
    diagnostics: Vec<crate::diagnostic::Diagnostic>,
    source_text: &str,
    fallback_path: Option<&Path>,
    source_map: Option<&SourceMap>,
    include_byte_ranges: bool,
) {
    for diagnostic in diagnostics {
        let mapped = source_map.and_then(|source_map| source_map.location(diagnostic.span));
        let location = mapped
            .as_ref()
            .map(|location| format!("{}:{}:{}", location.origin, location.line, location.column))
            .unwrap_or_else(|| {
                let location = source_location(source_text, diagnostic.span);
                fallback_path
                    .map(|path| format!("{}:{location}", path.display()))
                    .unwrap_or(location)
            });
        let excerpt = mapped
            .as_ref()
            .map(|location| location.excerpt.clone())
            .or_else(|| source_excerpt(source_text, diagnostic.span));
        let byte_range = if include_byte_ranges {
            format!(" {}..{}", diagnostic.span.start, diagnostic.span.end)
        } else {
            String::new()
        };
        eprintln!(
            "{}{}: {}{}",
            location,
            byte_range,
            diagnostic.message,
            excerpt
                .map(|excerpt| format!(" | {excerpt}"))
                .unwrap_or_default()
        );
    }
}

fn print_compile_error(error: &CompileError, include_byte_ranges: bool) {
    for diagnostic in error.diagnostics() {
        match &diagnostic.site {
            DiagnosticSite::Source {
                origin,
                line,
                column,
                byte_range,
                excerpt,
            } => {
                let byte_range = if include_byte_ranges {
                    byte_range
                        .as_ref()
                        .map(|range| format!(" {}..{}", range.start, range.end))
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                eprintln!(
                    "{}:{}:{}{}: {}{}",
                    origin,
                    line,
                    column,
                    byte_range,
                    diagnostic.message,
                    excerpt
                        .as_ref()
                        .map(|excerpt| format!(" | {excerpt}"))
                        .unwrap_or_default()
                );
            }
            DiagnosticSite::File { origin } => {
                eprintln!("{}: {}", origin, diagnostic.message)
            }
            DiagnosticSite::Ir { routine, block } => {
                let phase = compiler_phase_name(diagnostic.phase);
                match (routine, block) {
                    (Some(routine), Some(block)) => {
                        eprintln!("{phase} {routine}:{block}: {}", diagnostic.message)
                    }
                    (Some(routine), None) => {
                        eprintln!("{phase} {routine}: {}", diagnostic.message)
                    }
                    (None, _) => eprintln!("{phase}: {}", diagnostic.message),
                }
            }
            DiagnosticSite::Unknown => eprintln!("{}", diagnostic.message),
        }
    }
}

fn compiler_phase_name(phase: CompilerPhase) -> &'static str {
    match phase {
        CompilerPhase::Configuration => "configuration",
        CompilerPhase::Input => "input",
        CompilerPhase::Frontend => "frontend",
        CompilerPhase::Semantic => "semantic",
        CompilerPhase::Nir => "nir",
        CompilerPhase::Mir6502 => "mir6502",
        CompilerPhase::Codegen => "codegen",
    }
}

fn emit_output(
    output: &CodegenOutput,
    source_text: &str,
    emit_load: bool,
    emit_map: bool,
    emit_proofs: bool,
    emit_proof_attempts: bool,
    emit_listing: bool,
    emit_source_listing: bool,
) {
    if emit_load {
        if let Err(err) = io::stdout().write_all(&format_load_file(output)) {
            eprintln!("failed to write load file: {err}");
            process::exit(1);
        }
    } else if emit_map {
        print_map(output);
    } else if emit_proofs {
        print_proofs(output, source_text);
    } else if emit_proof_attempts {
        print_proof_attempts(output, source_text);
    } else if emit_listing {
        println!("{}", format_listing_with_boundaries(output));
    } else if emit_source_listing {
        println!("{}", format_listing_with_source(output, source_text));
    } else {
        println!("{}", format_hex(&output.bytes));
    }
}

fn write_compiled_program_or_exit(compiled: &CompiledProgram, outputs: &CompileOutputs) {
    if let Err(err) = write_file_atomically(&outputs.object, compiled.object_bytes()) {
        eprintln!(
            "failed to write object file {}: {err}",
            outputs.object.display()
        );
        process::exit(1);
    }
    if let Some(path) = &outputs.listing {
        let listing = compiled.source_listing();
        if let Err(err) = write_file_atomically(path, listing.as_bytes()) {
            eprintln!("failed to write listing file {}: {err}", path.display());
            process::exit(1);
        }
    }
}

fn write_file_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("output");
    let temporary = path.with_file_name(format!(".{file_name}.actionc-{}.tmp", process::id()));
    if let Err(err) = fs::write(&temporary, contents) {
        let _ = fs::remove_file(&temporary);
        return Err(err);
    }
    if let Err(err) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(err);
    }
    Ok(())
}

fn print_map(output: &CodegenOutput) {
    println!("runtime {}", output.map.runtime);
    for binding in &output.map.runtime_bindings {
        let target = binding
            .address
            .map(|address| format!("${address:04X}"))
            .unwrap_or_else(|| binding.implementation.clone());
        println!(
            "runtime-binding {} {} origin={} reason={}{}",
            binding.helper,
            target,
            binding.origin,
            binding.reason,
            binding
                .suppressed_default
                .as_ref()
                .map(|default| format!(" suppressed-default={default}"))
                .unwrap_or_default()
        );
    }
    for routine in &output.routine_addresses {
        println!("${:04X} {}", routine.address, routine.name);
    }
    for signature in &output.map.routine_signatures {
        let address = output
            .map
            .routine_addresses
            .iter()
            .find(|routine| routine.name == signature.name)
            .map(|routine| format!("${:04X}", routine.address))
            .unwrap_or_else(|| "-".to_string());
        println!(
            "signature {} {} kind={} params={} return={}",
            address,
            signature.name,
            signature.kind,
            format_signature_params(&signature.params),
            format_signature_return(signature.return_type.as_deref(), signature.return_width)
        );
    }
    for optimization in &output.optimizations {
        let address = optimization
            .address
            .map(|address| format!("${address:04X}"))
            .unwrap_or_else(|| "-".to_string());
        let routine = optimization.routine.as_deref().unwrap_or("program");
        println!(
            "opt {:<22} {:>4} bytes {} {}",
            format_optimization_kind(optimization.kind),
            optimization.bytes_saved,
            address,
            routine
        );
    }
    for effect in &output.map.routine_effects {
        println!("effect {:<20} {}", effect.routine, effect.summary);
    }
    for analysis in &output.map.machine_blocks {
        let routine = analysis.routine.as_deref().unwrap_or("program");
        println!(
            "machine {:<20} ${:04X} {}",
            routine, analysis.address, analysis.summary
        );
    }
    for proof in &output.proofs {
        let address = proof
            .address
            .map(|address| format!("${address:04X}"))
            .unwrap_or_else(|| "-".to_string());
        let routine = proof.routine.as_deref().unwrap_or("program");
        println!(
            "proof {:<20} {} {} {}",
            proof.kind, address, routine, proof.summary
        );
    }
}

fn format_signature_params(params: &[crate::codegen::CodegenRoutineParam]) -> String {
    if params.is_empty() {
        return "-".to_string();
    }
    params
        .iter()
        .map(|param| format!("{}:{}:{}", param.name, param.type_name, param.width))
        .collect::<Vec<_>>()
        .join(",")
}

fn format_signature_return(return_type: Option<&str>, return_width: Option<u16>) -> String {
    match (return_type, return_width) {
        (Some(return_type), Some(width)) => format!("{return_type}:{width}"),
        (Some(return_type), None) => return_type.to_string(),
        _ => "-".to_string(),
    }
}

fn print_proofs(output: &CodegenOutput, source_text: &str) {
    for proof in &output.proofs {
        let address = proof
            .address
            .map(|address| format!("${address:04X}"))
            .unwrap_or_else(|| "-".to_string());
        let routine = proof.routine.as_deref().unwrap_or("program");
        let location = source_location(source_text, proof.source_span);
        println!(
            "{:<20} {:>6} {:<20} {} | {}",
            proof.kind, address, routine, location, proof.summary
        );
    }
}

fn print_proof_attempts(output: &CodegenOutput, source_text: &str) {
    for attempt in &output.proof_attempts {
        let address = attempt
            .address
            .map(|address| format!("${address:04X}"))
            .unwrap_or_else(|| "-".to_string());
        let routine = attempt.routine.as_deref().unwrap_or("program");
        let location = source_location(source_text, attempt.source_span);
        let status = if attempt.accepted { "ok" } else { "reject" };
        println!(
            "{:<7} {:<20} {:>6} {:<20} {} | {}",
            status, attempt.kind, address, routine, location, attempt.summary
        );
    }
}

fn source_location(source_text: &str, span: crate::source::Span) -> String {
    let mut line = 1usize;
    let mut column = 1usize;
    for (offset, ch) in source_text.char_indices() {
        if offset >= span.start {
            break;
        }
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    format!("{line}:{column}")
}

fn source_excerpt(source_text: &str, span: crate::source::Span) -> Option<String> {
    if span.start >= source_text.len() {
        return None;
    }
    let line_start = source_text[..span.start]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let line_end = source_text[span.start..]
        .find('\n')
        .map(|index| span.start + index)
        .unwrap_or(source_text.len());
    let line = source_text[line_start..line_end].trim();
    (!line.is_empty()).then(|| line.to_string())
}

fn print_help_for(flavor: CliFlavor) {
    match flavor {
        CliFlavor::Compile => print_compile_help(),
        CliFlavor::Emit => print_help(),
    }
}

fn print_compile_help() {
    eprintln!(
        "usage: actionc [--mode compatibility|optimized|mir6502] [--runtime cart|standalone] [--origin <addr>] [-o <file.com>] [--listing <file.asm>] <file.act>\n       actionc --version\n\nCompile an Action! source file to an Atari load-format object.\nThe default mode is compatibility and the default runtime is cart. Advanced\nusers may select --profile and --backend directly instead of --mode. With no\n-o option, write <source-stem>.com in the current directory. --listing writes\nre-originable, source-annotated MADS assembly. Change only ACTIONC_ORIGIN in\nthe generated listing to move its main segment. Use actionc-emit for compiler\nrepresentations."
    );
}

fn print_help() {
    eprintln!(
        "usage: actionc-emit [--emit-tokens] [--emit-semir|--emit-nir|--emit-optimized-nir|--emit-nir-stats|--emit-mir6502|--emit-materialized-mir6502|--emit-code|--emit-listing|--emit-source-listing|--emit-load|--emit-map|--emit-proofs|--emit-proof-attempts] [--diagnostic-byte-ranges] [--runtime cart|standalone] [--origin <addr>] [--profile legacy|modern] [--backend classic|mir6502] <file.act>\n       actionc-emit --version\n\nListings are re-originable MADS assembly. Change only ACTIONC_ORIGIN to move\nthe main segment. --emit-source-listing adds Action! source comments."
    );
}

fn parse_profile(value: &str) -> CodegenProfile {
    match value {
        "legacy" | "compat" => CodegenProfile::Compat,
        "modern" => CodegenProfile::Modern,
        _ => {
            eprintln!("invalid codegen profile: {value}");
            process::exit(2);
        }
    }
}

fn parse_profile_or_codegen_alias(
    value: &str,
    codegen_source: &mut CodegenSource,
) -> CodegenProfile {
    match value {
        "semir-native" | "sem-ir-native" | "native-ir" | "modern-ir" => {
            *codegen_source = CodegenSource::SemIrNative;
            CodegenProfile::Modern
        }
        _ => parse_profile(value),
    }
}

fn parse_codegen_source(value: &str) -> Option<CodegenSource> {
    match value {
        "ast" => Some(CodegenSource::Ast),
        "semir" | "sem-ir" => Some(CodegenSource::SemIr),
        "native" | "semir-native" | "sem-ir-native" | "native-ir" | "modern-ir" => {
            Some(CodegenSource::SemIrNative)
        }
        _ => None,
    }
}

fn parse_codegen_source_or_exit(value: &str) -> CodegenSource {
    parse_codegen_source(value).unwrap_or_else(|| {
        eprintln!("invalid codegen source: {value}");
        process::exit(2);
    })
}

fn format_optimization_kind(kind: CodegenOptimizationKind) -> &'static str {
    match kind {
        CodegenOptimizationKind::TrampolineElided => "trampoline-elided",
        CodegenOptimizationKind::ParameterStorageElided => "parameter-storage-elided",
        CodegenOptimizationKind::FinalRtsRemoved => "final-rts-removed",
        CodegenOptimizationKind::RegisterReloadRemoved => "register-reload-removed",
        CodegenOptimizationKind::ConstantStoreReusedRegister => "constant-store-reused-register",
        CodegenOptimizationKind::CallResultMaterializationRemoved => {
            "call-result-materialization-removed"
        }
        CodegenOptimizationKind::PointerReloadRemoved => "pointer-reload-removed",
        CodegenOptimizationKind::EffectiveAddressLowered => "effective-address-lowered",
        CodegenOptimizationKind::EffectiveAddressReused => "effective-address-reused",
        CodegenOptimizationKind::ArgumentStoreRemoved => "argument-store-removed",
        CodegenOptimizationKind::ArgumentStackForwarded => "argument-stack-forwarded",
        CodegenOptimizationKind::BranchInverted => "branch-inverted",
        CodegenOptimizationKind::TailCall => "tail-call",
        CodegenOptimizationKind::JumpToRtsRemoved => "jump-to-rts-removed",
        CodegenOptimizationKind::CallFactPreserved => "call-fact-preserved",
    }
}

fn optimize_nir_or_exit(program: nir::NirProgram) -> nir::NirProgram {
    match nir::optimize_program(&program) {
        Ok(program) => program,
        Err(diagnostics) => {
            print_nir_diagnostics(diagnostics);
            process::exit(1);
        }
    }
}

fn parse_origin(value: &str) -> u16 {
    let parsed = if let Some(hex) = value.strip_prefix('$') {
        u16::from_str_radix(hex, 16)
    } else if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u16::from_str_radix(hex, 16)
    } else {
        value.parse::<u16>()
    };

    match parsed {
        Ok(origin) => origin,
        Err(_) => {
            eprintln!("invalid origin address: {value}");
            process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::{CodegenMap, RoutineAddress, RoutineRange, opcode};

    #[test]
    fn parses_codegen_source_modes() {
        assert_eq!(parse_codegen_source("ast"), Some(CodegenSource::Ast));
        assert_eq!(parse_codegen_source("semir"), Some(CodegenSource::SemIr));
        assert_eq!(parse_codegen_source("sem-ir"), Some(CodegenSource::SemIr));
        assert_eq!(
            parse_codegen_source("semir-native"),
            Some(CodegenSource::SemIrNative)
        );
        assert_eq!(
            parse_codegen_source("modern-ir"),
            Some(CodegenSource::SemIrNative)
        );
        assert_eq!(parse_codegen_source("modern"), None);
    }

    #[test]
    fn parses_native_codegen_profile_alias_for_compatibility() {
        let mut codegen_source = CodegenSource::Ast;
        let profile = parse_profile_or_codegen_alias("semir-native", &mut codegen_source);
        assert_eq!(profile, CodegenProfile::Modern);
        assert_eq!(codegen_source, CodegenSource::SemIrNative);
    }

    #[test]
    fn explicit_modern_profile_keeps_default_classic_backend() {
        assert_eq!(
            backend_for_profile_default(CodegenProfile::Modern, true, Backend::Classic, false),
            Backend::Classic
        );
        assert_eq!(
            backend_for_profile_default(CodegenProfile::Modern, true, Backend::Classic, true),
            Backend::Classic
        );
        assert_eq!(
            backend_for_profile_default(CodegenProfile::Modern, false, Backend::Classic, false),
            Backend::Classic
        );
        assert_eq!(parse_profile("legacy"), CodegenProfile::Compat);
        assert_eq!(parse_profile("compat"), CodegenProfile::Compat);
        assert_eq!(parse_backend_or_exit("classic"), Backend::Classic);
        assert_eq!(parse_backend_or_exit("legacy"), Backend::Classic);
    }

    #[test]
    fn legacy_profile_rejects_mir6502_backend() {
        assert_eq!(
            profile_backend_error(CodegenProfile::Compat, Backend::Mir6502),
            Some("--backend mir6502 requires --profile modern")
        );
        assert_eq!(
            profile_backend_error(CodegenProfile::Modern, Backend::Mir6502),
            None
        );
        assert_eq!(
            profile_backend_error(CodegenProfile::Compat, Backend::Classic),
            None
        );
    }

    #[test]
    fn multiple_emit_modes_are_rejected() {
        assert_eq!(
            emit_mode_error(
                false, false, true, false, true, false, false, false, false, false, false, false,
                false, false,
            ),
            Some("multiple emit modes selected: --emit-listing, --emit-load".to_string())
        );
        assert_eq!(
            emit_mode_error(
                false, false, false, false, true, false, false, false, false, false, false, false,
                false, false,
            ),
            None
        );
    }

    #[test]
    fn default_classic_backend_runs_codegen_without_emit_flag() {
        assert!(should_run_codegen_backend(
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            Backend::Classic
        ));
        assert!(should_run_codegen_backend(
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            Backend::Mir6502
        ));
    }

    #[test]
    fn emit_modes_run_codegen_backend() {
        assert!(should_run_codegen_backend(
            true,
            false,
            false,
            false,
            false,
            false,
            false,
            Backend::Classic
        ));
    }

    #[test]
    fn source_codegen_settings_fill_unspecified_cli_defaults() {
        let mut profile = CodegenProfile::Compat;
        let mut backend = Backend::Classic;

        apply_source_codegen_settings(
            ";@actionc profile modern\n;@actionc backend mir6502\nPROC Main() RETURN",
            &mut profile,
            false,
            &mut backend,
            false,
        );

        assert_eq!(profile, CodegenProfile::Modern);
        assert_eq!(backend, Backend::Mir6502);
    }

    #[test]
    fn explicit_cli_settings_override_source_codegen_settings() {
        let mut profile = CodegenProfile::Compat;
        let mut backend = Backend::Classic;

        apply_source_codegen_settings(
            ";@actionc profile modern\n;@actionc backend mir6502\nPROC Main() RETURN",
            &mut profile,
            true,
            &mut backend,
            true,
        );

        assert_eq!(profile, CodegenProfile::Compat);
        assert_eq!(backend, Backend::Classic);
    }

    #[test]
    fn mir6502_default_origin_honors_action_code_pointer_sets() {
        let source = "SET $491=$E6 SET $492=$00 SET $491=$2C00 PROC Main() RETURN";
        let tokens = tokenize(source).unwrap();
        let program = crate::parser::parse(&tokens).unwrap();
        let model = analyze(&program).unwrap();
        let semir = ir::lower_program(&program, &model);

        assert_eq!(
            mir6502_default_origin_from_semir(&semir, CODE_ORIGIN),
            0x2C00
        );
    }

    #[test]
    fn nir_preflight_rejects_legacy_routine_retargeting() {
        let tokens = tokenize("PROC A() RETURN PROC T() PROC Main() T=A T() RETURN").unwrap();
        let program = crate::parser::parse(&tokens).unwrap();
        let diagnostics = legacy_routine_retargeting_diagnostics(&program);

        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics[0]
                .message
                .contains("legacy routine-name retargeting")
        );
    }

    #[test]
    fn nir_preflight_allows_function_pointer_assignment() {
        let tokens =
            tokenize("CARD FUNC POINTER fp PROC A() RETURN PROC Main() fp=@A RETURN").unwrap();
        let program = crate::parser::parse(&tokens).unwrap();

        assert!(legacy_routine_retargeting_diagnostics(&program).is_empty());
    }

    #[test]
    fn listing_marks_rpar_inline_payload_as_data() {
        let origin = 0x3A99;
        let bytes = vec![
            opcode::JSR_ABS,
            0x81,
            0x32,
            0xDF,
            opcode::ROL_ABS,
            0x02,
            opcode::LDA_ABS,
            0xDF,
            0x2E,
            opcode::JMP_ABS,
            0x9F,
            0x3A,
        ];
        let end = origin + bytes.len() as u16;
        let routine_addresses = vec![RoutineAddress {
            name: "r_Par".to_string(),
            address: 0x3281,
        }];
        let routine_ranges = vec![RoutineRange {
            name: "FindItem".to_string(),
            start: origin,
            end,
        }];
        let output = CodegenOutput {
            bytes,
            origin,
            run_address: origin,
            relocations: Vec::new(),
            skipped_ranges: Vec::new(),
            routine_addresses: routine_addresses.clone(),
            optimizations: Vec::new(),
            proofs: Vec::new(),
            proof_attempts: Vec::new(),
            map: CodegenMap {
                runtime: crate::runtime::Runtime::ActionCart,
                runtime_bindings: Vec::new(),
                origin,
                run_address: origin,
                skipped_ranges: Vec::new(),
                routine_addresses,
                routine_ranges,
                routine_signatures: Vec::new(),
                storage_symbols: Vec::new(),
                source_ranges: Vec::new(),
                routine_effects: Vec::new(),
                machine_blocks: Vec::new(),
                optimizations: Vec::new(),
                proofs: Vec::new(),
                proof_attempts: Vec::new(),
            },
        };

        let listing = format_listing_with_boundaries(&output);

        assert!(listing.contains("JSR.A proc_r_par"));
        assert!(listing.contains("; $3A99: 20 81 32 ; r_Par"));
        assert!(listing.contains(".BYTE $DF,$2E,$02"));
        assert!(listing.contains("; $3A9C: DF 2E 02"));
        assert!(listing.contains("loc_finditem_1:\n        LDA.A $2EDF"));
        assert!(!listing.contains("ROL $AD02"));
    }

    #[test]
    fn listing_marks_inline_string_literals_as_data() {
        let source = "PROC Main()\n  PrintE(\"Hello, world!\")\nRETURN\n";
        let tokens = tokenize(source).unwrap();
        let program = crate::parser::parse(&tokens).unwrap();
        let output = generate_profile_with_origin(&program, 0x3000, CodegenProfile::Compat)
            .expect("generate hello-world");

        let listing = format_listing_with_source(&output, source);

        assert!(
            listing.contains("; 2:3 storage inline string literal | PrintE(\"Hello, world!\")")
        );
        assert!(listing.contains("; ===== DATA inline string literal $3006 ====="));
        assert!(listing.contains(".BYTE $0D,$48,$65,$6C,$6C,$6F,$2C,$20"));
        assert!(listing.contains("; $3006: 0D 48 65 6C 6C 6F 2C 20"));
        assert!(listing.contains(".BYTE $77,$6F,$72,$6C,$64,$21"));
        assert!(listing.contains("; $300E: 77 6F 72 6C 64 21"));
        assert!(listing.contains("ORG $02E2\n        DTA A(proc_main)"));
        assert!(!listing.contains("ORA $6548"));
        assert!(!listing.contains("JMP ($6F6C)"));
        assert!(!listing.contains("BIT $7720"));
    }
}
