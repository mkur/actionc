use super::*;
use crate::compiler::{
    native::{self, NativeCompileOptions, NativeCompiledProgram},
    settings::BackendSetting,
};

#[derive(Default)]
pub(super) struct Flags {
    pub backend: bool,
    pub bare: bool,
    pub amiga: bool,
    pub no_opt: bool,
    pub no_codegen_opt: bool,
}
impl Flags {
    #[allow(clippy::too_many_arguments)]
    pub fn resolve(
        &self,
        target: TargetId,
        profile: Option<CodegenProfile>,
        backend: Option<BackendSetting>,
        runtime_explicit: bool,
        mode: bool,
        origin: Option<u32>,
        module_paths: &[PathBuf],
    ) -> Option<NativeCompileOptions> {
        if target != TargetId::Motorola68000 {
            if self.backend
                || self.bare
                || self.amiga
                || self.no_opt
                || self.no_codegen_opt
                || backend == Some(BackendSetting::Mir68k)
            {
                configuration(
                    "--backend mir68k, --runtime bare/amiga, --no-opt and --no-codegen-opt require --target motorola-68000",
                );
            }
            return None;
        }
        if mode
            || profile == Some(CodegenProfile::Compat)
            || matches!(backend, Some(BackendSetting::Atari(_)))
            || (runtime_explicit && !self.bare && !self.amiga)
        {
            configuration(
                "Motorola 68000 requires modern semantics, MIR68K and runtime bare or amiga; omit --mode and Atari settings, or use --profile modern --backend mir68k --runtime bare",
            );
        }
        if self.amiga && origin.is_some() {
            configuration("--origin is invalid for Amiga; the OS assigns load addresses");
        }
        let mut options = NativeCompileOptions {
            module_paths: module_paths.to_vec(),
            optimize: !self.no_opt,
            ..Default::default()
        };
        if let Some(origin) = origin {
            options.origin = origin;
        }
        if self.no_codegen_opt {
            options.codegen = crate::mir68k::materialize::Options::conservative();
        }
        Some(options)
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    input: &str,
    options: &NativeCompileOptions,
    amiga: bool,
    outputs: Option<&CompileOutputs>,
    listing: bool,
    map: bool,
    diagnostic_byte_ranges: bool,
) {
    let written = if amiga {
        let program = native::amiga::compile_file(input, &options.into()).unwrap_or_else(|error| {
            print_compile_error(&error, diagnostic_byte_ranges);
            process::exit(if error.kind() == CompileErrorKind::Configuration {
                2
            } else {
                1
            });
        });
        if let Some(outputs) = outputs {
            native::amiga::write(
                &program,
                &outputs.object,
                outputs.listing.as_deref(),
                &[Path::new(input)],
            )
        } else {
            if listing {
                println!("{:#?}", program.machine);
            } else if map {
                println!(
                    "target motorola-68000\nruntime amiga\nentry {:?}",
                    program.executable.object.entry
                );
                for symbol in &program.executable.object.symbols {
                    println!(
                        "{} {:?} size={} alignment={} array={:?}",
                        symbol.name, symbol.location, symbol.size, symbol.alignment, symbol.array
                    );
                }
            } else {
                for (i, bytes) in program.executable.bytes.chunks(16).enumerate() {
                    print!("HUNK+{:08X}:", i * 16);
                    for byte in bytes {
                        print!(" {byte:02X}");
                    }
                    println!();
                }
            }
            Ok(())
        }
    } else {
        let program = compile(input, options, diagnostic_byte_ranges);
        if let Some(outputs) = outputs {
            native::artifacts::write(
                &program,
                &outputs.object,
                outputs.listing.as_deref(),
                &[Path::new(input)],
            )
        } else {
            emit(&program, listing, map);
            Ok(())
        }
    };
    if let Err(error) = written {
        eprintln!("failed to write native output: {error}");
        process::exit(1);
    }
}

pub(super) fn configuration(message: &str) -> ! {
    eprintln!("{message}");
    process::exit(2)
}

pub(super) fn compile(
    input: &str,
    options: &NativeCompileOptions,
    diagnostic_byte_ranges: bool,
) -> NativeCompiledProgram {
    native::compile_file(input, options).unwrap_or_else(|error| {
        print_compile_error(&error, diagnostic_byte_ranges);
        process::exit(if error.kind() == CompileErrorKind::Configuration {
            2
        } else {
            1
        });
    })
}

pub(super) fn emit(program: &NativeCompiledProgram, listing: bool, map: bool) {
    if listing {
        println!("{:#?}", program.machine);
    } else if map {
        println!("target motorola-68000\nentry ${:08X}", program.image.entry);
        for symbol in &program.image.symbols {
            println!(
                "{} {:?} size={} alignment={} array={:?}",
                symbol.name, symbol.location, symbol.size, symbol.alignment, symbol.array
            );
        }
    } else {
        for segment in &program.image.segments {
            println!(
                "segment ${:08X} size={} writable={} executable={}",
                segment.address,
                segment.bytes.len(),
                segment.writable,
                segment.executable
            );
            for (i, bytes) in segment.bytes.chunks(16).enumerate() {
                print!("${:08X}:", segment.address + i as u32 * 16);
                for byte in bytes {
                    print!(" {byte:02X}");
                }
                println!();
            }
        }
        for zero in &program.image.zero_fill {
            println!(
                "zero-fill ${:08X} size={} writable={}",
                zero.address, zero.size, zero.writable
            );
        }
    }
}
