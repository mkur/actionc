use actionc::{
    compiler::native::{NativeCompileOptions, artifacts::NativeArtifact, compile_file},
    mir68k::image::{ImageView, SymbolView},
};
use actionc_vm68k_tests::Machine;

const USAGE: &str = "usage: actionc-vm68k-tests SOURCE [--origin ADDRESS] [--no-opt] [--no-codegen-opt] [--budget INSTRUCTIONS] [--dump PREFIX]\n       actionc-vm68k-tests --image MANIFEST [--budget INSTRUCTIONS]";

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let (mut source, mut image) = (None, None);
    let mut options = NativeCompileOptions::default();
    let mut budget = 1_000_000;
    let mut dump = None;
    let mut compile_options = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            "--image" => {
                if image.is_some() {
                    return Err("--image may be specified only once".into());
                }
                image = Some(args.next().ok_or("--image requires a manifest path")?);
            }
            "--no-opt" => {
                options.optimize = false;
                compile_options = true;
            }
            "--no-codegen-opt" => {
                options.codegen = actionc::mir68k::materialize::Options::conservative();
                compile_options = true;
            }
            "--dump" => {
                dump = Some(std::path::PathBuf::from(
                    args.next().ok_or("--dump requires a path prefix")?,
                ));
                compile_options = true;
            }
            "--origin" => {
                let value = args.next().ok_or("--origin requires an address")?;
                options.origin = if let Some(hex) = value
                    .strip_prefix("0x")
                    .or_else(|| value.strip_prefix("0X"))
                    .or_else(|| value.strip_prefix('$'))
                {
                    u32::from_str_radix(hex, 16)
                } else {
                    value.parse()
                }
                .map_err(|e| format!("invalid origin: {e}"))?;
                compile_options = true;
            }
            "--budget" => {
                budget = args
                    .next()
                    .ok_or("--budget requires a count")?
                    .parse()
                    .map_err(|e| format!("invalid budget: {e}"))?
            }
            _ if !arg.starts_with('-') && source.is_none() => source = Some(arg),
            _ => return Err(format!("unknown option or extra source: {arg}")),
        }
    }
    if let Some(path) = image {
        if source.is_some() || compile_options {
            return Err("--image accepts only --budget; source, origin, optimization and dump options require source mode".into());
        }
        let artifact = NativeArtifact::load(path)?;
        execute(&artifact, &artifact.manifest.symbols, budget)
    } else {
        let compiled = compile_file(source.ok_or(USAGE)?, &options).map_err(|e| e.to_string())?;
        if let Some(prefix) = dump {
            let manifest = actionc_vm68k_tests::artifacts::dump(&compiled, &prefix)?;
            println!("Image manifest: {}", manifest.display());
        }
        execute(&compiled.image, &compiled.image.symbols, budget)
    }
}

fn execute(image: &impl ImageView, symbols: &[impl SymbolView], budget: u64) -> Result<(), String> {
    let mut vm = Machine::from_image(image)?;
    let result = vm.run(budget);
    if !matches!(result.outcome, actionc_vm68k_tests::Outcome::Completed) {
        return Err(format!("{result:#?}"));
    }
    println!(
        "Completed after {} instructions ({} cycles)",
        result.steps, result.cycles
    );
    for symbol in symbols {
        if let Ok(value) = vm.read_scalar(symbol) {
            let (width, signed) = symbol.scalar_type().unwrap();
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
                symbol.name(),
                symbol.address()?,
                number
            );
        }
    }
    Ok(())
}
