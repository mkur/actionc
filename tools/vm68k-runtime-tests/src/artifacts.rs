//! Versioned JSON metadata and exact native segment bytes for inspection.
use actionc::{
    compiler::native::NativeCompiledProgram,
    mir68k::{
        Mir68kDataId,
        image::{SymbolId, SymbolLocation},
    },
};
use std::path::{Path, PathBuf};

pub fn dump(program: &NativeCompiledProgram, prefix: &Path) -> Result<PathBuf, String> {
    let directory = prefix
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let stem = prefix
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("dump prefix needs a UTF-8 file name")?;
    let image = &program.image;
    image.verify()?;
    let mut segments = Vec::new();
    for (index, segment) in image.segments.iter().enumerate() {
        let name = format!("{stem}.segment{index}.bin");
        std::fs::write(directory.join(&name), &segment.bytes).map_err(|e| e.to_string())?;
        segments.push(format!(
            "{{\"file\":{},\"address\":{},\"size\":{},\"writable\":{},\"executable\":{}}}",
            quoted(&name),
            segment.address,
            segment.bytes.len(),
            segment.writable,
            segment.executable
        ));
    }
    let zeros: Vec<_> = image
        .zero_fill
        .iter()
        .map(|z| {
            format!(
                "{{\"address\":{},\"size\":{},\"writable\":{}}}",
                z.address, z.size, z.writable
            )
        })
        .collect();
    let symbols: Vec<_> = image.symbols.iter().map(|s| {
        let identity = match s.id {
            SymbolId::Data(id) => match id {
                Mir68kDataId::Global(id) => format!("{{\"kind\":\"global\",\"id\":{}}}",id.0),
                Mir68kDataId::Static(id) => format!("{{\"kind\":\"static\",\"id\":{}}}",id.0),
                Mir68kDataId::ArrayBacking(id) => format!("{{\"kind\":\"array_backing\",\"owner\":{}}}",id.0),
                Mir68kDataId::Local(routine,id) => format!("{{\"kind\":\"static_local\",\"routine\":{},\"id\":{}}}",routine.0,id.0),
            },
            SymbolId::Routine(id) => format!("{{\"kind\":\"routine\",\"id\":{}}}",id.0),
            SymbolId::Automatic { routine, object } => format!("{{\"kind\":\"automatic\",\"routine\":{},\"object\":{}}}",routine.0,object.0),
            SymbolId::Parameter { routine, param } => format!("{{\"kind\":\"parameter\",\"routine\":{},\"id\":{}}}",routine.0,param.0),
        };
        let location = match s.location {
            SymbolLocation::Absolute(address) => format!("{{\"address\":{address}}}"),
            SymbolLocation::Frame { routine, offset } => format!("{{\"routine\":{},\"frame_offset\":{offset}}}",routine.0),
        };
        let ty = s.ty.as_ref().map(|t|format!("{{\"kind\":{},\"width\":{},\"signed\":{}}}",quoted(&format!("{:?}",t.kind)),optional(t.width.map(|w| w.get())),t.kind.integer().is_some_and(|t|t.signed))).unwrap_or("null".into());
        let array = s.array.as_ref().map(|a|format!("{{\"element_width\":{},\"stride\":{},\"count\":{},\"descriptor\":{},\"backing_address\":{}}}",a.element_width,a.stride,optional(a.count),a.descriptor,optional(a.backing_address))).unwrap_or("null".into());
        format!("{{\"name\":{},\"identity\":{},\"location\":{},\"size\":{},\"alignment\":{},\"type\":{},\"array\":{}}}",quoted(&s.name),identity,location,s.size,s.alignment,ty,array)
    }).collect();
    let listing = format!("{stem}.machine.txt");
    std::fs::write(
        directory.join(&listing),
        format!("{:#?}\n", program.machine),
    )
    .map_err(|e| e.to_string())?;
    let manifest = format!(
        "{{\n  \"version\":1,\n  \"target\":\"Motorola68000\",\n  \"endian\":\"big\",\n  \"pointer_width\":4,\n  \"link_address_bits\":24,\n  \"entry\":{},\n  \"machine_listing\":{},\n  \"segments\":[{}],\n  \"zero_fill\":[{}],\n  \"symbols\":[{}]\n}}\n",
        image.entry,
        quoted(&listing),
        segments.join(","),
        zeros.join(","),
        symbols.join(",")
    );
    let path = directory.join(format!("{stem}.json"));
    std::fs::write(&path, manifest).map_err(|e| e.to_string())?;
    Ok(path)
}
fn optional(value: Option<u32>) -> String {
    value.map(|v| v.to_string()).unwrap_or("null".into())
}
fn quoted(text: &str) -> String {
    let mut result = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\u{0000}'..='\u{001f}' => result.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => result.push(ch),
        }
    }
    result.push('"');
    result
}
