//! Exact static payload and address relocation checks across NIR backends.
use actionc::{compiler::Runtime, mir68k, mir6502, mir65816, nir, target::TargetId};

#[test]
fn labeled_data_nir_fixture_preserves_the_contract_with_lf_and_crlf() {
    let source = include_str!("../fixtures/nir/labeled_data.act").replace("\r\n", "\n");
    let expected = include_str!("../fixtures/nir/labeled_data.nir").replace("\r\n", "\n");
    for source in [source.clone(), source.replace('\n', "\r\n")] {
        let ast = actionc::parser::parse(&actionc::lexer::tokenize(&source).unwrap()).unwrap();
        let model = actionc::semantic::analyze_with_options(
            &ast, actionc::semantic::SemanticOptions::modern(),
        ).unwrap();
        let n = nir::lower_program(&actionc::semantic::ir::lower_program(&ast, &model));
        nir::verify_program(&n).unwrap();
        assert_eq!(nir::format_program(&n), expected);
    }
}

#[test]
fn labeled_data_identifier_operands_do_not_start_declarations() {
    for separator in [" ", "\n", "\r\n"] {
        let source = [
            "CARD FUNC Copy(CARD data)",
            "CARD captured,pushed",
            "pushed=data",
            "captured=1+data",
            "pushed=2",
            "RETURN(captured)",
            "DATA table=[0]",
            "PROC Main()",
            "BYTE data=2,next",
            "BYTE copied=data",
            "next=1+data",
            "data=3",
            "RETURN",
        ].join(separator);
        let ast = actionc::parser::parse(&actionc::lexer::tokenize(&source).unwrap()).unwrap();
        let model = actionc::semantic::analyze_with_options(
            &ast, actionc::semantic::SemanticOptions::modern(),
        ).unwrap();
        let n = nir::lower_program(&actionc::semantic::ir::lower_program(&ast, &model));
        nir::verify_program(&n).unwrap();
        assert_eq!(n.routines.len(), 2);
        assert_eq!(n.statics.len(), 1);
    }
}

fn semantic(target: TargetId) -> actionc::semantic::ir::SemProgram {
    let source = "DATA links=[@bytes <@bytes >@bytes @links+2] DATA bytes=[\"OK\" 'K $1234] PROC Main() RETURN";
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model = actionc::semantic::analyze_with_options(
        &ast,
        actionc::semantic::SemanticOptions::modern().with_target(target),
    )
    .unwrap();
    actionc::semantic::ir::lower_program(&ast, &model)
}

fn program(target: TargetId) -> nir::NirProgram {
    nir::optimize_program(&nir::lower_program(&semantic(target))).unwrap()
}

#[test]
fn labeled_data_classic_projection_keeps_exact_inline_bytes() {
    let sem = semantic(TargetId::Atari6502);
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let output = actionc::codegen::generate_semir_profile_at_origin_with_runtime(
            &sem,
            0x3000,
            actionc::codegen::CodegenProfile::Modern,
            runtime,
        )
        .unwrap();
        let address = |name| {
            u32::from(
                output
                    .map
                    .storage_symbols
                    .iter()
                    .find(|s| s.name.eq_ignore_ascii_case(name))
                    .unwrap_or_else(|| panic!("missing {name}, {runtime:?}: {:?}", output.map.storage_symbols))
                    .address,
            )
        };
        check(address("links"), address("bytes"), 2, false, |a, len| {
            let offset = a as usize - 0x3000;
            output.bytes[offset..offset + len].to_vec()
        });
    }
}

fn check(
    links: u32,
    bytes: u32,
    width: usize,
    big_endian: bool,
    read: impl Fn(u32, usize) -> Vec<u8>,
) {
    let pointer = |value: u32| {
        if big_endian {
            value.to_be_bytes()[4 - width..].to_vec()
        } else {
            value.to_le_bytes()[..width].to_vec()
        }
    };
    let mut expected = pointer(bytes);
    expected.extend([bytes as u8, (bytes >> 8) as u8]);
    expected.extend(pointer(links + 2));
    assert_eq!(read(links, expected.len()), expected);
    let mut payload = vec![2, b'O', b'K'];
    payload.extend(pointer(bytes));
    payload.extend([b'K', 0x9A]);
    payload.extend(if big_endian {
        [0x12, 0x34]
    } else {
        [0x34, 0x12]
    });
    assert_eq!(read(bytes, payload.len()), payload);
}

#[test]
fn labeled_data_6502_emits_exact_payload_and_relocated_addresses() {
    let n = program(TargetId::Atari6502);
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let output = mir6502::generate_output_with_config_and_runtime(
            &n,
            0x3000,
            &mir6502::Mir6502Config::optimized(),
            runtime,
        )
        .unwrap();
        let address = |name| {
            u32::from(
                output
                    .map
                    .storage_symbols
                    .iter()
                    .find(|s| s.name.eq_ignore_ascii_case(name))
                    .unwrap_or_else(|| panic!("missing {name}, {runtime:?}: {:?}", output.map.storage_symbols))
                    .address,
            )
        };
        check(address("links"), address("bytes"), 2, false, |a, len| {
            let offset = a as usize - 0x3000;
            output.bytes[offset..offset + len].to_vec()
        });
        assert_eq!(
            output
                .routine_addresses
                .iter()
                .filter(|r| ["links", "bytes"].contains(&r.name.as_str()))
                .count(),
            0
        );
    }
}

#[test]
fn labeled_data_native_backends_emit_static_relocations() {
    let mir = mir65816::lower_program(&program(TargetId::Wdc65816Native)).unwrap();
    let machine = mir65816::emit::materialize(&mir).unwrap();
    let options = mir65816::image::LinkOptions {
        stack_checks: true,
        code_origin: 0x18000,
        data_origin: 0x120000,
        read_only_origin: None,
        zero_fill_origin: None,
        stack_overflow: 0x48000,
        arithmetic_fault: None,
        nmi_extra_stack: 0,
        imports: vec![],
    };
    let image = mir65816::image::link(&mir, &machine, &options).unwrap();
    let address = |name| image.data.iter().find(|s| s.name.eq_ignore_ascii_case(name)).unwrap().address;
    check(address("links"), address("bytes"), 3, false, |a, len| {
        let segment = image
            .segments
            .iter()
            .find(|s| a >= s.address && a + len as u32 <= s.address + s.bytes.len() as u32)
            .unwrap();
        segment.bytes[(a - segment.address) as usize..(a - segment.address) as usize + len].to_vec()
    });
    let mir = mir68k::lower_program(&program(TargetId::Motorola68000)).unwrap();
    let machine = mir68k::materialize::materialize(&mir).unwrap();
    let image = mir68k::image::link(&mir, &machine, 0x10000).unwrap();
    let address = |name| {
        image
            .symbols
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
            .unwrap()
            .address()
            .unwrap()
    };
    check(address("links"), address("bytes"), 4, true, |a, len| {
        let segment = image
            .segments
            .iter()
            .find(|s| a >= s.address && a + len as u32 <= s.address + s.bytes.len() as u32)
            .unwrap();
        segment.bytes[(a - segment.address) as usize..(a - segment.address) as usize + len].to_vec()
    });
}

#[test]
fn labeled_data_preserves_raw_menu_encoding() {
    let source = "PROC menu=*() [\"Yes\" 'Y \"Abort\" 'A $00] PROC Main() RETURN";
    let compile = |source: &str| {
        let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
        let model = actionc::semantic::analyze_with_options(
            &ast,
            actionc::semantic::SemanticOptions::modern(),
        )
        .unwrap();
        let n = nir::lower_program(&actionc::semantic::ir::lower_program(&ast, &model));
        mir6502::generate_output_with_config_and_runtime(
            &n,
            0x3000,
            &mir6502::Mir6502Config::optimized(),
            Runtime::ActionCart,
        )
        .unwrap()
    };
    let raw = compile(source);
    let data = compile(&source.replace("PROC menu=*()", "DATA menu="));
    let start = raw
        .routine_addresses
        .iter()
        .find(|r| r.name == "menu")
        .unwrap()
        .address;
    let symbol = data
        .map
        .storage_symbols
        .iter()
        .find(|s| s.name == "menu")
        .unwrap();
    assert_eq!(symbol.size, 19);
    let normalized = |out: &actionc::codegen::CodegenOutput, address: u16| {
        let offset = usize::from(address - 0x3000);
        let mut bytes = out.bytes[offset..offset + 19].to_vec();
        for (position, record) in [(4, 0), (14, 8)] {
            let value = u16::from_le_bytes([bytes[position], bytes[position + 1]]);
            assert_eq!(value, address + record);
            bytes[position..position + 2].copy_from_slice(&record.to_le_bytes());
        }
        bytes
    };
    assert_eq!(normalized(&raw, start), normalized(&data, symbol.address));
}
