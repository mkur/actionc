use actionc::{
    nir::*,
    semantic::{self, SemanticOptions},
    target::TargetId,
};

const TARGETS: [TargetId; 4] = [
    TargetId::Atari6502,
    TargetId::Motorola68000,
    TargetId::Wdc65816Small,
    TargetId::Wdc65816Native,
];

fn lower(source: &str, target: TargetId) -> NirProgram {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, SemanticOptions::modern().with_target(target))
        .unwrap();
    let program = lower_program(&semantic::ir::lower_program(&ast, &model));
    verify_program(&program).unwrap();
    program
}

fn optimize(program: &NirProgram) -> NirProgram {
    let result = optimize_program(program).unwrap();
    verify_program(&result).unwrap();
    assert_eq!(
        result,
        optimize_program(&result).unwrap(),
        "optimizer fixed point"
    );
    result
}

fn has_local(program: &NirProgram, suffix: &str) -> bool {
    program
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .unwrap()
        .locals
        .iter()
        .any(|local| local.name == suffix || local.name.ends_with(&format!("::{suffix}")))
}

fn local_place(local: &NirLocal) -> NirPlace {
    NirPlace {
        kind: NirPlaceKind::Local {
            id: local.id,
            name: local.name.clone(),
        },
        ty: Some(local.ty.clone()),
    }
}

#[test]
fn direct_and_internal_address_reads_drop_copies_and_homes_on_all_layouts() {
    for target in TARGETS {
        for (declaration, reads) in [
            (
                "TYPE Value=[BYTE first,second]",
                "out=saved.first+saved.second",
            ),
            (
                "TYPE Value=UNION [CARD word BYTE ARRAY bytes(3)]",
                "out=saved.bytes(0)+saved.bytes(2)",
            ),
            (
                "TYPE Value=[BYTE ARRAY bytes(3)]",
                "out=saved.bytes(0)+saved.bytes(2)",
            ),
        ] {
            let raw = lower(
                &format!(
                    "{declaration} Value original BYTE out PROC Main() LET saved=original {reads} PrintBE(out) RETURN"
                ),
                target,
            );
            assert!(
                has_local(&raw, "saved"),
                "{target:?}: {}",
                format_program(&raw)
            );
            let optimized = optimize(&raw);
            assert!(
                !has_local(&optimized, "saved"),
                "{target:?}: {}",
                format_program(&optimized)
            );
            assert!(
                !optimized
                    .routines
                    .iter()
                    .find(|routine| routine.name == "Main")
                    .unwrap()
                    .blocks
                    .iter()
                    .flat_map(|b| &b.ops)
                    .any(|op| matches!(op, NirOp::CopyBytes { .. }))
            );
        }
    }
}

#[test]
fn same_block_proof_preserves_snapshots_at_every_read_not_just_the_first() {
    for target in TARGETS {
        for intervening in [
            "original.first=9",                 // known overlapping field
            "original=other",                   // complete replacement
            "Mutate()",                         // effects and static reentry barrier
            "p^=9",                             // unresolved pointer write
            "hardware=9",                       // absolute memory
            "IF flag THEN original.first=9 FI", // cross-block consumers
        ] {
            let source = format!(
                "TYPE Value=[BYTE first,second] Value original,other BYTE first,last,flag,hardware=$D400 BYTE POINTER p PROC Mutate() original.first=9 RETURN PROC Main() LET saved=original first=saved.first {intervening} last=saved.second RETURN"
            );
            let optimized = optimize(&lower(&source, target));
            assert!(
                has_local(&optimized, "saved"),
                "{target:?}/{intervening}: {}",
                format_program(&optimized)
            );
        }
        let raw = lower(
            "TYPE Value=[BYTE first,second] Value original,other BYTE out PROC Main() LET saved=original other.first=9 out=saved.first+saved.second original.first=8 RETURN",
            target,
        );
        assert!(
            !has_local(&optimize(&raw), "saved"),
            "disjoint writes and writes after last read"
        );
    }
}

// Insert a complete same-block capture link into verified NIR. Variant source
// validation/control flow is left intact: the rewrite must remove only this
// redundant byte-image snapshot, never a check or its failure path.
fn with_bridge(declaration: &str, target: TargetId) -> (NirProgram, LocalId) {
    let mut program = lower(
        &format!("{declaration} Value original PROC Main() LET saved=original RETURN"),
        target,
    );
    let routine = program.routines.last_mut().unwrap();
    let mut bridge = routine
        .locals
        .iter()
        .find(|l| l.purpose == NirLocalPurpose::AggregateCapture)
        .unwrap()
        .clone();
    bridge.id = LocalId(routine.locals.iter().map(|l| l.id.0).max().unwrap() + 1);
    bridge.name = "bridge".into();
    let place = local_place(&bridge);
    let id = bridge.id;
    routine.locals.push(bridge);
    let block = routine
        .blocks
        .iter_mut()
        .find(|b| b.ops.iter().any(|op| matches!(op, NirOp::CopyBytes { .. })))
        .unwrap();
    let index = block
        .ops
        .iter()
        .position(|op| matches!(op, NirOp::CopyBytes { .. }))
        .unwrap();
    let NirOp::CopyBytes { source, size, .. } = &mut block.ops[index] else {
        unreachable!()
    };
    let copy = NirOp::CopyBytes {
        destination: place.clone(),
        source: source.clone(),
        size: *size,
        destination_volatile: false,
        source_volatile: false,
    };
    *source = place;
    block.ops.insert(index, copy);
    for temp in &mut routine.temps {
        if temp.def.block == block.id
            && let Some(at) = &mut temp.def.op_index
            && *at >= index
        {
            *at += 1;
        }
    }
    verify_program(&program).unwrap();
    (program, id)
}

#[test]
fn complete_nominal_record_union_and_variant_chains_forward_without_removing_faults() {
    for target in TARGETS {
        for declaration in [
            "TYPE Value=[BYTE first CARD word BYTE tail]",
            "TYPE Value=UNION [BYTE first CARD word BYTE ARRAY bytes(3)]",
            "TYPE Value=VARIANT [NONE SOME [BYTE first] WIDE [CARD word]]",
        ] {
            let (raw, bridge) = with_bridge(declaration, target);
            let baseline = lower(
                &format!("{declaration} Value original PROC Main() LET saved=original RETURN"),
                target,
            );
            let optimized = optimize(&raw);
            assert!(
                !optimized
                    .routines
                    .last()
                    .unwrap()
                    .locals
                    .iter()
                    .any(|local| local.id == bridge)
            );
            let faults = |program: &NirProgram| {
                program
                    .routines
                    .iter()
                    .flat_map(|r| &r.blocks)
                    .flat_map(|b| &b.ops)
                    .filter(|op| {
                        matches!(
                            op,
                            NirOp::Call {
                                callee: NirCallee::Fault(_),
                                ..
                            }
                        )
                    })
                    .count()
            };
            assert_eq!(faults(&optimized), faults(&optimize(&baseline)));
        }
    }
}

#[test]
fn by_value_call_and_return_captures_remain_whole_abi_objects() {
    for target in TARGETS {
        let raw = lower(
            "TYPE Value=[BYTE first,second] Value original BYTE out Value FUNC Identity(Value arg) RETURN(arg) PROC Consume(Value arg) out=arg.first RETURN PROC Main() LET saved=original Consume(saved) original=Identity(saved) RETURN",
            target,
        );
        let optimized = optimize(&raw);
        for routine in &optimized.routines {
            let mut check = |value: &NirValue| {
                if let NirValue::Aggregate { place } = value {
                    let Some(NirStorageId::Local(id)) = direct_storage_id(place) else {
                        panic!("whole ABI capture")
                    };
                    assert_eq!(
                        routine.locals.iter().find(|l| l.id == id).unwrap().purpose,
                        NirLocalPurpose::AggregateCapture
                    );
                }
            };
            for block in &routine.blocks {
                for op in &block.ops {
                    if let NirOp::Call { args, .. } = op {
                        args.iter().for_each(&mut check);
                    }
                }
                if let NirTerminator::Return(Some(value)) = &block.terminator {
                    check(value);
                }
            }
        }
    }
}

#[test]
fn volatile_fault_machine_and_dynamic_index_boundaries_keep_snapshots() {
    for (declarations, middle, read) in [
        (
            "VOLATILE BYTE status=$D20A BYTE observed",
            "observed=status",
            "saved.first",
        ),
        ("CARD divisor,quotient", "quotient=9/divisor", "saved.first"),
        ("", "[ $EA ]", "saved.first"),
        ("BYTE index", "", "saved.bytes(index)"),
    ] {
        let raw = lower(
            &format!(
                "TYPE Value=UNION [BYTE first BYTE ARRAY bytes(3)] Value original BYTE out {declarations} PROC Main() LET saved=original {middle} out={read} RETURN"
            ),
            TargetId::Atari6502,
        );
        assert!(has_local(&optimize(&raw), "saved"), "{middle}/{read}");
    }
}

#[test]
fn volatile_transfers_and_equal_size_different_nominal_types_do_not_forward() {
    for target in TARGETS {
        for control in ["source-volatile", "destination-volatile", "different-type"] {
            let mut raw = lower(
                "TYPE Value=[BYTE first,second] TYPE OtherType=UNION [CARD word] Value original OtherType other BYTE out PROC Main() LET saved=original out=saved.first RETURN",
                target,
            );
            let other = raw
                .globals
                .iter()
                .find(|global| global.name == "other")
                .unwrap();
            let other = NirPlace {
                kind: NirPlaceKind::Global {
                    id: other.id,
                    name: other.name.clone(),
                },
                ty: other.ty.clone(),
            };
            let routine = raw
                .routines
                .iter_mut()
                .find(|routine| routine.name == "Main")
                .unwrap();
            let copy = routine
                .blocks
                .iter_mut()
                .flat_map(|block| &mut block.ops)
                .find(|op| matches!(op, NirOp::CopyBytes { .. }))
                .unwrap();
            let NirOp::CopyBytes {
                source,
                source_volatile,
                destination_volatile,
                ..
            } = copy
            else {
                unreachable!()
            };
            match control {
                "source-volatile" => *source_volatile = true,
                "destination-volatile" => *destination_volatile = true,
                _ => *source = other,
            }
            verify_program(&raw).unwrap();
            assert!(has_local(&optimize(&raw), "saved"), "{target:?}/{control}");
        }
    }
}

#[test]
fn data_relocations_and_explicit_effect_references_keep_capture_storage() {
    for target in TARGETS {
        for data in [false, true] {
            // Native automatic homes cannot appear in a load-time address
            // image at all; that stronger verifier rule is unchanged.
            if data && target != TargetId::Atari6502 {
                continue;
            }
            let mut raw = lower(
                "TYPE Value=[BYTE first,second] Value original Value POINTER observed PROC Main() Value POINTER address observed=address LET saved=original PrintBE(0) RETURN",
                target,
            );
            let routine = raw
                .routines
                .iter_mut()
                .find(|routine| routine.name == "Main")
                .unwrap();
            let capture = routine
                .locals
                .iter()
                .find(|local| local.purpose == NirLocalPurpose::AggregateCapture)
                .unwrap();
            let id = NirStorageId::Local(capture.id);
            let size = capture.layout.size;
            if data {
                // A typed relocation is an identity exposure even with no
                // remaining executable load/store of the aggregate.
                let width = ByteSize::new(u32::from(raw.target_layout.data_pointer.size_bytes));
                let holder = routine
                    .locals
                    .iter_mut()
                    .find(|local| local.ty.pointer)
                    .unwrap();
                holder.init = Some(NirStorageInit::Bytes {
                    image: NirDataImage {
                        bytes: vec![0; width.get() as usize],
                        fragments: vec![NirDataFragment::Address {
                            offset: ByteOffset::new(0),
                            encoding: NirDataAddressEncoding::Pointer {
                                address_space: actionc::target::TargetLayout::DATA_ADDRESS_SPACE,
                                width,
                            },
                            target: NirDataAddressTarget::Storage(id),
                            addend: 0,
                            span: actionc::source::Span { start: 0, end: 0 },
                        }],
                    },
                    zero_fill: ByteSize::new(0),
                    mutable: false,
                    section: ".data".into(),
                });
            } else {
                let NirOp::Call { effects, .. } = routine
                    .blocks
                    .iter_mut()
                    .flat_map(|block| &mut block.ops)
                    .find(|op| matches!(op, NirOp::Call { .. }))
                    .unwrap()
                else {
                    unreachable!()
                };
                effects.memory.reads = NirMemoryAccess::Regions(vec![NirMemoryRegion {
                    kind: NirMemoryRegionKind::Storage(id),
                    offset: ByteOffset::new(0),
                    size,
                }]);
            }
            verify_program(&raw).unwrap();
            assert!(
                has_local(&optimize(&raw), "saved"),
                "{target:?}/data={data}"
            );
        }
    }
}

#[test]
fn reads_before_initialization_and_case_reads_in_other_blocks_keep_captures() {
    let mut raw = lower(
        "TYPE Value=[BYTE first,second] Value original BYTE out PROC Main() LET saved=original out=saved.first RETURN",
        TargetId::Atari6502,
    );
    let routine = raw.routines.last_mut().unwrap();
    assert!(matches!(routine.blocks[0].ops[0], NirOp::CopyBytes { .. }));
    assert!(matches!(routine.blocks[0].ops[1], NirOp::Load { .. }));
    routine.blocks[0].ops.swap(0, 1);
    routine.temps[0].def.op_index = Some(0);
    verify_program(&raw).unwrap();
    assert!(has_local(&optimize(&raw), "saved"));

    let raw = lower(
        include_str!("support/fresh_maybe_byte.act"),
        TargetId::Atari6502,
    );
    let optimized = optimize(&raw);
    let main = optimized
        .routines
        .iter()
        .find(|routine| routine.name == "Main")
        .unwrap();
    assert_eq!(
        main.locals
            .iter()
            .filter(|local| local.purpose == NirLocalPurpose::AggregateCapture)
            .count(),
        2,
        "cross-block CASE proof is slice 4, not implied by LET immutability"
    );
}

#[test]
fn partial_overlap_copy_consumers_and_observed_capture_addresses_stay_staged() {
    for target in TARGETS {
        let mut raw = lower(
            "TYPE Value=[BYTE first,second] Value original BYTE out PROC Main() LET saved=original out=saved.first RETURN",
            target,
        );
        let routine = raw.routines.last_mut().unwrap();
        let block = &mut routine.blocks[0];
        let NirOp::CopyBytes {
            source: original, ..
        } = &block.ops[0]
        else {
            panic!("initialization")
        };
        let NirOp::Load {
            place: first, ty, ..
        } = &block.ops[1]
        else {
            panic!("field read")
        };
        let copy = NirOp::CopyBytes {
            destination: NirPlace {
                kind: NirPlaceKind::Field {
                    base: Box::new(original.clone()),
                    offset: ByteOffset::new(1),
                    ty: ty.clone(),
                },
                ty: Some(ty.clone()),
            },
            source: first.clone(),
            size: ByteSize::ONE,
            destination_volatile: false,
            source_volatile: false,
        };
        block.ops.truncate(1);
        block.ops.push(copy);
        routine.temps.clear();
        verify_program(&raw).unwrap();
        assert!(
            has_local(&optimize(&raw), "saved"),
            "{target:?}: partial overlap consumer"
        );

        let mut raw = lower(
            "TYPE Value=[BYTE first,second] Value original Value POINTER observed PROC Main() LET saved=original observed=@original RETURN",
            target,
        );
        let routine = raw.routines.last_mut().unwrap();
        let capture = local_place(
            routine
                .locals
                .iter()
                .find(|local| local.purpose == NirLocalPurpose::AggregateCapture)
                .unwrap(),
        );
        let NirOp::AddrOf { place, .. } = routine
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.ops)
            .find(|op| matches!(op, NirOp::AddrOf { .. }))
            .unwrap()
        else {
            unreachable!()
        };
        *place = capture;
        verify_program(&raw).unwrap();
        assert!(
            has_local(&optimize(&raw), "saved"),
            "{target:?}: exposed identity"
        );
        // Once that address is not observed, the shared pure-temp cleanup may
        // delete it and the copy/home proof can succeed. Retain its evaluated
        // inputs, if any; AddrOf itself does not dereference memory.
        let routine = raw.routines.last_mut().unwrap();
        assert!(matches!(routine.blocks[0].ops.last(), Some(NirOp::Store { .. })));
        routine.blocks[0].ops.pop();
        let optimized = optimize(&raw);
        assert!(!has_local(&optimized, "saved"));
        assert!(!optimized.routines.last().unwrap().blocks.iter().flat_map(|block| &block.ops)
            .any(|op| matches!(op, NirOp::AddrOf { .. })));
    }
}
