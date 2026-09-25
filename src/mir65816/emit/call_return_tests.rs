use super::*;

fn program(ty: &str) -> Mir65816Program {
    let source = format!(
        "{ty} FUNC Echo({ty} x) RETURN(x) {ty} FUNC Forward({ty} x) RETURN(Echo(x)) PROC Main() RETURN"
    );
    let ast = crate::parser::parse(&crate::lexer::tokenize(&source).unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(
        &ast,
        crate::semantic::SemanticOptions::modern()
            .with_target(crate::target::TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    crate::nir::verify_program(&nir).unwrap();
    crate::mir65816::lower_program(&nir).unwrap()
}

fn builder(routine: &Mir65816Routine) -> Builder<'_> {
    let frame = AllocatedFrame::new(routine).unwrap();
    Builder {
        routine,
        code: TrackedEmitter65816::for_test(&frame),
        frame,
        blocks: BTreeMap::new(),
        next_block: None,
        loop_x: None,
    }
}

#[test]
fn immediate_call_returns_remove_only_the_capture_and_reload() {
    for (ty, saving) in [("BYTE", 15), ("CARD", 4), ("INT", 4)] {
        let p = program(ty);
        let r = &p.routines[1];
        let block = &r.blocks[0];
        let call = block.ops.last().unwrap();
        let counts = liveness::input_counts(r);
        let Mir65816Terminator::Return { value, .. } = &block.terminator else {
            panic!()
        };
        let mut ordinary = builder(r);
        ordinary.operation(call).unwrap();
        ordinary.return_value(value.as_ref()).unwrap();
        let mut forwarded = builder(r);
        assert!(
            forwarded
                .call_return(call, &block.terminator, &counts)
                .unwrap()
        );
        forwarded.return_tail(true).unwrap();
        assert_eq!(ordinary.code.position() - forwarded.code.position(), saving);
        assert_eq!(
            format!("{:?}", ordinary.frame),
            format!("{:?}", forwarded.frame)
        );
        let [lo, hi] = forwarded.frame.extent.to_le_bytes();
        assert!(
            forwarded
                .code
                .code()
                .bytes
                .ends_with(&[0xa8, 0x3b, 0x18, 0x69, lo, hi, 0x1b, 0x98, 0x6b])
        );
    }
}

#[test]
fn call_return_refusals_leave_selection_unchanged() {
    let p = program("CARD");
    let r = &p.routines[1];
    let block = &r.blocks[0];
    for case in 0..8 {
        let mut call = block.ops.last().unwrap().clone();
        let mut term = block.terminator.clone();
        let mut counts = liveness::input_counts(r);
        let Mir65816Op::Call {
            target,
            result,
            plan,
            ..
        } = &mut call
        else {
            panic!()
        };
        let (dest, _) = result.unwrap();
        match case {
            0 => counts.insert(dest, 2),
            1 => {
                *target =
                    Mir65816CallTarget::Indirect(Mir65816Value::U24(0x041000), ByteSize::new(3));
                None
            }
            2 => {
                *result = None;
                None
            }
            3 => {
                plan.result = Some(Mir65816AbiHome::NativeResult(
                    abi::ResultLocation::A8ZeroExtended,
                ));
                None
            }
            4 => {
                let Mir65816Terminator::Return { value, .. } = &mut term else {
                    panic!()
                };
                *value = Some(Mir65816Value::U16(7));
                None
            }
            5 => {
                let Mir65816Terminator::Return { value, .. } = &mut term else {
                    panic!()
                };
                *value = Some(Mir65816Value::Temp(TempId(999), ByteSize::new(2)));
                None
            }
            6 => {
                let Mir65816Terminator::Return { value, .. } = &mut term else {
                    panic!()
                };
                *value = Some(Mir65816Value::Temp(dest, ByteSize::ONE));
                None
            }
            _ => {
                term = Mir65816Terminator::Fallthrough;
                None
            }
        };
        let mut b = builder(r);
        let before = format!("{:?}", b.code);
        assert!(!b.call_return(&call, &term, &counts).unwrap(), "{case}");
        assert_eq!(format!("{:?}", b.code), before, "{case}");
    }
    let mut r = r.clone();
    // A use in another (even unreachable) block disqualifies the capture.
    let mut duplicate = r.blocks[0].clone();
    duplicate.ops.clear();
    duplicate.id = crate::nir::BlockId(999);
    r.blocks.push(duplicate);
    let mut b = builder(&p.routines[1]);
    assert!(
        !b.call_return(
            block.ops.last().unwrap(),
            &block.terminator,
            &liveness::input_counts(&r)
        )
        .unwrap()
    );
}

#[test]
fn forwarded_calls_preflight_reserved_homes_and_native_contracts() {
    let p = program("CARD");
    let r = &p.routines[1];
    let block = &r.blocks[0];
    for case in 0..5 {
        let mut call = block.ops.last().unwrap().clone();
        let mut b = builder(r);
        let Mir65816Op::Call { result, plan, .. } = &mut call else {
            panic!()
        };
        let (id, _) = result.unwrap();
        match case {
            0 => {
                b.frame.temps.remove(&id);
            }
            1 => {
                b.frame.temps.insert(
                    id,
                    Location::Stack(Slot {
                        offset: 255,
                        width: 2,
                    }),
                );
            }
            2 => {
                plan.native = None;
            }
            3 => {
                plan.native.as_mut().unwrap().transfer = abi::FarTransfer::StackRtl;
            }
            _ => {
                b.code.test_delta(1);
            }
        }
        let before = format!("{:?}", b.code);
        assert!(
            b.call_return(&call, &block.terminator, &liveness::input_counts(r))
                .is_err(),
            "{case}"
        );
        assert_eq!(format!("{:?}", b.code), before, "{case}");
    }
}
