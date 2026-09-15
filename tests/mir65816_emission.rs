use actionc::{
    lexer,
    mir65816::{self, emit, image},
    nir, parser, semantic,
    target::TargetId,
};

fn mir(source: &str, optimize: bool) -> mir65816::Mir65816Program {
    let ast = parser::parse(&lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(
        &ast,
        semantic::SemanticOptions::modern().with_target(TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = nir::lower_program(&semantic::ir::lower_program(&ast, &model));
    let nir = if optimize {
        nir::optimize_program_with_promotion(&nir, nir::NirPromotionPolicy::NativeLoops).unwrap()
    } else {
        nir
    };
    mir65816::lower_program(&nir).unwrap()
}

fn layout() -> image::LinkOptions {
    image::LinkOptions {
        code_origin: 0x18000,
        data_origin: 0x120000,
        stack_overflow: 0x48000,
        nmi_extra_stack: 7,
        imports: vec![],
    }
}

#[test]
fn emits_checked_frames_and_round_trips_a_freestanding_image() {
    for optimize in [false, true] {
        let program = mir(
            "LONGINT result LONGINT FUNC Add(INT a LONGINT b) RETURN(LONGINT(a)+b) PROC Main() result=Add(-3,10) RETURN",
            optimize,
        );
        let machine = emit::materialize(&program).unwrap();
        assert!(machine.routines.iter().any(|r| r.frame.spill_bytes != 0));
        let image = image::link(&program, &machine, &layout()).unwrap();
        assert_eq!(image.task_headroom, 33);
        assert_eq!(image.irq_headroom, 20);
        assert!(image.routines.iter().all(|r| r.fixed_frame % 2 == 0));
        let loaded = image::Image::from_json(&image.to_json().unwrap()).unwrap();
        assert_eq!(loaded.entry, image.entry);
        assert!(
            loaded
                .data
                .iter()
                .any(|d| d.name == "result" && d.size == 4)
        );
    }
}

#[test]
fn unsupported_operations_and_unbound_assembly_fail_before_an_image_exists() {
    for (source, error) in [
        ("CARD a,b,result PROC Main() result=a*b RETURN", "Mul"),
        (
            "CARD FUNC POINTER callback() PROC Main() callback() RETURN",
            "slice 5",
        ),
    ] {
        let program = mir(source, false);
        assert!(emit::materialize(&program).unwrap_err().contains(error));
    }
    let mut program = mir("PROC Missing() RETURN PROC Main() Missing() RETURN", false);
    program.routines[0].entry.external = true;
    program.routines[0].entry.external_symbol = Some(nir::runtime_symbol_id("TEST.Missing"));
    let machine = emit::materialize(&program).unwrap();
    assert!(
        image::link(&program, &machine, &layout())
            .unwrap_err()
            .contains("unresolved assembly import")
    );
}

#[test]
fn allocated_spills_and_actual_outgoing_accesses_must_fit_stack_displacements() {
    let source = "CARD FUNC Large(CARD n) BYTE ARRAY buffer(248) buffer(0)=BYTE(n) RETURN(n+CARD(buffer(0))) PROC Main() RETURN";
    let program = mir(source, false);
    assert!(
        emit::materialize(&program)
            .unwrap_err()
            .contains("stack-relative")
    );
}

#[test]
fn linker_rejects_overlaps_address_overflow_and_corrupt_transport() {
    let program = mir("CARD result PROC Main() result=42 RETURN", false);
    let machine = emit::materialize(&program).unwrap();
    let mut options = layout();
    options.data_origin = options.code_origin;
    assert!(
        image::link(&program, &machine, &options)
            .unwrap_err()
            .contains("overlapping")
    );
    options = layout();
    options.code_origin = 0xffffff;
    assert!(image::link(&program, &machine, &options).is_err());
    let image = image::link(&program, &machine, &layout()).unwrap();
    let mut serialized: serde_json::Value =
        serde_json::from_slice(&image.to_json().unwrap()).unwrap();
    serialized["abi"] = "wrong-abi".into();
    assert!(image::Image::from_json(&serde_json::to_vec(&serialized).unwrap()).is_err());
}

#[test]
fn image_relocations_select_bytes_after_the_addend_and_reject_address_wrap() {
    use actionc::nir::{RoutineId, SymbolId};
    use actionc::target::{AddressValue, ByteOffset, ByteSize, TargetLayout};
    use mir65816::*;
    let mut program = mir("PROC Main() RETURN", false);
    program.data.push(Mir65816Data {
        id: Mir65816DataId::Static(SymbolId(900)),
        placement: Mir65816DataPlacement::Allocate,
        size: ByteSize::new(3),
        zero_fill: ByteSize::ZERO,
        mutable: false,
        ty: None,
        array: None,
        section: None,
        name: "relocation_bytes".into(),
        bytes: vec![0; 3],
        alignment: ByteSize::ONE,
        relocations: (0..3)
            .map(|byte| Mir65816Relocation {
                offset: ByteOffset::new(byte.into()),
                byte_index: Some(byte),
                width: ByteSize::ONE,
                address_space: TargetLayout::CODE_ADDRESS_SPACE,
                target: Mir65816RelocationTarget::Code(RoutineId(0)),
                addend: 0xffff,
            })
            .collect(),
    });
    let machine = emit::materialize(&program).unwrap();
    let linked = image::link(&program, &machine, &layout()).unwrap();
    let bytes = &linked
        .segments
        .iter()
        .find(|s| s.address == layout().data_origin)
        .unwrap()
        .bytes;
    assert_eq!(bytes, &[0xff, 0x7f, 2]); // $018000 + $FFFF = $027FFF
    program.data[0].relocations[0].target =
        Mir65816RelocationTarget::Absolute(AddressValue::code(0xffffff));
    program.data[0].relocations[0].addend = 1;
    assert!(
        image::link(&program, &machine, &layout())
            .unwrap_err()
            .contains("exceeds the 24-bit address space")
    );
}
