use actionc::compiler::native::{NativeCompileOptions, compile_file};
use actionc_vm68k_tests::Machine;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or(
        "usage: actionc-vm68k-tests SOURCE [--origin ADDRESS] [--no-opt] [--no-codegen-opt] [--budget INSTRUCTIONS] [--dump PREFIX]",
    )?;
    let mut options = NativeCompileOptions::default();
    let mut budget = 1_000_000;
    let mut dump = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--no-opt" => options.optimize = false,
            "--no-codegen-opt" => {
                options.codegen.forward_temporaries = false;
                options.codegen.select_instructions = false;
            }
            "--dump" => {
                dump = Some(std::path::PathBuf::from(
                    args.next().ok_or("--dump requires a path prefix")?,
                ))
            }
            "--origin" => {
                let value = args.next().ok_or("--origin requires an address")?;
                options.origin = if let Some(hex) = value.strip_prefix("0x") {
                    u32::from_str_radix(hex, 16)
                } else {
                    value.parse()
                }
                .map_err(|e| format!("invalid origin: {e}"))?;
            }
            "--budget" => {
                budget = args
                    .next()
                    .ok_or("--budget requires a count")?
                    .parse()
                    .map_err(|e| format!("invalid budget: {e}"))?
            }
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    let compiled = compile_file(path, &options).map_err(|e| format!("{e:#?}"))?;
    if let Some(prefix) = dump {
        let manifest = actionc_vm68k_tests::artifacts::dump(&compiled, &prefix)?;
        println!("Image manifest: {}", manifest.display());
    }
    let mut vm = Machine::from_image(&compiled.image)?;
    let result = vm.run(budget);
    if !matches!(result.outcome, actionc_vm68k_tests::Outcome::Completed) {
        return Err(format!("{result:#?}"));
    }
    println!(
        "Completed after {} instructions ({} cycles)",
        result.steps, result.cycles
    );
    for symbol in &compiled.image.symbols {
        if let Ok(value) = vm.read_scalar(symbol) {
            let signed = symbol
                .ty
                .as_ref()
                .and_then(|t| t.kind.integer())
                .is_some_and(|t| t.signed);
            let width = symbol.ty.as_ref().unwrap().width.unwrap().get();
            let number = if signed {
                match width {
                    1 => value as i8 as i64,
                    2 => value as i16 as i64,
                    _ => value as i32 as i64,
                }
            } else {
                i64::from(value)
            };
            println!(
                "{} @ {:08x} = {} (${value:08x})",
                symbol.name,
                symbol.address()?,
                number
            );
        }
    }
    Ok(())
}
