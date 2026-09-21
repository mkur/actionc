use actionc::mir65816::o65;
use std::{
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};
struct Directory(PathBuf);
impl Directory {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "actionc-o65-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        std::fs::write(p.join("main.act"), source).unwrap();
        std::fs::write(
            p.join("options.json"),
            serde_json::to_vec(&o65::Options::default()).unwrap(),
        )
        .unwrap();
        Self(p)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_actionc-65816"))
            .current_dir(&self.0)
            .args(args)
            .arg("main.act")
            .output()
            .unwrap()
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
const ARGS: &[&str] = &[
    "--format",
    "o65-experimental",
    "--o65-options",
    "options.json",
];
#[test]
fn o65_cli_emits_loadable_deterministic_files_for_both_modes_and_newlines() {
    for optimize in [false, true] {
        let mut expected = None;
        for newline in ["\n", "\r\n"] {
            let d = Directory::new(
                &"CARD output,initial=[7]\nPROC Main()\noutput=initial+1\nRETURN\n"
                    .replace('\n', newline),
            );
            let mut args = ARGS.to_vec();
            if !optimize {
                args.push("--no-opt");
            }
            let result = d.run(&args);
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let bytes = std::fs::read(d.0.join("main.o65")).unwrap();
            let profile = o65::inspect(&bytes).unwrap();
            assert_eq!(profile.imports.len(), 1);
            let placed = o65::relocate(
                &bytes,
                &o65::Placement {
                    bases: [0x10000, 0x120000, 0x130000],
                    allowed: vec![o65::Region {
                        address: 0x10000,
                        size: 0xff0000,
                    }],
                    reserved: vec![],
                    nmi_extra_stack: 0,
                    providers: vec![o65::Provider {
                        name: o65::profile::OVERFLOW.into(),
                        address: 0x48000,
                        size: 1,
                        contract: o65::profile::Contract::overflow(),
                    }],
                },
            )
            .unwrap();
            assert_eq!(placed.entry(), 0x10000);
            if let Some(e) = &expected {
                assert_eq!(&bytes, e);
            } else {
                expected = Some(bytes);
            }
        }
    }
}
#[test]
fn o65_cli_rejects_ambiguous_options_and_preserves_existing_files() {
    let d = Directory::new("PROC Main() RETURN");
    let mut args = ARGS.to_vec();
    args.extend(["-o", "output.o65"]);
    std::fs::write(d.0.join("output.o65"), b"keep").unwrap();
    for extra in [
        vec!["--layout", "unused.json"],
        vec!["--emit-interfaces"],
        vec!["--format", "json"],
        vec!["--format", "unknown"],
    ] {
        let mut a = args.clone();
        a.extend(extra);
        assert!(!d.run(&a).status.success());
        assert_eq!(std::fs::read(d.0.join("output.o65")).unwrap(), b"keep");
    }
    for target in ["main.act", "options.json"] {
        let before = std::fs::read(d.0.join(target)).unwrap();
        let mut a = ARGS.to_vec();
        a.extend(["-o", target]);
        assert!(!d.run(&a).status.success());
        assert_eq!(std::fs::read(d.0.join(target)).unwrap(), before);
    }
    std::fs::write(d.0.join("main.act"), "PROC Main() this is invalid RETURN").unwrap();
    assert!(!d.run(&args).status.success());
    assert_eq!(std::fs::read(d.0.join("output.o65")).unwrap(), b"keep");
}
#[test]
fn explicit_interface_ids_bind_names_and_full_physical_contracts() {
    let d = Directory::new(
        "MODULE API PUBLIC EXTERNAL LONGCARD FUNC Host(BYTE a CARD b BYTE POINTER p LONGCARD c) LONGCARD output PROC Main() output=Host(1,2,BYTE POINTER($120000),3) RETURN ENDMODULE",
    );
    let interface = d.run(&["--emit-interfaces"]);
    assert!(interface.status.success());
    let interface: serde_json::Value = serde_json::from_slice(&interface.stdout).unwrap();
    let options = serde_json::json!({"profile":o65::profile::ID,"nmi_extra_stack":0,"imports":[{"symbol":interface[0]["symbol"],"name":"Host","stack_peak":0,"checks_stack":true}]});
    std::fs::write(
        d.0.join("options.json"),
        serde_json::to_vec(&options).unwrap(),
    )
    .unwrap();
    let result = d.run(ARGS);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let profile = o65::inspect(&std::fs::read(d.0.join("main.o65")).unwrap()).unwrap();
    let host = profile.imports.iter().find(|i| i.name == "Host").unwrap();
    assert_eq!(host.contract.arguments.len(), 4);
    assert_eq!(host.contract.result, 4);
    assert_eq!(host.contract.domains, 3);
}
#[test]
fn module_inputs_are_protected_by_the_o65_publisher() {
    use actionc::{compiler::native65816, includes::ModuleLoadOptions};
    let d = Directory::new(
        "MODULE TEST\nUSE A816MEMORY\nBYTE ARRAY data(4)\nPROC Main()\nA816MEMORY.Clear(data,4)\nRETURN\nENDMODULE",
    );
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut modules = ModuleLoadOptions::default();
    modules.module_paths.push(root.join("runtime/65816"));
    let prepared = native65816::prepare_file(d.0.join("main.act"), false, &modules).unwrap();
    let compiled = prepared.compile_o65(&Default::default()).unwrap();
    let loaded = compiled
        .source_paths
        .iter()
        .find(|p| p.ends_with("a816memory.act") || p.ends_with("A816MEMORY.ACT"))
        .unwrap_or_else(|| {
            compiled
                .source_paths
                .iter()
                .find(|p| !p.ends_with("main.act"))
                .unwrap()
        });
    let before = std::fs::read(loaded).unwrap();
    assert!(native65816::write_o65(&compiled, loaded, &[]).is_err());
    assert_eq!(std::fs::read(loaded).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn canonical_input_aliases_cannot_be_overwritten() {
    let d = Directory::new("PROC Main() RETURN");
    std::os::unix::fs::symlink(d.0.join("main.act"), d.0.join("alias.o65")).unwrap();
    let before = std::fs::read(d.0.join("main.act")).unwrap();
    let mut args = ARGS.to_vec();
    args.extend(["-o", "alias.o65"]);
    assert!(!d.run(&args).status.success());
    assert_eq!(std::fs::read(d.0.join("main.act")).unwrap(), before);
}
