use super::*;

fn routine() -> Mir65816Routine {
    let source = "PROC Touch() RETURN BYTE FUNC Read(BYTE POINTER p) BYTE v v=p^ Touch() RETURN(v)";
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(
        &ast,
        crate::semantic::SemanticOptions::modern()
            .with_target(crate::target::TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    crate::nir::verify_program(&nir).unwrap();
    crate::mir65816::lower_program(&nir)
        .unwrap()
        .routines
        .into_iter()
        .find(|r| r.name == "Read")
        .unwrap()
}

#[test]
fn adjacent_pointer_binding_keeps_authoritative_identity_and_reserved_homes() {
    let r = routine();
    let frame = AllocatedFrame::new(&r).unwrap();
    let plan = Plan::new(&r, &frame).unwrap();
    assert_eq!(plan.bindings.len(), 1);
    let binding = &plan.bindings[0];
    assert_eq!(binding.source.parameter, r.frame.parameters[0].param);
    assert_eq!(binding.source.home.width, 3);
    let machine = super::super::routine(&r, false).unwrap();
    assert_eq!(format!("{:?}", frame), format!("{:?}", machine.frame));
    assert!(machine.code.mir_spans[&binding.definition].is_empty());
    assert!(!machine.code.mir_spans[&(binding.definition.0, binding.definition.1 + 1)].is_empty());
}

#[test]
fn unsafe_or_incomplete_adjacent_bindings_retain_the_capture() {
    for problem in 0..9 {
        let mut r = routine();
        let original = Plan::new(&r, &AllocatedFrame::new(&r).unwrap()).unwrap();
        let binding = &original.bindings[0];
        let temp = binding.temp;
        let def = binding.definition.1;
        let consumer = r.blocks[0].ops[def + 1].clone();
        let mut frame = AllocatedFrame::new(&r).unwrap();
        match problem {
            0 => {
                if let Mir65816Op::Load { volatile, .. } = &mut r.blocks[0].ops[def] {
                    *volatile = true
                }
            }
            1 => {
                if let Mir65816Op::Load { volatile, .. } = &mut r.blocks[0].ops[def + 1] {
                    *volatile = true
                }
            }
            2 => {
                r.blocks[0].ops.push(consumer);
            }
            3 => {
                let call = r.blocks[0]
                    .ops
                    .iter()
                    .find(|op| matches!(op, Mir65816Op::Call { .. }))
                    .unwrap()
                    .clone();
                r.blocks[0].ops.insert(def + 1, call);
            }
            4 => {
                frame.temps.insert(
                    temp,
                    Location::DirectPage(Slot {
                        offset: 32,
                        width: 3,
                    }),
                );
            }
            5 => {
                if let Mir65816Op::Load { address, .. } = &mut r.blocks[0].ops[def] {
                    address.displacement = ByteOffset::new(1)
                }
            }
            6 => {
                r.frame.parameters[0].frame_object = Some(Mir65816FrameObjectId(999));
            }
            7 => {
                let mut bad = r.blocks[0].ops[def].clone();
                if let Mir65816Op::Load { width, .. } = &mut bad {
                    *width = ByteSize::new(2)
                }
                r.blocks[0].ops.push(bad);
            }
            _ => {
                let Mir65816Op::Load { address, .. } = &r.blocks[0].ops[def] else {
                    unreachable!()
                };
                let address = address.clone();
                r.blocks[0].ops.push(Mir65816Op::AddressOf {
                    dest: temp,
                    address,
                    width: ByteSize::new(3),
                });
            }
        }
        assert!(
            Plan::new(&r, &frame).unwrap().bindings.is_empty(),
            "refusal {problem}"
        );
    }
}

#[test]
fn incoming_pointer_preflight_checks_the_last_byte_at_255() {
    let r = routine();
    let mut f = AllocatedFrame::new(&r).unwrap();
    f.extent = 249;
    let plan = Plan::new(&r, &f).unwrap();
    assert_eq!(plan.bindings[0].source.home.offset, 253);
    f.extent = 250;
    assert!(Plan::new(&r, &f).is_err());
}
