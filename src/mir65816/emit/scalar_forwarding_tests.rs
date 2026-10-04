use super::*;

fn routine(source: &str) -> Mir65816Routine {
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(
        &ast,
        crate::semantic::SemanticOptions::modern()
            .with_target(crate::target::TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    crate::nir::verify_program(&nir).unwrap();
    let nir = crate::nir::optimize_program(&nir).unwrap();
    crate::mir65816::lower_program(&nir)
        .unwrap()
        .routines
        .into_iter()
        .find(|r| r.name == "Work")
        .unwrap()
}

#[test]
fn adjacent_long_parameter_reads_keep_frames_and_cover_both_operand_roles() {
    for ty in ["LONGCARD", "LONGINT"] {
        for op in ["+", "-", "&", "%", "XOR", "=", "#", "<", ">=", ">", "<="] {
            for swap in [false, true] {
                let constant = format!("{ty}($8123FFFF)");
                let (left, right) = if swap {
                    (constant.as_str(), "x")
                } else {
                    ("x", constant.as_str())
                };
                let result = if ["+", "-", "&", "%", "XOR"].contains(&op) {
                    ty
                } else {
                    "CARD"
                };
                let r = routine(&format!(
                    "{result} FUNC Work({ty} x) RETURN({left} {op} {right})"
                ));
                let frame = AllocatedFrame::new(&r).unwrap();
                let plan = Plan::new(&r, &frame).unwrap();
                assert_eq!(plan.bindings.len(), 1, "{ty}/{op}/{swap}");
                let binding = &plan.bindings[0];
                let machine = super::super::routine(&r, true).unwrap();
                assert!(machine.code.mir_spans[&binding.definition].is_empty());
                assert_eq!(format!("{frame:?}"), format!("{:?}", machine.frame));
                assert!(machine.frame.temps.contains_key(&binding.temp));
                assert_eq!(binding.source.home.width, 4);
            }
        }
    }
}

#[test]
fn scalar_bindings_reject_extra_uses_mutability_escape_and_noncanonical_homes() {
    let base = routine("LONGINT FUNC Work(LONGINT x) RETURN(x+LONGINT(1))");
    let frame = AllocatedFrame::new(&base).unwrap();
    let original = Plan::new(&base, &frame).unwrap();
    let binding = &original.bindings[0];
    let index = binding.definition.1;
    let consumer = binding.consumer;
    for problem in 0..9 {
        let mut r = base.clone();
        match problem {
            0 => {
                let op = r.blocks[0].ops[consumer].clone();
                r.blocks[0].ops.push(op);
            }
            1 => {
                let op = r.blocks[0].ops[index].clone();
                r.blocks[0].ops.push(op);
            }
            2 => {
                if let Mir65816Op::Load { volatile, .. } = &mut r.blocks[0].ops[index] {
                    *volatile = true;
                }
            }
            3 => {
                if let Mir65816Op::Load { address, .. } = &mut r.blocks[0].ops[index] {
                    address.displacement = ByteOffset::new(1);
                }
            }
            4 => r.frame.parameters[0].frame_object = Some(Mir65816FrameObjectId(99)),
            5 => {
                let Mir65816Op::Load { address, .. } = &r.blocks[0].ops[index] else {
                    unreachable!()
                };
                let address = address.clone();
                r.blocks[0].ops.push(Mir65816Op::Store {
                    address,
                    value: Mir65816Value::U32(0),
                    width: ByteSize::new(4),
                    volatile: false,
                });
            }
            6 => {
                let Mir65816Op::Load { address, .. } = &r.blocks[0].ops[index] else {
                    unreachable!()
                };
                let address = address.clone();
                r.blocks[0].ops.push(Mir65816Op::AddressOf {
                    dest: TempId(99),
                    address,
                    width: ByteSize::new(3),
                });
            }
            7 => {
                if let Mir65816Op::Binary { right, .. } = &mut r.blocks[0].ops[consumer] {
                    *right = Mir65816Value::Temp(binding.temp, ByteSize::new(4));
                }
            }
            _ => {
                if let Mir65816Terminator::Return { value, .. } = &mut r.blocks[0].terminator {
                    *value = Some(Mir65816Value::Temp(binding.temp, ByteSize::new(4)));
                }
            }
        }
        assert!(
            Plan::new(&r, &frame).unwrap().bindings.is_empty(),
            "{problem}"
        );
    }
    let mut forged = frame.clone();
    forged
        .temps
        .insert(binding.temp, Location::Stack(binding.source.home));
    assert!(Plan::new(&base, &forged).is_err());
    let mut forged = frame.clone();
    forged.extent = 252;
    assert!(Plan::new(&base, &forged).is_err());
}

#[test]
fn source_bindings_expire_at_the_exact_consumer_and_never_redirect_writes() {
    let r = routine("LONGINT FUNC Work(LONGINT x) RETURN(x+LONGINT(1))");
    let frame = AllocatedFrame::new(&r).unwrap();
    let plan = Plan::new(&r, &frame).unwrap();
    let binding = &plan.bindings[0];
    let mut b = Builder {
        stack_checks: false,
        routine: &r,
        code: TrackedEmitter65816::for_test(&frame),
        frame,
        blocks: BTreeMap::new(),
        next_block: None,
        loop_x: None,
        borrowed: BTreeMap::new(),
        scalar_borrowed: BTreeMap::new(),
    };
    let allocated = b.temp(binding.temp).unwrap();
    let value = Mir65816Value::Temp(binding.temp, ByteSize::new(4));
    assert!(plan.enter(&mut b, binding.definition.0, binding.definition.1));
    assert!(!plan.enter(&mut b, binding.definition.0, binding.consumer));
    assert_eq!(b.temp(binding.temp).unwrap(), allocated);
    assert!(
        matches!(b.value_memory(&value).unwrap(), Some(Memory::Stack(s)) if s == u32::from(binding.source.home.offset))
    );
    assert!(!plan.enter(&mut b, binding.definition.0, binding.consumer + 1));
    assert!(
        matches!(b.value_memory(&value).unwrap(), Some(Memory::Stack(s)) if s == u32::from(allocated.slot().offset))
    );
}
