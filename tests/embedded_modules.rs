use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use actionc::ast::{FundType, SourceUnitKind, Visibility};
use actionc::compiler::{CompileMode, CompileOptions, Runtime, compile_file};
use actionc::includes::{ModuleLoadOptions, load_compilation};
use actionc::semantic::ir::{self, SemExprKind, SemItem, SemLiteral};
use actionc::semantic::{SymbolClass, SymbolId, ValueTypeBase, analyze_compilation};

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "actionc-embedded-modules-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create embedded-module test directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn source(&self, name: &str, source: &str) -> PathBuf {
        let path = self.path().join(name);
        fs::write(&path, source).expect("write Action source");
        path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn hardware_source(temp: &TestDir) -> PathBuf {
    temp.source(
        "hardware.act",
        r#"MODULE HARDWARE.TEST
USE ATARI.ANTIC
USE ATARI.GTIA
USE ATARI.OS
USE ATARI.POKEY
USE ATARI.PIA
USE ATARI.VBXE

BYTE sink

PROC Main()
  sink=ANTIC.VCOUNT
  ANTIC.DMACTL=sink
  GTIA.COLBAK=sink
  OS.SDLST=$3456
  POKEY.AUDF1=sink
  PIA.PORTA=sink
  sink=VBXE.D6_REGS(VBXE.REG_CORE_REVISION)
RETURN
ENDMODULE
"#,
    )
}

fn count_instruction(bytes: &[u8], instruction: &[u8]) -> usize {
    bytes
        .windows(instruction.len())
        .filter(|candidate| *candidate == instruction)
        .count()
}

#[test]
fn embedded_atari_registers_compile_with_exact_volatile_accesses_in_all_modes() {
    let temp = TestDir::new();
    let source = hardware_source(&temp);

    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        let compiled = compile_file(&source, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|error| panic!("compile embedded modules in {mode:?}: {error}"));
        let bytes = compiled.object_bytes();

        for (name, instruction) in [
            ("ANTIC.VCOUNT read", [0xAD, 0x0B, 0xD4]),
            ("ANTIC.DMACTL write", [0x8D, 0x00, 0xD4]),
            ("GTIA.COLBAK write", [0x8D, 0x1A, 0xD0]),
            ("POKEY.AUDF1 write", [0x8D, 0x00, 0xD2]),
            ("PIA.PORTA write", [0x8D, 0x00, 0xD3]),
        ] {
            assert_eq!(
                count_instruction(bytes, &instruction),
                1,
                "{mode:?} must issue exactly one {name}: {bytes:02X?}"
            );
        }
        assert!(
            bytes.windows(3).any(|bytes| bytes == [0x8D, 0x30, 0x02]),
            "{mode:?} must write the low byte of OS.SDLST"
        );
        assert!(
            bytes.windows(3).any(|bytes| bytes == [0x8D, 0x31, 0x02]),
            "{mode:?} must write the high byte of OS.SDLST"
        );
        assert!(
            count_instruction(bytes, &[0xAD, 0x40, 0xD6]) >= 1,
            "{mode:?} must read VBXE's D6 core revision register"
        );
    }
}

#[test]
fn module_examples_compile_standalone_with_both_backends() {
    let samples = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("samples")
        .join("modules");

    for file_name in [
        "rainbow.act",
        "sys-memory-qualified.act",
        "sys-memory-open.act",
        "local-runtime-override.act",
        "native-real-library.act",
        "project/main.act",
    ] {
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            let options = CompileOptions::for_mode(mode).with_runtime(Runtime::Standalone);
            compile_file(samples.join(file_name), &options).unwrap_or_else(|error| {
                panic!("compile standalone {file_name} in {mode:?}: {error}")
            });
        }
    }
}

#[test]
fn vbxe_detection_sample_compiles_standalone_with_both_backends() {
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/vbxe/detect.act");

    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        let options = CompileOptions::for_mode(mode).with_runtime(Runtime::Standalone);
        compile_file(&sample, &options)
            .unwrap_or_else(|error| panic!("compile VBXE detection sample in {mode:?}: {error}"));
    }
}

#[test]
fn vbxe_gradient_sample_compiles_standalone_with_both_backends() {
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/vbxe/gradient.act");

    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        let options = CompileOptions::for_mode(mode).with_runtime(Runtime::Standalone);
        compile_file(&sample, &options)
            .unwrap_or_else(|error| panic!("compile VBXE gradient sample in {mode:?}: {error}"));
    }
}

#[test]
fn native_real_square_root_compiles_with_both_backends() {
    let temp = TestDir::new();
    let source = temp.source(
        "real-square-root.act",
        r#"MODULE REAL_SQUARE_ROOT_TEST
USE ATARI.REAL AS FPP

REAL input,result

PROC Main()
  input=0
  FPP.Sqrt(@input,@result)
  input=1
  FPP.Sqrt(@input,@result)
  input=2
  FPP.Sqrt(@input,@result)
  input=4
  FPP.Sqrt(@input,@result)
  input=0.0001
  FPP.Sqrt(@input,@result)
  input=10000
  FPP.Sqrt(@input,@result)
RETURN

ENDMODULE
"#,
    );

    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        compile_file(&source, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|error| panic!("compile native REAL square root in {mode:?}: {error}"));
    }
}

#[test]
fn classic_standalone_keeps_the_named_root_main_as_the_run_address() {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/modules/native-real-library.act");
    let compiled = compile_file(
        &source,
        &CompileOptions::for_mode(CompileMode::Optimized).with_runtime(Runtime::Standalone),
    )
    .expect("compile named standalone REAL library sample");
    let listing = compiled.source_listing();
    let main_header = listing
        .lines()
        .find(|line| line.contains("PROC M_NATIVE_REAL_LIBRARY_MAIN_") && line.contains(" entry $"))
        .expect("named Main listing header");
    let entry = main_header
        .split(" entry $")
        .nth(1)
        .and_then(|tail| tail.split_whitespace().next())
        .and_then(|hex| u16::from_str_radix(hex, 16).ok())
        .expect("named Main entry address");

    assert_eq!(compiled.run_address(), entry);
}

#[test]
fn sys_misc_routines_compile_standalone_with_both_backends() {
    let temp = TestDir::new();
    let source = temp.source(
        "sys-misc.act",
        r#"MODULE SYS_MISC_TEST
USE SYS

BYTE byteValue
CARD cardValue

PROC Main()
  byteValue=SYS.Rand(10)
  SYS.Sound(0,0,0,0)
  SYS.SndRst()
  byteValue=SYS.Paddle(0)
  byteValue=SYS.PTrig(0)
  byteValue=SYS.Stick(0)
  byteValue=SYS.STrig(0)
  byteValue=SYS.Peek($D20A)
  cardValue=SYS.PeekC($D20A)
  SYS.Poke($D01A,0)
  SYS.PokeC($0230,$4000)
RETURN

ENDMODULE
"#,
    );

    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        let options = CompileOptions::for_mode(mode).with_runtime(Runtime::Standalone);
        compile_file(&source, &options)
            .unwrap_or_else(|error| panic!("compile standalone SYS misc in {mode:?}: {error}"));
    }
}

#[test]
fn sys_graphics_routines_link_cross_unit_standalone_dependencies() {
    let temp = TestDir::new();
    let source = temp.source(
        "sys-graphics.act",
        r#"MODULE SYS_GRAPHICS_TEST
USE SYS

BYTE pixel

PROC Main()
  SYS.Graphics(8)
  SYS.Position(10,20)
  SYS.DrawTo(30,40)
  pixel=SYS.Locate(10,20)
  SYS.Plot(10,20)
  SYS.SetColor(0,8,6)
  SYS.Fill(30,40)
RETURN

ENDMODULE
"#,
    );

    for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
        let options = CompileOptions::for_mode(mode).with_runtime(Runtime::Standalone);
        compile_file(&source, &options)
            .unwrap_or_else(|error| panic!("compile standalone SYS graphics in {mode:?}: {error}"));
    }
}

#[test]
fn sys_io_routines_compile_with_both_runtimes_and_backends() {
    let temp = TestDir::new();
    let source = temp.source(
        "sys-io.act",
        r#"MODULE SYS_IO_TEST
USE SYS

STRING fileName(0)="E:"
STRING text(0)="TEST"
STRING input(40)
CHAR value
CARD sector
BYTE offset
BYTE byteValue
CARD cardValue
INT intValue

PROC Main()
  SYS.Put('A)
  SYS.PutE()
  SYS.PutD(0,'A)
  SYS.PutDE(0)
  SYS.Print(text)
  SYS.PrintE(text)
  SYS.PrintD(0,text)
  SYS.PrintDE(0,text)
  SYS.PrintF(text,cardValue)
  PrintF(text,cardValue)
  SYS.PrintH(cardValue)
  PrintH(cardValue)
  value=SYS.GetD(0)
  SYS.InputS(input)
  SYS.InputSD(0,input)
  SYS.InputMD(0,input,39)
  SYS.InputD(0,input)
  InputD(0,input)
  SYS.Open(1,fileName,4,0)
  SYS.Close(1)
  SYS.XIO(1,0,0,0,0,text)
  SYS.Note(1,@sector,@offset)
  SYS.Point(1,sector,offset)
  SYS.PrintB(byteValue)
  SYS.PrintBE(byteValue)
  SYS.PrintBD(0,byteValue)
  SYS.PrintBDE(0,byteValue)
  PrintBDE(0,byteValue)
  SYS.PrintC(cardValue)
  SYS.PrintCE(cardValue)
  SYS.PrintCD(0,cardValue)
  SYS.PrintCDE(0,cardValue)
  SYS.PrintI(intValue)
  SYS.PrintIE(intValue)
  SYS.PrintID(0,intValue)
  SYS.PrintIDE(0,intValue)
  byteValue=SYS.InputB()
  byteValue=SYS.InputBD(0)
  cardValue=SYS.InputC()
  cardValue=SYS.InputCD(0)
  intValue=SYS.InputI()
  intValue=SYS.InputID(0)
  byteValue=SYS.ValB(text)
  cardValue=SYS.ValC(text)
  intValue=SYS.ValI(text)
RETURN

ENDMODULE
"#,
    );

    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
            let options = CompileOptions::for_mode(mode).with_runtime(runtime);
            compile_file(&source, &options)
                .unwrap_or_else(|error| panic!("compile {runtime:?} SYS I/O in {mode:?}: {error}"));
        }
    }
}

#[test]
fn legacy_inputd_and_printbde_use_the_verified_cartridge_entries() {
    let temp = TestDir::new();
    let source = temp.source(
        "legacy-inputd.act",
        r#"STRING input(40)

PROC Main()
  InputD(0,input)
  PrintBDE(0,1)
RETURN
"#,
    );

    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        let compiled = compile_file(&source, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|error| panic!("compile legacy cartridge I/O in {mode:?}: {error}"));
        let bytes = compiled.object_bytes();
        assert!(
            bytes.windows(3).any(|bytes| bytes == [0x20, 0xA7, 0xA4]),
            "{mode:?} must call the verified InputD entry at $A4A7"
        );
        assert!(
            bytes
                .windows(3)
                .any(|bytes| { matches!(bytes, [0x20 | 0x4C, 0x08, 0xA5]) }),
            "{mode:?} must call or tail-call the verified PrintBDE entry at $A508"
        );
    }
}

fn implicit_sys_source(named: bool) -> &'static str {
    if named {
        r#"MODULE IMPLICIT_SYS_TEST

BYTE ARRAY buffer(8)
BYTE ARRAY left="A",right="B"
STRING input(40)
BYTE byteValue
CARD cardValue
INT comparison
INT intValue

PROC Main()
  Zero(buffer,8)
  byteValue=Rand(10)
  comparison=SCompare(left,right)
  Position(10,20)
  InputD(0,input)
  PrintBDE(0,byteValue)
  PrintF(input,cardValue)
  PrintH(cardValue)
  StrB(byteValue,input)
  StrC(cardValue,input)
  StrI(intValue,input)
  Error(1)
  Break()
RETURN

ENDMODULE
"#
    } else {
        r#"BYTE ARRAY buffer(8)
BYTE ARRAY left="A",right="B"
STRING input(40)
BYTE byteValue
CARD cardValue
INT comparison
INT intValue

PROC Main()
  Zero(buffer,8)
  byteValue=Rand(10)
  comparison=SCompare(left,right)
  Position(10,20)
  InputD(0,input)
  PrintBDE(0,byteValue)
  PrintF(input,cardValue)
  PrintH(cardValue)
  StrB(byteValue,input)
  StrC(cardValue,input)
  StrI(intValue,input)
  Error(1)
  Break()
RETURN
"#
    }
}

#[test]
fn implicit_sys_compatibility_aliases_compile_in_all_backend_runtime_pairs() {
    let temp = TestDir::new();
    for (name, named) in [("legacy.act", false), ("named.act", true)] {
        let source = temp.source(name, implicit_sys_source(named));
        for runtime in [Runtime::ActionCart, Runtime::Standalone] {
            for mode in [CompileMode::Optimized, CompileMode::Mir6502] {
                let options = CompileOptions::for_mode(mode).with_runtime(runtime);
                let compiled = compile_file(&source, &options).unwrap_or_else(|error| {
                    panic!("compile implicit SYS source {name} in {mode:?}/{runtime:?}: {error}")
                });
                assert_eq!(compiled.runtime(), runtime);
                if named && runtime == Runtime::ActionCart && mode == CompileMode::Optimized {
                    for address in [
                        0x04CB, 0xA3CC, 0xA4A7, 0xA508, 0xA544, 0xA54C, 0xA55B, 0xA6AE, 0xA6F1,
                        0xA78A, 0xA7DA, 0xA864,
                    ] {
                        assert!(
                            compiled.object_bytes().windows(3).any(|bytes| {
                                matches!(bytes[0], 0x20 | 0x4C)
                                    && bytes[1] == (address & 0x00FF) as u8
                                    && bytes[2] == (address >> 8) as u8
                            }),
                            "named classic cart output calls or tail-calls resident address ${address:04X}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn implicit_sys_public_routines_share_the_compatibility_symbol_ids() {
    let temp = TestDir::new();
    for (name, named) in [("legacy.act", false), ("named.act", true)] {
        let source = temp.source(name, implicit_sys_source(named));
        let loaded = load_compilation(&source, &ModuleLoadOptions::default())
            .unwrap_or_else(|diagnostics| panic!("load implicit SYS source: {diagnostics:#?}"));
        let model = analyze_compilation(&loaded)
            .unwrap_or_else(|diagnostics| panic!("analyze implicit SYS source: {diagnostics:#?}"));
        let sys = loaded
            .modules
            .iter()
            .find(|module| {
                module
                    .declared_path
                    .as_ref()
                    .is_some_and(|path| path.canonical_name() == "sys")
            })
            .expect("implicit SYS module");
        let source_scope = model
            .module(loaded.root)
            .map(|module| module.scope)
            .unwrap_or_else(|| model.symbols.global_scope());
        let builtin_scope = model.symbols.builtin_scope().expect("builtin scope");
        let public_routines = model
            .symbols
            .symbols
            .iter()
            .enumerate()
            .filter_map(|(index, symbol)| {
                (symbol.defining_module == Some(sys.id)
                    && symbol.visibility == Visibility::Public
                    && matches!(symbol.class, SymbolClass::Proc | SymbolClass::Func))
                .then_some((SymbolId(index), symbol))
            })
            .collect::<Vec<_>>();

        assert_eq!(public_routines.len(), 71, "complete SYS compatibility API");
        for (symbol_id, symbol) in public_routines {
            let alias = model
                .symbols
                .resolve_action_name(source_scope, &symbol.name)
                .unwrap_or_else(|| panic!("compatibility alias for SYS.{}", symbol.name));
            assert_eq!(
                alias.id,
                model
                    .symbols
                    .lookup_exact(builtin_scope, &symbol.name)
                    .unwrap()
            );
            assert_eq!(alias.id, symbol_id);
        }
    }
}

#[test]
fn mir6502_machine_blocks_resolve_resident_routines_through_sys_bindings() {
    let temp = TestDir::new();
    let source = temp.source("machine-sys.act", "PROC Main() [$20Break] RETURN\n");
    let compiled = compile_file(&source, &CompileOptions::for_mode(CompileMode::Mir6502))
        .expect("compile machine reference to the implicit SYS interface");

    assert!(
        compiled
            .object_bytes()
            .windows(3)
            .any(|bytes| bytes == [0x20, 0xDA, 0xA7]),
        "MIR6502 must bind the structured Break relocation through sys-cart.act"
    );

    compile_file(
        &source,
        &CompileOptions::for_mode(CompileMode::Mir6502).with_runtime(Runtime::Standalone),
    )
    .expect("link machine reference to the standalone SYS implementation");
}

#[test]
fn embedded_atari_interfaces_preserve_addresses_types_visibility_and_origins() {
    let temp = TestDir::new();
    let source = hardware_source(&temp);
    let loaded = load_compilation(&source, &ModuleLoadOptions::default())
        .expect("load embedded Atari modules");
    let model = analyze_compilation(&loaded).expect("analyze embedded Atari modules");
    let semir = ir::lower_compilation(&loaded, &model);

    for path in [
        "ATARI.ANTIC",
        "ATARI.GTIA",
        "ATARI.OS",
        "ATARI.POKEY",
        "ATARI.PIA",
        "ATARI.VBXE",
    ] {
        let module = loaded
            .modules
            .iter()
            .find(|module| match &module.program.source_kind {
                SourceUnitKind::Named(declaration) => declaration.path.display_name() == path,
                SourceUnitKind::Legacy => false,
            })
            .unwrap_or_else(|| panic!("loaded module {path}"));
        assert_eq!(module.origin.to_string(), format!("<embedded:{path}>"));
    }

    for (qualified_name, address, fund_type) in [
        ("ATARI.ANTIC.DLIST", 0xD402, FundType::Card),
        ("ATARI.GTIA.COLBAK", 0xD01A, FundType::Byte),
        ("ATARI.GTIA.M0PF", 0xD000, FundType::Byte),
        ("ATARI.GTIA.M1PF", 0xD001, FundType::Byte),
        ("ATARI.GTIA.M2PF", 0xD002, FundType::Byte),
        ("ATARI.GTIA.M3PF", 0xD003, FundType::Byte),
        ("ATARI.GTIA.P0PF", 0xD004, FundType::Byte),
        ("ATARI.GTIA.P1PF", 0xD005, FundType::Byte),
        ("ATARI.GTIA.P2PF", 0xD006, FundType::Byte),
        ("ATARI.GTIA.P3PF", 0xD007, FundType::Byte),
        ("ATARI.GTIA.M0PL", 0xD008, FundType::Byte),
        ("ATARI.GTIA.M1PL", 0xD009, FundType::Byte),
        ("ATARI.GTIA.M2PL", 0xD00A, FundType::Byte),
        ("ATARI.GTIA.M3PL", 0xD00B, FundType::Byte),
        ("ATARI.GTIA.P0PL", 0xD00C, FundType::Byte),
        ("ATARI.GTIA.P1PL", 0xD00D, FundType::Byte),
        ("ATARI.GTIA.P2PL", 0xD00E, FundType::Byte),
        ("ATARI.GTIA.P3PL", 0xD00F, FundType::Byte),
        ("ATARI.GTIA.TRIG0", 0xD010, FundType::Byte),
        ("ATARI.GTIA.TRIG1", 0xD011, FundType::Byte),
        ("ATARI.GTIA.TRIG2", 0xD012, FundType::Byte),
        ("ATARI.GTIA.TRIG3", 0xD013, FundType::Byte),
        ("ATARI.GTIA.PAL", 0xD014, FundType::Byte),
        ("ATARI.OS.RTCLOK", 0x0012, FundType::Byte),
        ("ATARI.OS.RTCLOK_MID", 0x0013, FundType::Byte),
        ("ATARI.OS.RTCLOK_LO", 0x0014, FundType::Byte),
        ("ATARI.OS.ATRACT", 0x004D, FundType::Byte),
        ("ATARI.OS.LMARGIN", 0x0052, FundType::Byte),
        ("ATARI.OS.RMARGIN", 0x0053, FundType::Byte),
        ("ATARI.OS.ROWCRS", 0x0054, FundType::Byte),
        ("ATARI.OS.COLCRS", 0x0055, FundType::Card),
        ("ATARI.OS.DINDEX", 0x0057, FundType::Byte),
        ("ATARI.OS.SAVMSC", 0x0058, FundType::Card),
        ("ATARI.OS.RAMTOP", 0x006A, FundType::Byte),
        ("ATARI.OS.VDSLST", 0x0200, FundType::Card),
        ("ATARI.OS.SDMCTL", 0x022F, FundType::Byte),
        ("ATARI.OS.SDLST", 0x0230, FundType::Card),
        ("ATARI.OS.GPRIOR", 0x026F, FundType::Byte),
        ("ATARI.OS.TXTROW", 0x0290, FundType::Byte),
        ("ATARI.OS.TXTCOL", 0x0291, FundType::Card),
        ("ATARI.OS.TINDEX", 0x0293, FundType::Byte),
        ("ATARI.OS.TXTMSC", 0x0294, FundType::Card),
        ("ATARI.OS.PCOLR0", 0x02C0, FundType::Byte),
        ("ATARI.OS.PCOLR1", 0x02C1, FundType::Byte),
        ("ATARI.OS.PCOLR2", 0x02C2, FundType::Byte),
        ("ATARI.OS.PCOLR3", 0x02C3, FundType::Byte),
        ("ATARI.OS.COLOR0", 0x02C4, FundType::Byte),
        ("ATARI.OS.COLOR1", 0x02C5, FundType::Byte),
        ("ATARI.OS.COLOR2", 0x02C6, FundType::Byte),
        ("ATARI.OS.COLOR3", 0x02C7, FundType::Byte),
        ("ATARI.OS.COLOR4", 0x02C8, FundType::Byte),
        ("ATARI.OS.CRSINH", 0x02F0, FundType::Byte),
        ("ATARI.OS.CHACT", 0x02F3, FundType::Byte),
        ("ATARI.OS.CHBAS", 0x02F4, FundType::Byte),
        ("ATARI.OS.CH", 0x02FC, FundType::Byte),
        ("ATARI.POKEY.RANDOM", 0xD20A, FundType::Byte),
        ("ATARI.PIA.PORTA", 0xD300, FundType::Byte),
        ("ATARI.PIA.DDRA", 0xD300, FundType::Byte),
        ("ATARI.PIA.DDRB", 0xD301, FundType::Byte),
    ] {
        let symbol = model
            .symbols
            .symbols
            .iter()
            .find(|symbol| symbol.qualified_name == qualified_name)
            .unwrap_or_else(|| panic!("semantic symbol {qualified_name}"));
        assert_eq!(symbol.class, SymbolClass::Var);
        assert_eq!(symbol.visibility, Visibility::Public);
        assert!(symbol.is_volatile);
        assert!(matches!(
            symbol.ty.as_ref().map(|ty| &ty.base),
            Some(ValueTypeBase::Fund(actual)) if *actual == fund_type
        ));

        let declaration = semir
            .modules
            .iter()
            .flat_map(|module| &module.items)
            .find_map(|item| match item {
                SemItem::Declaration(declaration)
                    if declaration.symbol.qualified_name == qualified_name =>
                {
                    Some(declaration)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("SemIR declaration {qualified_name}"));
        assert!(matches!(
            declaration.initializer.as_ref().map(|initializer| &initializer.kind),
            Some(SemExprKind::Literal(SemLiteral::Number(number)))
                if number.value == Some(address)
        ));
    }

    for (qualified_name, value) in [
        ("ATARI.ANTIC.CHACTL_REFLECT", 0x04),
        ("ATARI.ANTIC.MODE_2", 0x02),
        ("ATARI.ANTIC.MODE_F", 0x0F),
        ("ATARI.ANTIC.DL_HSCROL", 0x10),
        ("ATARI.ANTIC.BLANK_1", 0x00),
        ("ATARI.ANTIC.BLANK_8", 0x70),
        ("ATARI.ANTIC.NMI_DLI", 0x80),
        ("ATARI.GTIA.MODE_9", 0x40),
        ("ATARI.GTIA.PRIOR_FIFTH_PLAYER", 0x10),
        ("ATARI.GTIA.LATCH_TRIGGERS", 0x04),
        ("ATARI.GTIA.COLLISION_3", 0x08),
        ("ATARI.GTIA.TV_PAL", 0x01),
        ("ATARI.GTIA.CONSOL_START", 0x01),
        ("ATARI.POKEY.AUDC_VOLUME_MASK", 0x0F),
        ("ATARI.POKEY.AUDCTL_POLY9", 0x80),
        ("ATARI.POKEY.IRQ_TIMER4", 0x04),
        ("ATARI.POKEY.SKCTL_NORMAL", 0x03),
        ("ATARI.POKEY.KBCODE_MASK", 0x3F),
        ("ATARI.PIA.CONTROL_PORT_ACCESS", 0x04),
        ("ATARI.PIA.CONTROL_C2_HIGH", 0x38),
        ("ATARI.VBXE.REG_CORE_REVISION", 0x00),
        ("ATARI.VBXE.REG_MEMAC_BANK_SEL", 0x1F),
        ("ATARI.VBXE.VIDEO_XDL_ENABLE", 0x01),
        ("ATARI.VBXE.XDLC_LOW_RES", 0x20),
        ("ATARI.VBXE.MEMAC_SIZE_16K", 0x02),
    ] {
        let symbol_id = model
            .symbols
            .symbols
            .iter()
            .position(|symbol| symbol.qualified_name == qualified_name)
            .map(SymbolId)
            .unwrap_or_else(|| panic!("semantic constant {qualified_name}"));
        let symbol = &model.symbols.symbols[symbol_id.0];
        assert_eq!(symbol.class, SymbolClass::Const);
        assert_eq!(symbol.visibility, Visibility::Public);
        assert!(!symbol.is_volatile);
        assert_eq!(model.constants[&symbol_id].bits, value, "{qualified_name}");
    }
}

#[test]
fn module_derived_listing_is_byte_identical_across_compilations() {
    let temp = TestDir::new();
    let source = hardware_source(&temp);
    let options = CompileOptions::for_mode(CompileMode::Optimized);
    let first = compile_file(&source, &options).expect("first module compilation");
    let second = compile_file(&source, &options).expect("second module compilation");

    assert_eq!(first.object_bytes(), second.object_bytes());
    assert_eq!(first.source_listing(), second.source_listing());
    assert!(
        first
            .source_listing()
            .contains("global_m_atari_antic_vcount_")
    );

    let emit_map = || {
        let output = Command::new(env!("CARGO_BIN_EXE_actionc-emit"))
            .args(["--profile", "modern", "--backend", "classic", "--emit-map"])
            .arg(&source)
            .output()
            .expect("emit module-derived map");
        assert!(
            output.status.success(),
            "map emission failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    };
    let first_map = emit_map();
    let second_map = emit_map();
    assert_eq!(first_map, second_map);
    assert!(
        String::from_utf8(first_map)
            .expect("map is UTF-8")
            .contains("M_HARDWARE_TEST_MAIN_")
    );
}

#[test]
fn loaded_user_modules_are_emitted_whole_in_all_modes() {
    let temp = TestDir::new();
    let library = temp.path().join("lib/util.act");
    fs::create_dir_all(library.parent().expect("library parent")).expect("create module directory");
    fs::write(
        &library,
        r#"MODULE LIB.UTIL
BYTE sink

PUBLIC PROC Used()
  sink=1
RETURN

PROC Unused()
  sink=2
RETURN

ENDMODULE
"#,
    )
    .expect("write user module");
    let source = temp.source(
        "app.act",
        r#"MODULE APP
USE LIB.UTIL

PROC Main()
  UTIL.Used()
RETURN

ENDMODULE
"#,
    );

    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        let compiled = compile_file(&source, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|error| panic!("compile whole user module in {mode:?}: {error}"));
        let listing = compiled.source_listing();
        assert!(
            listing.contains("M_LIB_UTIL_USED_"),
            "{mode:?} omitted the referenced user routine:\n{listing}"
        );
        assert!(
            listing.contains("M_LIB_UTIL_UNUSED_"),
            "{mode:?} selectively removed an unreferenced user routine:\n{listing}"
        );
    }
}

#[test]
fn copied_compiler_compiles_standalone_plasma_without_adjacent_support_files() {
    let temp = TestDir::new();
    let compiler_source = Path::new(env!("CARGO_BIN_EXE_actionc"));
    let compiler = temp.path().join(
        compiler_source
            .file_name()
            .expect("compiler executable has a file name"),
    );
    fs::copy(compiler_source, &compiler).expect("copy compiler executable");

    let sample_source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("samples")
        .join("demoscene")
        .join("plasma.act");
    let sample = temp.path().join("plasma.act");
    fs::copy(sample_source, &sample).expect("copy module sample");
    let output = temp.path().join("plasma.com");

    let result = Command::new(&compiler)
        .current_dir(temp.path())
        .args(["--mode", "mir6502", "--runtime", "standalone", "-o"])
        .arg(&output)
        .arg(&sample)
        .output()
        .expect("run copied compiler");
    assert!(
        result.status.success(),
        "copied compiler failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(output.exists());
    assert_eq!(
        fs::read_dir(temp.path())
            .expect("read copied compiler directory")
            .count(),
        3,
        "embedded modules must not be extracted beside the compiler"
    );
}
