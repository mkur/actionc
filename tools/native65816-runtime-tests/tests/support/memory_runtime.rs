use super::*;
use actionc::mir65816::{abi, image::AssemblyImport};

pub const ORIGIN: u32 = 0x050000;

pub fn assembly() -> Assembly {
    assemble_artifact(
        &std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/65816/memory.s"),
        )
        .unwrap(),
        ORIGIN,
    )
}

pub fn bind(prepared: &native65816::Prepared, options: &mut LinkOptions, assembly: &Assembly) {
    for name in ["Move", "Clear", "Fill"] {
        let symbol = actionc::nir::runtime_symbol_id(&format!("A816MEMORY.{name}"));
        if let Some(routine) = prepared
            .mir
            .routines
            .iter()
            .find(|r| r.entry.external_symbol == Some(symbol))
        {
            let label = format!("a816_memory_{}", name.to_ascii_lowercase());
            let address = assembly.symbols[&label];
            options.imports.push(AssemblyImport {
                symbol: symbol.0,
                signature: routine.signature.0,
                abi: abi::generated::ABI_NAME.into(),
                address,
                size: assembly.symbols[&(label + "_end")] - address,
                stack_peak: 0,
                checks_stack: true,
                irq_effect: Default::default(),
            });
        }
    }
}

pub fn compile_memory(source: &str, optimize: bool) -> (Image, Assembly) {
    let assembly = assembly();
    let prepared = prepare(source, optimize);
    let mut options = layout();
    bind(&prepared, &mut options, &assembly);
    let compiled = prepared.compile(&options).unwrap();
    (
        Image::from_json(&compiled.image.to_json().unwrap()).unwrap(),
        assembly,
    )
}
