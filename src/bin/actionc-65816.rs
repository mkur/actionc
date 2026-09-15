//! Initial freestanding native driver, separate from Atari/Amiga runtime policy.
use actionc::{
    compiler::native65816,
    includes::ModuleLoadOptions,
    mir65816::{self, Mir65816AbiHome},
};
use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut layout = None;
    let mut output = None;
    let mut input = None;
    let mut optimize = true;
    let mut interfaces = false;
    let mut modules = ModuleLoadOptions::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!(
                    "usage: actionc-65816 --layout <layout.json> [-o <image.a816.json>] [--no-opt] [--module-path <dir>] <source.act>\n       actionc-65816 --emit-interfaces <source.act>\n\nEmits a freestanding action65816.native.v1 scalar image. The platform supplies\nABI entry state, stack/direct-page domains and a raw stack-overflow adapter.\nLayout specifies code_origin, data_origin, stack_overflow, nmi_extra_stack and imports."
                );
                return Ok(());
            }
            "--layout" => {
                layout = Some(PathBuf::from(
                    args.next().ok_or("--layout requires a file")?,
                ))
            }
            "-o" => output = Some(PathBuf::from(args.next().ok_or("-o requires a file")?)),
            "--no-opt" => optimize = false,
            "--emit-interfaces" => interfaces = true,
            "--module-path" => modules.module_paths.push(PathBuf::from(
                args.next().ok_or("--module-path requires a directory")?,
            )),
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg}")),
            _ if input.is_none() => input = Some(PathBuf::from(arg)),
            _ => return Err("only one input source is accepted".into()),
        }
    }
    let input = input.ok_or("missing source; see actionc-65816 --help")?;
    let prepared =
        native65816::prepare_file(&input, optimize, &modules).map_err(|e| e.to_string())?;
    if interfaces {
        if layout.is_some() || output.is_some() {
            return Err("--emit-interfaces does not accept --layout or -o".into());
        }
        let declarations = prepared.mir.routines.iter().filter(|r| r.entry.external).map(|r| {
            let arguments = r.frame.parameters.iter().map(|p| {
                let Mir65816AbiHome::StackArgument { offset, size, alignment } = p.incoming else { unreachable!() };
                serde_json::json!({"offset": offset.get(), "size": size.get(), "alignment": alignment.get()})
            }).collect::<Vec<_>>();
            serde_json::json!({"name": r.name, "symbol": r.entry.external_symbol.map(|s| s.0), "signature": r.signature.0, "abi": mir65816::abi::generated::ABI_NAME, "arguments": arguments, "result": format!("{:?}", r.result_home), "outgoing_bytes": r.frame.incoming_extent.get()})
        }).collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&declarations).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    let layout_path = layout.ok_or("--layout is required: platform addresses must be explicit")?;
    let options = serde_json::from_slice(&std::fs::read(&layout_path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("invalid layout: {e}"))?;
    let program = prepared.compile(&options).map_err(|e| e.to_string())?;
    let output = output.unwrap_or_else(|| input.with_extension("a816.json"));
    native65816::write(&program, &output, &[&input, &layout_path])?;
    println!(
        "wrote {} ({} routines)",
        output.display(),
        program.image.routines.len()
    );
    Ok(())
}
