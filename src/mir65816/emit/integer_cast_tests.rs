use super::*;

fn program() -> Mir65816Program {
    let ast=crate::parser::parse(&crate::lexer::tokenize("LONGCARD FUNC Extend(CARD v) RETURN(LONGCARD(v)) LONGCARD FUNC Extend24(SIZE v) RETURN(LONGCARD(v)) SIZE FUNC Narrow(LONGCARD v) RETURN(SIZE(v)) CARD FUNC Word(LONGCARD v) RETURN(CARD(v)) PROC Main() RETURN").unwrap()).unwrap();
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

fn builder<'a>(r: &'a Mir65816Routine, op: &Mir65816Op, source: u16, dest: u16) -> Builder<'a> {
    let mut frame = AllocatedFrame::stack(r).unwrap();
    frame.extent = 256;
    let Mir65816Op::Cast {
        dest: id,
        from,
        to,
        value: Mir65816Value::Temp(input, _),
        ..
    } = op
    else {
        panic!()
    };
    frame.temps.insert(
        *input,
        Location::Stack(Slot {
            offset: source,
            width: from.get() as u8,
        }),
    );
    frame.temps.insert(
        *id,
        Location::Stack(Slot {
            offset: dest,
            width: to.get() as u8,
        }),
    );
    Builder {
        routine: r,
        code: TrackedEmitter65816::for_test(&frame),
        frame,
        blocks: BTreeMap::new(),
        next_block: None,
        loop_x: None,
        borrowed: BTreeMap::new(),
    }
}

#[test]
fn unsigned_casts_preflight_geometry_and_cost_before_any_write() {
    let p = program();
    let mut cases = 0;
    for r in &p.routines {
        for op in r.blocks.iter().flat_map(|b| &b.ops).filter(|op| {
            matches!(
                op,
                Mir65816Op::Cast {
                    kind: NirCastKind::Integer,
                    ..
                }
            )
        }) {
            cases += 1;
            let Mir65816Op::Cast { from, .. } = op else {
                unreachable!()
            };
            for (source, destination) in [(20, 40), (21, 41), (256 - from.get() as u16, 40)] {
                for byte in [false, true] {
                    let mut b = builder(r, op, source, destination);
                    if byte {
                        b.code.a8();
                    } else {
                        b.code.a16();
                    }
                    let start = b.code.position();
                    let selected = b.integer_cast(op).unwrap();
                    let Mir65816Op::Cast { from, to, .. } = op else {
                        unreachable!()
                    };
                    let fallback = usize::from(!byte) * 2
                        + from.get().min(to.get()) as usize * 4
                        + if to > from {
                            2 + 2 * (to.get() - from.get()) as usize
                        } else {
                            0
                        };
                    if selected {
                        assert!(b.code.position() - start < fallback);
                        use crate::mir65816::emit::effects::{Access, Memory as EffectMemory};
                        for record in b.code.recorded() {
                            let Action::Instruction { effects, .. } = &record.action else {
                                continue;
                            };
                            for access in &effects.memory {
                                if let EffectMemory::Stack {
                                    displacement,
                                    bytes,
                                } = access.memory
                                {
                                    let (begin, width) = if access.access == Access::Read {
                                        (source, from.get())
                                    } else {
                                        (destination, to.get())
                                    };
                                    assert!(displacement >= i32::from(begin));
                                    assert!(
                                        displacement as u32 + u32::from(bytes)
                                            <= u32::from(begin) + width
                                    );
                                }
                            }
                        }
                    } else {
                        assert_eq!(b.code.position(), start);
                    }
                }
            }
            let mut b = builder(r, op, 20, 21);
            b.code.a16();
            let before = b.code.code().bytes.clone();
            assert!(!b.integer_cast(op).unwrap());
            assert_eq!(b.code.code().bytes, before);
            let mut b = builder(r, op, 20, 20);
            b.code.a16();
            assert!(b.integer_cast(op).unwrap());
            for (source, dest) in [(0, 40), (255, 40), (20, 255)] {
                let mut b = builder(r, op, source, dest);
                let before = b.code.code().bytes.clone();
                assert!(b.integer_cast(op).is_err());
                assert_eq!(b.code.code().bytes, before);
            }
            let mut signed = op.clone();
            let Mir65816Op::Cast { from_signed, .. } = &mut signed else {
                unreachable!()
            };
            *from_signed = true;
            let mut b = builder(r, &signed, 20, 40);
            assert!(!b.integer_cast(&signed).unwrap());
            assert!(b.code.code().bytes.is_empty());
        }
    }
    assert_eq!(cases, 5);
}
