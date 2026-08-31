use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use actionc::compiler::{CompileMode, CompileOptions, compile_file};

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn source(&self, name: &str, source: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, source).expect("write volatile test source");
        path
    }
}

impl Default for TestDir {
    fn default() -> Self {
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "actionc-volatile-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create volatile test directory");
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn count_instruction(bytes: &[u8], instruction: &[u8]) -> usize {
    bytes
        .windows(instruction.len())
        .filter(|candidate| *candidate == instruction)
        .count()
}

#[test]
fn classic_modes_keep_repeated_volatile_reads_writes_and_safe_compounds() {
    let temp = TestDir::default();
    let source = temp.source(
        "classic.act",
        r#"
VOLATILE BYTE VCOUNT=$D40B, COLBAK=$D01A
BYTE sink

PROC Main()
  sink=VCOUNT
  sink=VCOUNT
  COLBAK=0
  COLBAK=0
  COLBAK==+1
RETURN
"#,
    );

    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        let compiled = compile_file(&source, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|err| panic!("compile volatile source in {mode:?}: {err}"));
        let bytes = compiled.object_bytes();

        assert_eq!(
            count_instruction(bytes, &[0xAD, 0x0B, 0xD4]),
            2,
            "{mode:?} must issue both VCOUNT reads: {bytes:02X?}"
        );
        let colbak_stores = [0x8D, 0x8C, 0x8E]
            .into_iter()
            .map(|opcode| count_instruction(bytes, &[opcode, 0x1A, 0xD0]))
            .sum::<usize>();
        assert_eq!(
            colbak_stores, 3,
            "{mode:?} must issue every COLBAK write: {bytes:02X?}"
        );
        assert_eq!(
            count_instruction(bytes, &[0xEE, 0x1A, 0xD0]),
            0,
            "{mode:?} must not use INC's observable dummy write for volatile storage"
        );
    }
}

#[test]
fn classic_modes_preserve_volatility_inherited_by_an_alias() {
    let temp = TestDir::default();
    let source = temp.source(
        "alias.act",
        r#"
VOLATILE BYTE VCOUNT=$D40B
BYTE VCOUNT_ALIAS=VCOUNT
BYTE sink

PROC Main()
  sink=VCOUNT_ALIAS
  sink=VCOUNT_ALIAS
RETURN
"#,
    );

    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        let compiled = compile_file(&source, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|err| panic!("compile volatile alias in {mode:?}: {err}"));
        assert_eq!(
            count_instruction(compiled.object_bytes(), &[0xAD, 0x0B, 0xD4]),
            2,
            "{mode:?} must keep both reads through the volatile alias"
        );
    }
}

#[test]
fn classic_modes_keep_dynamic_volatile_array_accesses() {
    let temp = TestDir::default();
    let source = temp.source(
        "array.act",
        r#"
VOLATILE BYTE ARRAY POKEY(16)=$D200
BYTE sink, index

PROC Main()
  sink=POKEY(index)
  sink=POKEY(index)
  POKEY(index)=sink
  POKEY(index)=sink
RETURN
"#,
    );

    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        let compiled = compile_file(&source, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|err| panic!("compile volatile array in {mode:?}: {err}"));
        let bytes = compiled.object_bytes();
        let pokey_reads = [0xBD, 0xB9]
            .into_iter()
            .map(|opcode| count_instruction(bytes, &[opcode, 0x00, 0xD2]))
            .sum::<usize>();
        assert_eq!(
            pokey_reads, 2,
            "{mode:?} must issue both POKEY reads: {bytes:02X?}"
        );
        let pokey_writes = [0x9D, 0x99]
            .into_iter()
            .map(|opcode| count_instruction(bytes, &[opcode, 0x00, 0xD2]))
            .sum::<usize>();
        assert_eq!(
            pokey_writes, 2,
            "{mode:?} must issue both POKEY writes: {bytes:02X?}"
        );
    }
}

#[test]
fn mir6502_forwards_each_immediate_volatile_read_without_staging() {
    let temp = TestDir::default();
    let source = temp.source(
        "forward.act",
        r#"
VOLATILE BYTE VCOUNT=$D40B, RANDOM=$D20A
BYTE sink

PROC Main()
  IF VCOUNT<100 THEN
    sink=1
  FI
  sink=RANDOM&127
RETURN
"#,
    );

    let compiled = compile_file(&source, &CompileOptions::for_mode(CompileMode::Mir6502))
        .expect("compile immediate volatile consumers");
    let bytes = compiled.object_bytes();

    assert_eq!(count_instruction(bytes, &[0xAD, 0x0B, 0xD4]), 1);
    assert_eq!(count_instruction(bytes, &[0xAD, 0x0A, 0xD2]), 1);
    assert_eq!(
        count_instruction(bytes, &[0xAD, 0x0B, 0xD4, 0xC9, 100]),
        1,
        "VCOUNT must flow directly from LDA to CMP: {bytes:02X?}"
    );
    assert_eq!(
        count_instruction(bytes, &[0xAD, 0x0A, 0xD2, 0x29, 127]),
        1,
        "RANDOM must flow directly from LDA to AND: {bytes:02X?}"
    );
}

#[test]
fn plasma_uses_hardware_modules_in_every_public_mode() {
    let sample = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("samples")
        .join("demoscene")
        .join("plasma.act");
    let source = fs::read_to_string(&sample).expect("read plasma source");

    for expected in [
        "MODULE PLASMA",
        "USE ATARI.ANTIC",
        "USE ATARI.GTIA",
        "USE ATARI.OS",
        "ANTIC.VCOUNT",
        "GTIA.COLBAK",
        "OS.SDLST",
    ] {
        assert!(source.contains(expected), "missing `{expected}`");
    }
    assert!(!source.contains("VOLATILE CARD SDLSTL"));
    assert!(!source.contains("VOLATILE BYTE SDMCTL"));

    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        compile_file(&sample, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|err| panic!("compile plasma in {mode:?}: {err}"));
    }
}
