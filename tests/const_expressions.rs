use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use actionc::compiler::{CompileMode, CompileOptions, compile_file};

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("actionc-const-{}-{sequence}", std::process::id()));
        fs::create_dir_all(&path).expect("create CONST test directory");
        Self(path)
    }

    fn write(&self, name: &str, source: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, source).expect("write CONST test source");
        path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn constants_match_literal_output_in_every_public_mode() {
    let temp = TestDir::new();
    let with_constants = temp.write(
        "constants.act",
        r#"
CONST CODE_BASE=$3400, COUNT=4, FIRST=2, LAST=FIRST+COUNT-1
CONST EXTERNAL_ADDRESS=$E456
SET $E=CODE_BASE
SET $491=CODE_BASE
BYTE ARRAY values(COUNT)=[1 2 3 4]
BYTE result=FIRST

PROC External=EXTERNAL_ADDRESS()

PROC Main()
  CONST LOCAL=COUNT*2
  BYTE i

  result=LOCAL
  FOR i=FIRST TO LAST DO
    result==+1
  OD
  IF result=12 THEN
    result=COUNT
  FI
RETURN
"#,
    );
    let with_literals = temp.write(
        "literals.act",
        r#"
SET $E=$3400
SET $491=$3400
BYTE ARRAY values(4)=[1 2 3 4]
BYTE result=2

PROC External=$E456()

PROC Main()
  BYTE i

  result=8
  FOR i=2 TO 5 DO
    result==+1
  OD
  IF result=12 THEN
    result=4
  FI
RETURN
"#,
    );

    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        let constants = compile_file(&with_constants, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|error| panic!("compile CONST source in {mode:?}: {error}"));
        let literals = compile_file(&with_literals, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|error| panic!("compile literal source in {mode:?}: {error}"));

        assert_eq!(
            constants.object_bytes(),
            literals.object_bytes(),
            "CONST output differs in {mode:?}"
        );
    }
}

#[test]
fn inline_assembler_constants_match_numeric_operands_in_every_public_mode() {
    let temp = TestDir::new();
    let with_constants = temp.write(
        "asm-constants.act",
        r#"
CONST VALUE=$11, COLOR=$D01A, DISPLAY_LIST=$5000

PROC Main()
  CONST VALUE=$2A
ASM
  lda #VALUE
  sta COLOR
  lda #<DISPLAY_LIST
  sta $80
  lda #>DISPLAY_LIST
  sta $81
  lda #0
  sta DISPLAY_LIST+8
ENDASM
RETURN
"#,
    );
    let with_literals = temp.write(
        "asm-literals.act",
        r#"
PROC Main()
ASM
  lda #$2A
  sta $D01A
  lda #<$5000
  sta $80
  lda #>$5000
  sta $81
  lda #0
  sta $5000+8
ENDASM
RETURN
"#,
    );

    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        let constants = compile_file(&with_constants, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|error| panic!("compile inline ASM constants in {mode:?}: {error}"));
        let literals = compile_file(&with_literals, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|error| panic!("compile inline ASM literals in {mode:?}: {error}"));

        assert_eq!(
            constants.object_bytes(),
            literals.object_bytes(),
            "inline ASM CONST output differs in {mode:?}"
        );
    }
}
