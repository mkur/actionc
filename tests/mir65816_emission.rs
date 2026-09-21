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
fn native_word_add_sub_keep_frame_and_abi_costs_with_bounded_code() {
    for operation in ["+", "-"] {
        for optimize in [false, true] {
            let program = mir(
                &format!("CARD FUNC Work(CARD a,b) RETURN(a{operation}b) PROC Main() RETURN"),
                optimize,
            );
            let machine = emit::materialize(&program).unwrap();
            let image = image::link(&program, &machine, &layout()).unwrap();
            let work = image.routines.iter().find(|r| r.name == "Work").unwrap();
            // Includes the checked entry, direct word return and frame teardown.
            assert!(
                work.size <= 80,
                "{operation}/{optimize}: {} bytes",
                work.size
            );
            assert_eq!(
                (work.fixed_frame, work.spill_bytes, work.local_stack_peak),
                (8, 8, 8)
            );
            assert_eq!((work.outgoing_bytes, work.result_bytes), (5, 2));
            assert_eq!(
                work.arguments
                    .iter()
                    .map(|a| (a.offset, a.body_displacement, a.size))
                    .collect::<Vec<_>>(),
                [(0, 12, 2), (2, 14, 2)]
            );
            assert!(work.calls.is_empty());
            assert!(work.objects.is_empty());
            assert!(work.temporaries.iter().all(|t| t.size == 2));
        }
    }
}

#[test]
fn native_word_identity_keeps_its_frame_and_has_a_bounded_return() {
    for optimize in [false, true] {
        let program = mir(
            "CARD FUNC Echo(CARD value) RETURN(value) PROC Main() RETURN",
            optimize,
        );
        let machine = emit::materialize(&program).unwrap();
        let image = image::link(&program, &machine, &layout()).unwrap();
        let echo = image.routines.iter().find(|r| r.name == "Echo").unwrap();
        assert!(echo.size <= 70, "{optimize}: {} bytes", echo.size);
        assert_eq!(
            (echo.fixed_frame, echo.spill_bytes, echo.local_stack_peak),
            (4, 4, 4)
        );
        assert_eq!((echo.outgoing_bytes, echo.result_bytes), (3, 2));
        assert_eq!(
            echo.arguments
                .iter()
                .map(|a| (a.offset, a.body_displacement, a.size))
                .collect::<Vec<_>>(),
            [(0, 8, 2)]
        );
        assert!(echo.calls.is_empty());
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
    // Reused spills fit the last byte at 255, but rounding this allocation to
    // an even fixed frame requires 256 bytes, still outside the strategy.
    let error = emit::materialize(&program).unwrap_err();
    assert!(error.contains("fixed frame requires 256 bytes"), "{error}");
}

#[test]
fn stack_reuse_keeps_closed_operations_and_rejects_corrupt_plans() {
    use mir65816::{
        Mir65816Op, Mir65816Value,
        emit::{Location, Slot},
    };
    let program = mir(
        "LONGCARD FUNC Chain(LONGCARD n) RETURN(n+1+2+3+4+5+6+7+8) PROC Main() RETURN",
        false,
    );
    let machine = emit::materialize(&program).unwrap();
    let routine = &program.routines[0];
    let frame = &machine.routines[0].frame;
    frame.verify_stack(routine).unwrap();
    assert!(frame.extent <= 14, "{}", frame.extent);
    assert!(frame.temps.len() > 8);
    let (dest, input) = routine
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .find_map(|op| match op {
            Mir65816Op::Binary {
                dest,
                left: Mir65816Value::Temp(input, _),
                ..
            } => Some((*dest, *input)),
            _ => None,
        })
        .unwrap();
    let mut corrupt = frame.clone();
    let input_slot = frame.temps[&input].slot();
    // Partially overlapping wide homes are as invalid as identical homes.
    corrupt.temps.insert(
        dest,
        Location::Stack(Slot {
            offset: input_slot.offset + 2,
            width: 4,
        }),
    );
    assert!(
        corrupt
            .verify_stack(routine)
            .unwrap_err()
            .contains("overlapping live")
    );
    for slot in [
        Slot {
            offset: 0,
            width: 4,
        },
        Slot {
            offset: 255,
            width: 4,
        },
        Slot {
            offset: 2,
            width: 3,
        },
    ] {
        let mut corrupt = frame.clone();
        corrupt.temps.insert(dest, Location::Stack(slot));
        assert!(corrupt.verify_stack(routine).is_err());
    }
    let mut corrupt = frame.clone();
    corrupt.temps.remove(&input);
    assert!(corrupt.verify_stack(routine).is_err());
    for field in [0, 1, 2] {
        let mut corrupt = frame.clone();
        match field {
            0 => corrupt.extent += 2,
            1 => corrupt.spill_bytes += 2,
            _ => corrupt.peak_below_entry += 2,
        }
        assert!(corrupt.verify_stack(routine).is_err());
    }
    let linked = image::link(&program, &machine, &layout()).unwrap();
    let loaded = image::Image::from_json(&linked.to_json().unwrap()).unwrap();
    assert_eq!(
        loaded.routines[0]
            .temporaries
            .iter()
            .map(|t| (t.id, t.home, t.size))
            .collect::<Vec<_>>(),
        linked.routines[0]
            .temporaries
            .iter()
            .map(|t| (t.id, t.home, t.size))
            .collect::<Vec<_>>()
    );
}

#[test]
fn reused_stack_homes_keep_the_incoming_last_byte_at_255() {
    for (padding, accepted) in [(244, true), (246, false)] {
        let program = mir(
            &format!(
                "CARD FUNC Edge(CARD n) BYTE ARRAY padding({padding}) RETURN(n+1+2+3+4) PROC Main() RETURN"
            ),
            false,
        );
        if accepted {
            let machine = emit::materialize(&program).unwrap();
            assert_eq!(machine.routines[0].frame.extent, 250);
            let linked = image::link(&program, &machine, &layout()).unwrap();
            assert_eq!(linked.routines[0].arguments[0].body_displacement, 254);
        } else {
            let error = emit::materialize(&program).unwrap_err();
            assert!(
                error.contains("stack-relative access at 256 with width 2"),
                "{error}"
            );
        }
    }
}

#[test]
fn stack_reuse_preserves_backedge_live_ins_and_dead_parallel_destinations() {
    use actionc::target::ByteSize;
    use mir65816::{Mir65816Block, Mir65816Edge, Mir65816Op, Mir65816Terminator, Mir65816Value};
    use nir::{BlockId, NirBinaryOp, TempId};
    let mut program = mir("CARD FUNC Probe(CARD n) RETURN(n)", false);
    let routine = &mut program.routines[0];
    let ty = routine.temps[0].1.clone();
    routine.temps = (0..6).map(|id| (TempId(id), ty.clone())).collect();
    let word = ByteSize::new(2);
    let value = |id| Mir65816Value::Temp(TempId(id), word);
    let edge = |a, b| Mir65816Edge {
        target: BlockId(1),
        args: vec![value(a), value(b), Mir65816Value::U16(99)],
    };
    let mut load = routine.blocks[0].ops[0].clone();
    let Mir65816Op::Load { dest, .. } = &mut load else {
        panic!("expected parameter load")
    };
    *dest = TempId(0);
    let mut ret = routine.blocks[0].terminator.clone();
    let Mir65816Terminator::Return {
        value: returned, ..
    } = &mut ret
    else {
        panic!("expected return")
    };
    *returned = Some(value(4));
    routine.blocks = vec![
        Mir65816Block {
            id: BlockId(0),
            params: vec![],
            ops: vec![load],
            terminator: Mir65816Terminator::Goto(edge(0, 0)),
        },
        Mir65816Block {
            id: BlockId(1),
            params: vec![(TempId(1), word), (TempId(2), word), (TempId(5), word)],
            ops: vec![Mir65816Op::Binary {
                dest: TempId(3),
                width: word,
                signed: false,
                operation: NirBinaryOp::Add,
                left: value(1),
                right: Mir65816Value::U16(1),
            }],
            terminator: Mir65816Terminator::Branch {
                condition: Mir65816Value::U8(0),
                then_edge: edge(2, 1),
                else_edge: Mir65816Edge {
                    target: BlockId(2),
                    args: vec![],
                },
            },
        },
        Mir65816Block {
            id: BlockId(2),
            params: vec![],
            ops: vec![Mir65816Op::Binary {
                dest: TempId(4),
                width: word,
                signed: false,
                operation: NirBinaryOp::Add,
                left: value(0),
                right: value(3),
            }],
            terminator: ret,
        },
    ];
    mir65816::verify_program(&program).unwrap();
    let machine = emit::materialize(&program).unwrap();
    let routine = &program.routines[0];
    let frame = &machine.routines[0].frame;
    frame.verify_stack(routine).unwrap();
    for id in [1, 2, 3, 4, 5] {
        let mut corrupt = frame.clone();
        corrupt.temps.insert(TempId(id), frame.temps[&TempId(0)]);
        assert!(
            corrupt
                .verify_stack(routine)
                .unwrap_err()
                .contains("overlapping live")
        );
    }
    assert_eq!(frame.edge_copies.len(), 3);
    let mut corrupt = frame.clone();
    corrupt.edge_copies[0].offset = frame.temps[&TempId(0)].slot().offset;
    assert!(
        corrupt
            .verify_stack(routine)
            .unwrap_err()
            .contains("edge-copy")
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
