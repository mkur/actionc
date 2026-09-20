use actionc::mir65816::image::{Image, LinkOptions};
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
fn native_cli_hex_layout_matches_decimal_image() {
    let dir = Directory::new(
        "MODULE API PUBLIC EXTERNAL PROC Host() CARD output,initialized=[7] \
         CARD FUNC Local(CARD n) BYTE ARRAY values=[1 2 3] RETURN(n+CARD(values(1))) \
         PROC Main() Host() output=Local(initialized) RETURN ENDMODULE",
    );
    let result = dir.run(&["--emit-interfaces"]);
    assert!(result.status.success(), "{:?}", result);
    let interfaces: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let decimal = serde_json::json!({
        "code_origin": 0x018000,
        "data_origin": 0x120000,
        "read_only_origin": 0x340000,
        "zero_fill_origin": 0x560000,
        "stack_overflow": 0xffffff,
        "nmi_extra_stack": 0,
        "imports": [{
            "symbol": interfaces[0]["symbol"],
            "signature": interfaces[0]["signature"],
            "abi": interfaces[0]["abi"],
            "address": 0x04abcd,
            "size": 1,
            "stack_peak": 0,
            "checks_stack": true
        }]
    });
    let mut layouts = vec![decimal.clone()];
    for prefix in ["0x", "0X", "$"] {
        let mut hex = decimal.clone();
        for field in [
            "code_origin",
            "data_origin",
            "read_only_origin",
            "zero_fill_origin",
            "stack_overflow",
        ] {
            hex[field] = format!("{prefix}{:06X}", decimal[field].as_u64().unwrap()).into();
        }
        hex["imports"][0]["address"] = format!("{prefix}04abcd").into();
        layouts.push(hex);
    }
    let mut expected = None;
    for layout in layouts {
        std::fs::write(
            dir.0.join("layout.json"),
            serde_json::to_vec(&layout).unwrap(),
        )
        .unwrap();
        let result = dir.run(&["--layout", "layout.json", "--no-opt"]);
        assert!(result.status.success(), "{:?}", result);
        let bytes = std::fs::read(dir.0.join("source.a816.json")).unwrap();
        let image = Image::from_json(&bytes).unwrap();
        assert!(
            image
                .segments
                .iter()
                .any(|s| s.address == 0x018000 && s.executable)
        );
        assert_eq!(image.stack_overflow, 0xffffff);
        assert_eq!(image.imports[0].address, 0x04abcd);
        assert!(
            image
                .segments
                .iter()
                .any(|s| s.address == 0x120000 && s.writable)
        );
        assert!(
            image
                .segments
                .iter()
                .any(|s| s.address == 0x340000 && !s.writable)
        );
        assert!(image.zero_fill.iter().any(|s| s.address == 0x560000));
        if let Some(expected) = &expected {
            assert_eq!(&bytes, expected, "hex layout changed emitted image");
        } else {
            expected = Some(bytes);
        }
    }
}

#[test]
fn layout_optional_origins_preserve_defaults_and_numeric_serialization() {
    let mut layout = serde_json::json!({
        "code_origin": "0x018000", "data_origin": 1179648,
        "stack_overflow": "$048000", "nmi_extra_stack": 0, "imports": []
    });
    let missing: LinkOptions = serde_json::from_value(layout.clone()).unwrap();
    layout["read_only_origin"] = serde_json::Value::Null;
    layout["zero_fill_origin"] = serde_json::Value::Null;
    let explicit_null: LinkOptions = serde_json::from_value(layout).unwrap();
    assert!(missing.read_only_origin.is_none());
    assert!(missing.zero_fill_origin.is_none());
    let serialized = serde_json::to_value(&missing).unwrap();
    assert_eq!(serialized, serde_json::to_value(&explicit_null).unwrap());
    assert_eq!(serialized["code_origin"], 98304);
    assert_eq!(serialized["data_origin"], 1179648);
    assert_eq!(serialized["stack_overflow"], 294912);
}

#[test]
fn invalid_layout_addresses_leave_existing_output_intact() {
    let dir = Directory::new("PROC Main() RETURN");
    let base: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.0.join("layout.json")).unwrap()).unwrap();
    let output = dir.0.join("existing.json");
    std::fs::write(&output, "sentinel").unwrap();
    for value in serde_json::json!([
        "0x",
        "$",
        "0x12GG",
        "018000",
        "-0x1",
        "0x+1",
        "0x 1",
        "0x100000000",
        -1,
        1.5,
        4294967296_u64,
        true,
        null,
        "0x1000000"
    ])
    .as_array()
    .unwrap()
    {
        let mut layout = base.clone();
        layout["code_origin"] = value.clone();
        std::fs::write(
            dir.0.join("layout.json"),
            serde_json::to_vec(&layout).unwrap(),
        )
        .unwrap();
        let result = dir.run(&["--layout", "layout.json", "-o", "existing.json"]);
        assert!(!result.status.success(), "accepted {value}");
        let message = if value == "0x1000000" {
            "native link address exceeds 24 bits"
        } else {
            "invalid layout"
        };
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(message),
            "{:?}",
            result
        );
        assert_eq!(std::fs::read_to_string(&output).unwrap(), "sentinel");
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

#[test]
fn native_cli_keeps_comma_group_widths_before_contextual_types() {
    for newline in ["\n", "\r\n"] {
        let source = "MODULE API\nPUBLIC EXTERNAL LONGINT FUNC Mixed(CARD a,b LONGCARD c BYTE d,e LONGINT f)\nPROC Main() Mixed(1,2,LONGCARD($12345678),3,4,LONGINT(-70000)) RETURN\nENDMODULE".replace('\n', newline);
        let dir = Directory::new(&source);
        let result = dir.run(&["--emit-interfaces"]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let interfaces: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(interfaces[0]["outgoing_bytes"], 15);
        assert_eq!(
            interfaces[0]["arguments"],
            serde_json::json!([
                {"offset":0,"size":2,"alignment":2}, {"offset":2,"size":2,"alignment":2},
                {"offset":4,"size":4,"alignment":2}, {"offset":8,"size":1,"alignment":1},
                {"offset":9,"size":1,"alignment":1}, {"offset":10,"size":4,"alignment":2}
            ])
        );
        let local = Directory::new(
            &source
                .replace("PUBLIC EXTERNAL", "PUBLIC")
                .replace("LONGINT f)", "LONGINT f) RETURN(f)"),
        );
        for no_opt in [false, true] {
            let mut args = vec!["--layout", "layout.json"];
            if no_opt {
                args.push("--no-opt");
            }
            let result = local.run(&args);
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let image = Image::from_json(&std::fs::read(local.0.join("source.a816.json")).unwrap())
                .unwrap();
            let mixed = image
                .routines
                .iter()
                .find(|r| r.name.contains("MIXED"))
                .unwrap();
            assert_eq!(
                mixed.arguments.iter().map(|a| a.size).collect::<Vec<_>>(),
                [2, 2, 4, 1, 1, 4]
            );
        }
    }
}
