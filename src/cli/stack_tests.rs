use super::{CliFlavor, run_main};
use crate::compiler::native::artifacts::NativeArtifact;
use std::{fs, path::PathBuf};

struct OutputDir(PathBuf);

impl Drop for OutputDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn dijkstra_compiles_and_emits_from_a_small_stack_caller() {
    let directory = OutputDir(
        std::env::temp_dir().join(format!("actionc-cli-small-stack-{}", std::process::id())),
    );
    fs::create_dir(&directory.0).unwrap();
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/runtime/tacle/dijkstra/dijkstra.act");

    // Exercise the same dispatch boundary as both public CLI entry points.
    // Without its worker, even the raw pipeline overflows a 1 MiB host stack.
    for optimize in [false, true] {
        for flavor in [CliFlavor::Compile, CliFlavor::Emit] {
            let output = directory.0.join(format!("dijkstra-{optimize}.json"));
            let mut args = vec!["--target".into(), "motorola-68000".into()];
            if !optimize {
                args.push("--no-opt".into());
            }
            match flavor {
                CliFlavor::Compile => {
                    args.push("--output".into());
                    args.push(output.to_str().unwrap().into());
                }
                CliFlavor::Emit => args.push("--emit-map".into()),
            }
            args.push(source.to_str().unwrap().into());
            std::thread::Builder::new()
                .stack_size(128 * 1024)
                .spawn(move || run_main(flavor, args))
                .unwrap()
                .join()
                .unwrap();
            if flavor == CliFlavor::Compile {
                let image = NativeArtifact::load(&output).unwrap();
                assert!(
                    image
                        .manifest
                        .symbols
                        .iter()
                        .any(|symbol| symbol.name == "Find")
                );
            }
        }
    }
}
