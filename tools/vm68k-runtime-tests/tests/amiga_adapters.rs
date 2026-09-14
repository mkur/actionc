mod common;
use actionc::{
    compiler::native::{
        NativeCompileOptions, compile_file, prepare_file,
        runtime::{NativeRuntime, bind},
    },
    mir68k::{
        amiga::{Argument, LibraryCall, library_adapter},
        encode,
        machine::{Target, Width},
    },
};
use actionc_vm68k_tests::{
    Machine,
    amiga::{self, Call, Os},
};

fn install_adapter(vm: &mut Machine, address: u32, call: LibraryCall, args: &[Argument]) {
    let instructions = library_adapter(call, args).unwrap();
    let mut bytes = Vec::new();
    for instruction in instructions {
        bytes.extend(
            encode::encode_at(
                &instruction,
                address + bytes.len() as u32,
                &|a| match a.target {
                    Target::Absolute(value) => Ok(value),
                    _ => Err("unexpected relocation".into()),
                },
            )
            .unwrap(),
        );
    }
    bytes.extend([0x4e, 0x71, 0x4e, 0x71]);
    vm.cpu.mem.map(address, &bytes, false, true).unwrap();
}

#[test]
fn service_binding_uses_ids_and_rejects_missing_or_incompatible_interfaces() {
    let source = common::Source::new(
        "PROC Main()\nPut(65) PutE() Print(\"a\") PrintE(\"b\")\nPrintB(1) PrintBE(2) PrintC(3) PrintCE(4) PrintI(-5) PrintIE(-6)\nRETURN\n",
    );
    let program = prepare_file(&source.0, &Default::default()).unwrap();
    let bindings = bind(&program.mir, NativeRuntime::AmigaDos).unwrap();
    let external: Vec<_> = program
        .mir
        .routines
        .iter()
        .filter(|r| r.entry.external)
        .collect();
    assert_eq!(external.len(), 10);
    for r in &external {
        assert!(bindings.console(r.id).is_some());
    }
    assert!(
        bind(&program.mir, NativeRuntime::Bare)
            .unwrap_err()
            .contains("bare runtime")
    );
    let mut renamed = program.mir.clone();
    for r in &mut renamed.routines {
        r.name = format!("display {}", r.id.0);
    }
    let rebound = bind(&renamed, NativeRuntime::AmigaDos).unwrap();
    for r in &external {
        assert_eq!(bindings.console(r.id), rebound.console(r.id));
    }
    let mut bad = program.mir.clone();
    let r = bad
        .routines
        .iter_mut()
        .find(|r| r.entry.external && r.params.len() == 1)
        .unwrap();
    r.signature.variadic = Some(r.signature.params[0].clone());
    assert!(
        bind(&bad, NativeRuntime::AmigaDos)
            .unwrap_err()
            .contains("incompatible callable signature")
    );
    let source = common::Source::new("PROC Main() Graphics(0) RETURN\n");
    let program = prepare_file(&source.0, &Default::default()).unwrap();
    assert!(
        bind(&program.mir, NativeRuntime::AmigaDos)
            .unwrap_err()
            .contains("does not support")
    );
}

#[test]
fn compiled_calls_through_os_adapters_preserve_frames_and_live_values() {
    let source = common::Source::new(
        r#"
BYTE ARRAY libName=[100 111 115 46 108 105 98 114 97 114 121 0]
BYTE ARRAY text=[65 0 255]
BYTE POINTER exec,dos
LONGCARD seed,accumulator,handle
LONGINT wrote
BYTE POINTER FUNC POINTER opener(BYTE POINTER base,name BYTE version)
LONGCARD FUNC POINTER outputter(BYTE POINTER base)
LONGINT FUNC POINTER writer(BYTE POINTER base CARD handle BYTE POINTER buffer BYTE count)
PROC POINTER closer(BYTE POINTER base,library)
PROC Main()
  BYTE i
  LONGCARD a,b,c,d,e
  a=seed+1 b=seed+2 c=seed+3 d=seed+4 e=seed+5
  dos=opener(exec,@libName(0),40)
  handle=outputter(dos)
  FOR i=0 TO 2 DO
    wrote=writer(dos,CARD(handle),@text(i),1)
    a==+LONGCARD(wrote) b==+1 c==+2 d==+3 e==+4
  OD
  closer(exec,dos)
  accumulator=a+b+c+d+e
RETURN
"#,
    );
    for (optimize, codegen) in [
        (false, Default::default()),
        (true, Default::default()),
        (true, actionc::mir68k::materialize::Options::conservative()),
    ] {
        let image = compile_file(
            &source.0,
            &NativeCompileOptions {
                optimize,
                codegen,
                ..Default::default()
            },
        )
        .unwrap()
        .image;
        let mut vm = Machine::from_image(&image).unwrap();
        let mut os = Os::default();
        os.install(&mut vm).unwrap();
        let byte = Argument {
            width: Width::Byte,
            signed: false,
        };
        let word = Argument {
            width: Width::Word,
            signed: false,
        };
        for (name, address, call, args) in [
            (
                "opener",
                0x5000,
                LibraryCall::OpenLibrary,
                vec![Argument::LONG, byte],
            ),
            ("outputter", 0x5100, LibraryCall::Output, vec![]),
            (
                "writer",
                0x5200,
                LibraryCall::Write,
                vec![word, Argument::LONG, byte],
            ),
            (
                "closer",
                0x5300,
                LibraryCall::CloseLibrary,
                vec![Argument::LONG],
            ),
        ] {
            install_adapter(&mut vm, address, call, &args);
            vm.write_scalar(image.symbol(name).unwrap(), address)
                .unwrap();
        }
        vm.write_scalar(image.symbol("exec").unwrap(), amiga::EXEC_BASE)
            .unwrap();
        vm.write_scalar(image.symbol("seed").unwrap(), 0x12345678)
            .unwrap();
        os.run(&mut vm, 10000).assert_completed();
        assert_eq!(os.output, [65, 0, 255]);
        assert_eq!((os.open_count, os.close_count), (1, 1));
        assert_eq!(
            os.events.iter().map(|e| e.call).collect::<Vec<_>>(),
            [
                Call::OpenLibrary,
                Call::Output,
                Call::Write,
                Call::Write,
                Call::Write,
                Call::CloseLibrary
            ]
        );
        assert_eq!(os.events[0].args[1], 40);
        assert_eq!(
            vm.read_scalar(image.symbol("dos").unwrap()).unwrap(),
            amiga::DOS_BASE
        );
        assert_eq!(
            vm.read_scalar(image.symbol("handle").unwrap()).unwrap(),
            amiga::OUTPUT_HANDLE
        );
        assert_eq!(
            vm.read_scalar(image.symbol("accumulator").unwrap())
                .unwrap(),
            0x12345678u32.wrapping_mul(5) + 48
        );
    }
}

#[test]
fn signed_word_adapter_extends_the_captured_value_before_the_foreign_call() {
    let source = common::Source::new(
        r#"
BYTE POINTER base,buffer
LONGINT result
LONGINT FUNC POINTER writer(BYTE POINTER base LONGCARD handle BYTE POINTER buffer INT count)
PROC Main()
  result=writer(base,257,buffer,-32768)
RETURN
"#,
    );
    let image = compile_file(&source.0, &Default::default()).unwrap().image;
    let mut vm = Machine::from_image(&image).unwrap();
    Os::default().install(&mut vm).unwrap();
    install_adapter(
        &mut vm,
        0x5200,
        LibraryCall::Write,
        &[
            Argument::LONG,
            Argument::LONG,
            Argument {
                width: Width::Word,
                signed: true,
            },
        ],
    );
    for (name, value) in [
        ("base", amiga::DOS_BASE),
        ("buffer", 0x123456),
        ("writer", 0x5200),
    ] {
        vm.write_scalar(image.symbol(name).unwrap(), value).unwrap();
    }
    let mut seen = false;
    vm.run_with_traps(1000, &mut |pc, vector, cpu| {
        if pc != amiga::DOS_BASE - 48 || vector != 45 {
            return Ok(false);
        }
        assert_eq!(
            [cpu.dar[1], cpu.dar[2], cpu.dar[3]],
            [257, 0x123456, 0xffff8000]
        );
        cpu.dar[0] = 0xffff_ffff;
        cpu.dar[1] = 0xdead_beef;
        cpu.dar[8] = 0xdead_beef;
        cpu.dar[9] = 0xdead_beef;
        cpu.ccr_to_flags(31);
        seen = true;
        Ok(true)
    })
    .assert_completed();
    assert!(seen);
    assert_eq!(
        vm.read_scalar(image.symbol("result").unwrap()).unwrap(),
        u32::MAX
    );
    assert!(library_adapter(LibraryCall::Write, &[]).is_err());
    assert!(
        library_adapter(
            LibraryCall::OpenLibrary,
            &[
                Argument {
                    width: Width::Word,
                    signed: false
                },
                Argument::LONG
            ]
        )
        .is_err()
    );
}
