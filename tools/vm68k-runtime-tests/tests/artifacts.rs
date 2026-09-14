mod common;
use actionc::{
    compiler::native::{
        self, NativeCompileOptions,
        artifacts::{self, Identity, Location, NativeArtifact},
    },
    mir68k::image::{ImageView, SymbolView},
};
use actionc_vm68k_tests::Machine;
use std::{fs, path::Path};

const SOURCE: &str = "LONGINT result\nLONGCARD ARRAY table(3)=[1 2]\nLONGINT FUNC Sum(INT seed)\n VOLATILE INT local\n local=seed\nRETURN(LONGINT(local)+LONGINT(table(1)))\nPROC Entry() result=Sum(-300) RETURN\n";

fn exported(source: &common::Source) -> std::path::PathBuf {
    let program = native::compile_file(&source.0, &NativeCompileOptions::default()).unwrap();
    let path = source
        .0
        .parent()
        .unwrap()
        .join("bundle with spaces/image.json");
    artifacts::write(
        &program,
        &path,
        Some(&path.with_extension("txt")),
        &[&source.0],
    )
    .unwrap();
    path
}

#[test]
fn round_trip_runs_moved_sparse_images_and_preserves_symbol_properties() {
    for (optimize, origin, newline) in [(false, 0x10000, "\n"), (true, 0x23400, "\r\n")] {
        let source = common::Source::new(&SOURCE.replace('\n', newline));
        let mut program = native::compile_file(
            &source.0,
            &NativeCompileOptions {
                optimize,
                origin,
                ..Default::default()
            },
        )
        .unwrap();
        let result = program.image.symbol("result").unwrap().clone();
        let mut alias = result.clone();
        alias.id = actionc::mir68k::image::SymbolId::Data(actionc::mir68k::Mir68kDataId::Global(
            actionc::nir::SymbolId(90000),
        ));
        alias.name = "escaped \"name\" \\ and\nline".into();
        program.image.symbols.push(alias.clone());
        let dir = source.0.parent().unwrap().join("original bundle");
        let path = dir.join("image.json");
        artifacts::write(&program, &path, None, &[&source.0]).unwrap();
        let json = fs::read_to_string(&path).unwrap();
        fs::write(&path, json.replace('\n', newline)).unwrap();
        let moved = source.0.parent().unwrap().join("moved bundle");
        fs::rename(dir, &moved).unwrap();
        fs::remove_file(&source.0).unwrap();
        let loaded = NativeArtifact::load(moved.join("image.json")).unwrap();
        assert_eq!(loaded.entry(), program.image.entry);
        assert_eq!(loaded.segments, program.image.segments);
        assert_eq!(loaded.zero_fill(), program.image.zero_fill);
        for symbol in &program.image.symbols {
            assert!(
                loaded
                    .manifest
                    .symbols
                    .contains(&artifacts::ArtifactSymbol::from_symbol(symbol))
            );
        }
        let mut vm = Machine::from_image(&loaded).unwrap();
        assert_eq!(vm.read_scalar(loaded.symbol("result").unwrap()).unwrap(), 0);
        vm.run(10000).assert_completed();
        assert_eq!(
            vm.read_scalar(loaded.symbol(&alias.name).unwrap()).unwrap(),
            (-298i32) as u32
        );
        assert_eq!(
            vm.read_array(loaded.symbol("table").unwrap()).unwrap(),
            [1, 2, 0]
        );
        let frame = loaded
            .manifest
            .symbols
            .iter()
            .find(|s| matches!(s.location, Location::Frame { .. }))
            .unwrap();
        assert!(frame.address().is_err());
        assert!(vm.cpu.mem.write(loaded.entry(), &[0]).is_err());
        let table = loaded.symbol("table").unwrap();
        vm.cpu.mem.map(0x50000, &[0; 12], true, false).unwrap();
        vm.write_scalar(loaded.symbol("result").unwrap(), 9)
            .unwrap();
        vm.cpu
            .mem
            .write(table.address().unwrap(), &0x50000u32.to_be_bytes())
            .unwrap();
        vm.write_array(table, &[0x12345678, 0x80000000, 9]).unwrap();
        assert_eq!(vm.read_array(table).unwrap(), [0x12345678, 0x80000000, 9]);
        assert_eq!(
            vm.cpu.mem.bytes(0x50000, 4).unwrap(),
            [0x12, 0x34, 0x56, 0x78]
        );
    }
}

fn rejected(
    path: &Path,
    original: &serde_json::Value,
    change: impl FnOnce(&mut serde_json::Value),
) {
    let mut altered = original.clone();
    change(&mut altered);
    fs::write(path, serde_json::to_vec(&altered).unwrap()).unwrap();
    assert!(NativeArtifact::load(path).is_err(), "accepted {altered}");
}

#[test]
fn rejects_invalid_manifests_metadata_and_payloads_before_mapping() {
    let source = common::Source::new(SOURCE);
    let path = exported(&source);
    let original: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    use serde_json::json;
    for (field, value) in [
        ("version", json!(1)),
        ("target", json!("atari-6502")),
        ("endian", json!("little")),
        ("pointer_width", json!(2)),
        ("entry", json!(0)),
        ("entry", json!(0x10001)),
        ("unknown", json!(true)),
    ] {
        rejected(&path, &original, |j| j[field] = value);
    }
    for filename in [
        "../escape.bin",
        "/absolute.bin",
        "C:\\absolute.bin",
        "nested/payload.bin",
        "missing.bin",
    ] {
        rejected(&path, &original, |j| {
            j["segments"][0]["file"] = json!(filename)
        });
    }
    for (field, value) in [
        ("address", json!(u32::MAX)),
        ("size", json!(u32::MAX)),
        ("size", json!(0)),
        ("address", json!(0x10001)),
    ] {
        rejected(&path, &original, |j| j["segments"][0][field] = value);
    }
    rejected(&path, &original, |j| {
        let region = j["zero_fill"][0].clone();
        j["zero_fill"].as_array_mut().unwrap().push(region);
    });
    rejected(&path, &original, |j| {
        let symbol = j["symbols"][0].clone();
        j["symbols"].as_array_mut().unwrap().push(symbol);
    });
    for (field, value) in [
        ("alignment", json!(3)),
        ("size", json!(u32::MAX)),
        ("name", json!("")),
        (
            "location",
            json!({"kind":"frame","routine":999,"offset":-4}),
        ),
    ] {
        rejected(&path, &original, |j| j["symbols"][0][field] = value);
    }
    let table = original["symbols"]
        .as_array()
        .unwrap()
        .iter()
        .position(|s| s["name"] == "table")
        .unwrap();
    for (field, value) in [
        ("stride", json!(1)),
        ("count", json!(u32::MAX)),
        ("backing_address", json!(0xffffff)),
    ] {
        rejected(&path, &original, |j| {
            j["symbols"][table]["array"][field] = value
        });
    }
    fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    let file = path
        .parent()
        .unwrap()
        .join(original["segments"][0]["file"].as_str().unwrap());
    let bytes = fs::read(&file).unwrap();
    fs::write(&file, &bytes[..bytes.len() - 1]).unwrap();
    assert!(NativeArtifact::load(&path).is_err());
    fs::write(&file, [&bytes[..], &[0]].concat()).unwrap();
    assert!(NativeArtifact::load(&path).is_err());
}

#[test]
fn publishing_preserves_old_payloads_and_failures_preserve_completed_image() {
    let source = common::Source::new(SOURCE);
    let path = exported(&source);
    let old_json = fs::read(&path).unwrap();
    let old = NativeArtifact::load(&path).unwrap();
    let program = native::compile_file(&source.0, &NativeCompileOptions::default()).unwrap();
    assert!(artifacts::write(&program, &source.0, None, &[&source.0]).is_err());
    assert!(artifacts::write(&program, &path, Some(&path), &[]).is_err());
    let alias = path.parent().unwrap().join("new/../image.json");
    assert!(artifacts::write(&program, &path, Some(&alias), &[]).is_err());
    let occupied = source.0.parent().unwrap().join("occupied");
    fs::write(&occupied, b"leave this file").unwrap();
    assert!(artifacts::write(&program, &path, Some(&occupied.join("listing")), &[]).is_err());
    assert_eq!(fs::read(&path).unwrap(), old_json);
    assert_eq!(NativeArtifact::load(&path).unwrap().segments, old.segments);
    artifacts::write(&program, &path, None, &[]).unwrap();
    assert_ne!(fs::read(&path).unwrap(), old_json);
    for (spec, segment) in old.manifest.segments.iter().zip(&old.segments) {
        assert_eq!(
            fs::read(path.parent().unwrap().join(&spec.file)).unwrap(),
            segment.bytes
        );
    }
    let mut ambiguous = old.manifest.clone();
    let mut alias = ambiguous.symbol("result").unwrap().clone();
    alias.identity = Identity::Global { id: 90000 };
    ambiguous.symbols.push(alias);
    ambiguous.verify().unwrap();
    assert!(
        ambiguous
            .symbol("result")
            .unwrap_err()
            .contains("ambiguous")
    );
}

#[cfg(unix)]
#[test]
fn payload_symlinks_cannot_escape_bundle() {
    let source = common::Source::new(SOURCE);
    let path = exported(&source);
    let image = NativeArtifact::load(&path).unwrap();
    let payload = path
        .parent()
        .unwrap()
        .join(&image.manifest.segments[0].file);
    let outside = source.0.parent().unwrap().join("outside.bin");
    fs::rename(&payload, &outside).unwrap();
    std::os::unix::fs::symlink(outside, payload).unwrap();
    assert!(NativeArtifact::load(path).unwrap_err().contains("escapes"));
}
