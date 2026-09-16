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
        nir::optimize_program_with_promotion(&nir, nir::NirPromotionPolicy::Native65816).unwrap()
    } else {
        nir
    };
    mir65816::lower_program(&nir).unwrap()
}

fn layout() -> image::LinkOptions {
    image::LinkOptions {
        code_origin: 0x18000,
        data_origin: 0x120000,
        read_only_origin: None,
        zero_fill_origin: None,
        stack_overflow: 0x48000,
        nmi_extra_stack: 7,
        imports: vec![],
    }
}

#[test]
fn pointer_unlink_has_a_bounded_native_code_size() {
    let source = "TYPE Node=[Node POINTER ln_Succ Node POINTER ln_Pred] \
        PROC Remove(Node POINTER item) Node POINTER previous,following \
        previous=item.ln_Pred following=item.ln_Succ \
        previous.ln_Succ=following following.ln_Pred=previous RETURN";
    for optimize in [false, true] {
        let program = mir(source, optimize);
        let machine = emit::materialize(&program).unwrap();
        let image = image::link(&program, &machine, &layout()).unwrap();
        let remove = image.routines.iter().find(|r| r.name == "Remove").unwrap();
        // The original bytewise selector used 369 bytes. Include the complete
        // checked entry and ABI return in the budget, not just the four accesses.
        assert!(
            remove.size <= if optimize { 144 } else { 223 },
            "Remove grew to {} bytes",
            remove.size
        );
        assert_eq!(remove.result_bytes, 0);
    }
}

#[test]
fn word_pointer_capture_checks_its_last_stack_byte_after_call_reservation() {
    use mir65816::{Mir65816CallTarget, Mir65816Op, Mir65816Value};
    let mut program = mir(
        "PROC Invoke(BYTE padding PROC POINTER cb) BYTE ARRAY buffer(248) cb() RETURN",
        false,
    );
    let routine = &mut program.routines[0];
    let parameter = routine.frame.parameters[1].param;
    // Feed the incoming callable directly to selection. With the 248-byte
    // frame it occupies d,S=253..255; reserving the zero-argument call's one
    // byte moves it to 254..256. Both word starts fit, but the last byte does not.
    routine.temps.clear();
    routine.blocks[0].ops.retain_mut(|op| {
        if let Mir65816Op::Call {
            target: Mir65816CallTarget::Indirect(value, _),
            ..
        } = op
        {
            *value = Mir65816Value::Param(parameter);
            true
        } else {
            false
        }
    });
    mir65816::verify_program(&program).unwrap();
    assert!(
        emit::materialize(&program)
            .unwrap_err()
            .contains("stack-relative")
    );
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
            "TYPE Pair=[BYTE tag CARD value] VOLATILE Pair source,target PROC Main() target=source RETURN",
            "volatile aggregate copy",
        ),
    ] {
        for optimize in [false, true] {
            let program = mir(source, optimize);
            assert!(emit::materialize(&program).unwrap_err().contains(error));
        }
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
    let mut options = layout();
    options.imports.push(image::AssemblyImport {
        symbol: nir::runtime_symbol_id("TEST.Missing").0,
        signature: program.routines[0].signature.0,
        abi: mir65816::abi::generated::ABI_NAME.into(),
        address: 0x40100,
        size: 1,
        stack_peak: 1,
        checks_stack: true,
        irq_effect: image::IrqEffect::SaveDisable,
    });
    assert!(
        image::link(&program, &machine, &options)
            .unwrap_err()
            .contains("IRQ-state import")
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

#[test]
fn indirect_per_relocations_reject_invalid_continuations_and_ranges() {
    let program = mir(
        "PROC POINTER cb PROC Empty() RETURN PROC Main() cb=@Empty cb() RETURN",
        false,
    );
    let machine = emit::materialize(&program).unwrap();
    let r = machine
        .routines
        .iter()
        .position(|r| !r.code.return_fixups.is_empty())
        .unwrap();
    let (offset, label) = machine.routines[r].code.return_fixups[0];
    let mut bad = machine.clone();
    bad.routines[r].code.bytes.resize(40000, 0xea);
    bad.routines[r].code.labels.insert(label, 39999);
    assert!(
        image::link(&program, &bad, &layout())
            .unwrap_err()
            .contains("relative range")
    );
    let mut bad = machine.clone();
    bad.routines[r].code.bytes[offset - 1] = 0xea;
    assert!(
        image::link(&program, &bad, &layout())
            .unwrap_err()
            .contains("PER instruction")
    );
    let mut options = layout();
    options.code_origin = 0x01fff0;
    let linked = image::link(&program, &machine, &options).unwrap();
    assert!(
        linked
            .routines
            .iter()
            .all(|r| r.address >> 16 == (r.address + r.size - 1) >> 16)
    );
}

#[test]
fn section_origins_and_allocated_frame_maps_survive_transport_and_reject_corruption() {
    let program = mir(
        "CARD output,initialized=[7] CARD FUNC Local(CARD n) BYTE ARRAY values=[1 2 3] RETURN(n+CARD(values(1))) PROC Main() output=Local(initialized) RETURN",
        false,
    );
    let machine = emit::materialize(&program).unwrap();
    let mut options = layout();
    options.read_only_origin = Some(0x340000);
    options.zero_fill_origin = Some(0x560000);
    let image = image::link(&program, &machine, &options).unwrap();
    assert!(
        image
            .segments
            .iter()
            .any(|s| !s.executable && !s.writable && s.address == 0x340000)
    );
    assert!(image.zero_fill.iter().any(|s| s.address == 0x560000));
    assert!(
        image
            .segments
            .iter()
            .any(|s| s.writable && s.address == options.data_origin)
    );
    let local = image.routines.iter().find(|r| r.name == "Local").unwrap();
    assert!(local.objects.len() >= 2 && !local.temporaries.is_empty());
    assert_eq!(
        local.arguments[0].body_displacement,
        u32::from(local.fixed_frame) + 4
    );
    let mut bad = image.clone();
    bad.routines[0].fixed_frame = 255;
    assert!(bad.to_json().unwrap_err().contains("frame map"));
    let mut bad = image.clone();
    bad.routines[0].arguments[0].body_displacement += 1;
    assert!(bad.to_json().unwrap_err().contains("displacement map"));
}

#[test]
fn pointer_allocation_proves_closed_lifetimes_and_rejects_corrupt_locations() {
    use mir65816::emit::{AllocatedFrame, Location, Slot};
    let program = mir(
        "TYPE Link=[Link POINTER next Link POINTER prev] PROC Cut(Link POINTER p) \
        Link POINTER a,b a=p.prev b=p.next a.next=b b.prev=a RETURN",
        true,
    );
    let routine = &program.routines[0];
    let frame = AllocatedFrame::pointer_leaf(routine).unwrap().unwrap();
    assert_eq!(
        (frame.extent, frame.spill_bytes, frame.peak_below_entry),
        (0, 0, 0)
    );
    assert_eq!(frame.temps.len(), 3);
    let ids = routine.blocks[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            mir65816::Mir65816Op::Load { dest, .. } => Some(*dest),
            _ => None,
        })
        .collect::<Vec<_>>();
    for (id, offset) in ids.iter().zip([0, 3, 6]) {
        assert_eq!(
            frame.temps[id],
            Location::DirectPage(Slot { offset, width: 3 })
        );
    }
    for bad in [
        Location::DirectPage(Slot {
            offset: 7,
            width: 3,
        }),
        Location::DirectPage(Slot {
            offset: 0,
            width: 4,
        }),
        Location::Stack(Slot {
            offset: 1,
            width: 3,
        }),
        frame.temps[&ids[0]],
    ] {
        let mut corrupt = frame.clone();
        corrupt.temps.insert(ids[2], bad);
        assert!(corrupt.verify_pointer_leaf(routine).is_err());
    }
    let mut corrupt = frame.clone();
    corrupt.temps.remove(&ids[0]);
    assert!(corrupt.verify_pointer_leaf(routine).is_err());
    let mut corrupt = frame;
    corrupt.spill_bytes = 2;
    assert!(corrupt.verify_pointer_leaf(routine).is_err());

    let chain = mir(
        "TYPE Link=[Link POINTER next] Link POINTER result \
        PROC Follow(Link POINTER p) result=p.next.next.next RETURN",
        true,
    );
    let frame = AllocatedFrame::pointer_leaf(&chain.routines[0])
        .unwrap()
        .unwrap();
    // Every load's input stays live until its final bank-byte access. A chain
    // therefore alternates slots instead of overwriting the dying base early.
    let offsets = chain.routines[0].blocks[0]
        .ops
        .iter()
        .filter_map(|op| match op {
            mir65816::Mir65816Op::Load { dest, .. } => Some(frame.temps[dest].slot().offset),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(offsets, [0, 3, 0, 3]);
}

#[test]
fn pointer_allocation_falls_back_for_pressure_and_unmodelled_scratch() {
    use mir65816::emit::AllocatedFrame;
    let declarations = "TYPE Link=[Link POINTER a Link POINTER b Link POINTER c] ";
    for source in [
        "PROC Change(Link POINTER p) Link POINTER a,b,c a=p.a b=p.b c=p.c a.a=b b.a=c c.a=a RETURN",
        "Link POINTER FUNC Follow(Link POINTER p) RETURN(p.a)",
        "PROC Barrier() RETURN PROC Change(Link POINTER p) p.a=p.b Barrier() RETURN",
        "PROC Change(Link POINTER p BYTE flag) IF flag THEN p.a=p.b FI RETURN",
        "PROC Change(Link POINTER p) Link POINTER a a=p.a p.b=@a RETURN",
        "PROC Change(Link POINTER p CARD index) Link ARRAY values(2) values(index).a=p.a RETURN",
        "PROC Change(Link POINTER p PROC POINTER cb) p.a=p.b cb() RETURN",
        "PROC Change(Link POINTER p) Change(p) RETURN",
        "PROC Change(Link POINTER p,q) p^=q^ RETURN",
        "PROC Change(Link POINTER p) BYTE ARRAY a(2) a(0)=1 p.a=p.b RETURN",
    ] {
        let program = mir(&format!("{declarations}{source}"), true);
        let routine = program.routines.last().unwrap();
        assert!(
            AllocatedFrame::pointer_leaf(routine).unwrap().is_none(),
            "{source}"
        );
    }
    let mut program = mir(
        &format!("{declarations}PROC Change(Link POINTER p) p.a=p.b RETURN"),
        true,
    );
    let mut large_offset = program.clone();
    for op in &mut large_offset.routines[0].blocks[0].ops {
        if let mir65816::Mir65816Op::Load { address, .. } = op
            && matches!(address.base, mir65816::Mir65816AddressBase::Indirect(_))
        {
            address.displacement = actionc::target::ByteOffset::new(65533);
        }
    }
    mir65816::verify_program(&large_offset).unwrap();
    assert!(
        AllocatedFrame::pointer_leaf(&large_offset.routines[0])
            .unwrap()
            .is_none()
    );
    for op in &mut program.routines[0].blocks[0].ops {
        if let mir65816::Mir65816Op::Load { volatile, .. } = op {
            *volatile = true;
        }
    }
    mir65816::verify_program(&program).unwrap();
    assert!(
        AllocatedFrame::pointer_leaf(&program.routines[0])
            .unwrap()
            .is_none()
    );
}

#[test]
fn direct_page_locations_survive_image_v3_and_reject_corrupt_maps() {
    let program = mir(
        "TYPE Node=[Node POINTER a Node POINTER b] PROC Cut(Node POINTER p) \
        Node POINTER a,b a=p.a b=p.b a.b=b b.a=a RETURN",
        true,
    );
    let machine = emit::materialize(&program).unwrap();
    let image = image::link(&program, &machine, &layout()).unwrap();
    let loaded = image::Image::from_json(&image.to_json().unwrap()).unwrap();
    assert_eq!(loaded.version, 3);
    let routine = &loaded.routines[0];
    assert_eq!(routine.fixed_frame, 0);
    assert_eq!(routine.spill_bytes, 0);
    assert_eq!(routine.arguments[0].body_displacement, 4);
    assert!(
        routine
            .temporaries
            .iter()
            .all(|t| matches!(t.home, image::TemporaryHome::DirectPage { .. }))
    );
    for home in [
        image::TemporaryHome::DirectPage { offset: 7 },
        image::TemporaryHome::DirectPage { offset: 44 },
        image::TemporaryHome::Stack { displacement: 0 },
    ] {
        let mut corrupt = loaded.clone();
        corrupt.routines[0].temporaries[0].home = home;
        assert!(corrupt.verify().is_err());
    }
    for version in [1, 2] {
        let mut json: serde_json::Value =
            serde_json::from_slice(&image.to_json().unwrap()).unwrap();
        json["version"] = version.into();
        assert!(
            image::Image::from_json(&serde_json::to_vec(&json).unwrap())
                .unwrap_err()
                .contains("recompile")
        );
    }
}

#[test]
fn pointer_leaf_rechecks_incoming_last_byte_after_removing_spills() {
    use mir65816::{Mir65816AbiHome, emit::AllocatedFrame};
    let program = mir(
        "TYPE Cell=[Cell POINTER next] \
        PROC Change(BYTE prefix Cell POINTER p) BYTE ARRAY frame(248) p.next=p.next.next RETURN",
        false,
    );
    let routine = &program.routines[0];
    let frame = AllocatedFrame::pointer_leaf(routine).unwrap().unwrap();
    assert_eq!(frame.extent, 248);
    // The pointer occupies exactly 253..255,S. Extra stack spills would make
    // this routine unrepresentable; resident values keep the ABI range valid.
    let machine = emit::materialize(&program).unwrap();
    let image = image::link(&program, &machine, &layout()).unwrap();
    assert_eq!(image.routines[0].arguments[1].body_displacement, 253);
    let mut invalid = routine.clone();
    if let Mir65816AbiHome::StackArgument { offset, .. } = &mut invalid.frame.parameters[1].incoming
    {
        *offset = actionc::target::ByteOffset::new(2);
    }
    assert!(
        frame
            .verify_pointer_leaf(&invalid)
            .unwrap_err()
            .contains("stack-relative")
    );
}
