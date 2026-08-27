use std::path::PathBuf;

fn main() {
    let mut args = std::env::args_os().skip(1);
    let Some(path) = args.next().map(PathBuf::from) else {
        eprintln!("usage: actionc-runtime-link-manifest <output-path>");
        std::process::exit(2);
    };
    if args.next().is_some() {
        eprintln!("usage: actionc-runtime-link-manifest <output-path>");
        std::process::exit(2);
    }
    let manifest = match actionc::mir6502::generate_embedded_sys_link_manifest() {
        Ok(manifest) => manifest,
        Err(diagnostics) => {
            for diagnostic in diagnostics {
                eprintln!("{}", diagnostic.message);
            }
            std::process::exit(1);
        }
    };
    std::fs::write(&path, manifest)
        .unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
}
