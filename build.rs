#[path = "build_support/embedded_image.rs"]
mod embedded_image;

use embedded_image::{SourceInput, prepare_image};
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=embedded/modules");
    println!("cargo:rerun-if-changed=embedded/bindings");
    println!("cargo:rerun-if-changed=corpora/action-runtime/extracted");

    let mut inputs = Vec::new();
    collect_modules(
        Path::new("embedded/modules"),
        Path::new("embedded/modules"),
        &mut inputs,
    );
    collect_bindings(Path::new("embedded/bindings"), &mut inputs);
    collect_runtime(Path::new("corpora/action-runtime/extracted"), &mut inputs);
    let image = prepare_image(inputs);
    let generated = render_image(&image);
    let output =
        PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is set")).join("embedded_vfs.rs");
    fs::write(output, generated).expect("write embedded VFS table");
}

fn collect_bindings(directory: &Path, inputs: &mut Vec<SourceInput>) {
    let mut paths = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
        .map(|entry| entry.expect("read embedded binding entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "act"))
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        println!("cargo:rerun-if-changed={}", path.display());
        let file_name = path
            .file_name()
            .expect("binding source has a file name")
            .to_string_lossy()
            .to_ascii_lowercase();
        inputs.push(SourceInput {
            kind: "Binding".to_string(),
            canonical_key: format!("binding:{file_name}"),
            virtual_path: format!("bindings/{file_name}"),
            display_name: format!("<binding:{}>", file_name.to_ascii_uppercase()),
            bytes: fs::read(&path).expect("read embedded binding source"),
        });
    }
}

fn collect_modules(root: &Path, directory: &Path, inputs: &mut Vec<SourceInput>) {
    let mut entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
        .map(|entry| entry.expect("read embedded module entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_modules(root, &path, inputs);
            continue;
        }
        if !path.extension().is_some_and(|extension| extension == "act") {
            continue;
        }
        println!("cargo:rerun-if-changed={}", path.display());
        let relative = path
            .strip_prefix(root)
            .expect("module remains below embedded root")
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase();
        let module_name = relative
            .strip_suffix(".act")
            .unwrap_or(&relative)
            .replace('/', ".")
            .to_ascii_uppercase();
        inputs.push(SourceInput {
            kind: "Module".to_string(),
            canonical_key: format!("module:{relative}"),
            virtual_path: relative,
            display_name: format!("<embedded:{module_name}>"),
            bytes: fs::read(&path).expect("read embedded module"),
        });
    }
}

fn collect_runtime(directory: &Path, inputs: &mut Vec<SourceInput>) {
    let mut paths = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
        .map(|entry| entry.expect("read runtime entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "ACT"))
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        println!("cargo:rerun-if-changed={}", path.display());
        let file_name = path
            .file_name()
            .expect("runtime source has a file name")
            .to_string_lossy();
        let virtual_path = format!("runtime/{}", file_name.to_ascii_lowercase());
        inputs.push(SourceInput {
            kind: "Runtime".to_string(),
            canonical_key: format!("runtime:{}", file_name.to_ascii_lowercase()),
            virtual_path,
            display_name: format!("<runtime:{}>", file_name.to_ascii_uppercase()),
            bytes: fs::read(&path).expect("read embedded runtime source"),
        });
    }
}

fn render_image(image: &embedded_image::PreparedImage) -> String {
    let mut output = format!("pub const VFS_DIGEST: &str = {:?};\n", image.sha256);
    output.push_str("pub static SOURCES: &[EmbeddedSource] = &[\n");
    for source in &image.sources {
        output.push_str("    EmbeddedSource {\n");
        output.push_str(&format!(
            "        kind: EmbeddedSourceKind::{},\n",
            source.input.kind
        ));
        output.push_str(&format!(
            "        canonical_key: {:?},\n        virtual_path: {:?},\n        display_name: {:?},\n        sha256: {:?},\n",
            source.input.canonical_key,
            source.input.virtual_path,
            source.input.display_name,
            source.sha256
        ));
        output.push_str("        bytes: &[");
        for (index, byte) in source.input.bytes.iter().enumerate() {
            if index % 24 == 0 {
                output.push_str("\n            ");
            }
            output.push_str(&format!("0x{byte:02X}, "));
        }
        output.push_str("\n        ],\n    },\n");
    }
    output.push_str("];\n");
    output
}
