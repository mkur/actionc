use actionc::mir65816::image::Image;
use std::{
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

struct Directory(PathBuf);
impl Directory {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "actionc-65816-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("source.act"), source).unwrap();
        std::fs::write(root.join("layout.json"), r#"{"code_origin":98304,"data_origin":1179648,"stack_overflow":294912,"nmi_extra_stack":0,"imports":[]}"#).unwrap();
        Self(root)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_actionc-65816"))
            .current_dir(&self.0)
            .args(args)
            .arg("source.act")
            .output()
            .unwrap()
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn native_cli_writes_a_valid_image_and_preserves_inputs() {
    let dir = Directory::new("CARD value PROC Main() value=42 RETURN");
    let result = dir.run(&["--layout", "layout.json"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let image = Image::from_json(&std::fs::read(dir.0.join("source.a816.json")).unwrap()).unwrap();
    assert_eq!(image.abi, "action65816.native.v1");
    assert_eq!(image.routines.len(), 1);
    for input in ["source.act", "layout.json"] {
        let before = std::fs::read(dir.0.join(input)).unwrap();
        let result = dir.run(&["--layout", "layout.json", "-o", input]);
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("overwrite an input"));
        assert_eq!(std::fs::read(dir.0.join(input)).unwrap(), before);
    }
}

#[test]
fn unsupported_source_leaves_existing_output_intact() {
    for (source, message) in [
        ("CARD a,b,c PROC Main() c=a*b RETURN", "Mul"),
        (
            "VOLATILE BYTE io=$F00000 PROC Main() io=1 RETURN",
            "above bank zero",
        ),
    ] {
        let dir = Directory::new(source);
        let output = dir.0.join("existing.json");
        std::fs::write(&output, "sentinel").unwrap();
        let result = dir.run(&["--layout", "layout.json", "-o", "existing.json"]);
        assert!(!result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(std::fs::read_to_string(output).unwrap(), "sentinel");
    }
}

#[test]
fn native_cli_exports_interface_ids_and_the_physical_argument_layout() {
    let dir = Directory::new(
        "MODULE API PUBLIC EXTERNAL LONGINT FUNC Mixed(BYTE b CARD c BYTE POINTER p LONGINT n) PROC Main() Mixed(1,2,BYTE POINTER(0),3) RETURN ENDMODULE",
    );
    let result = dir.run(&["--emit-interfaces"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let interfaces: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(interfaces[0]["symbol"].is_u64());
    assert!(interfaces[0]["signature"].is_u64());
    assert_eq!(interfaces[0]["outgoing_bytes"], 13);
    assert_eq!(
        interfaces[0]["arguments"],
        serde_json::json!([
            {"offset":0,"size":1,"alignment":1}, {"offset":2,"size":2,"alignment":2},
            {"offset":4,"size":3,"alignment":1}, {"offset":8,"size":4,"alignment":2}
        ])
    );
}
