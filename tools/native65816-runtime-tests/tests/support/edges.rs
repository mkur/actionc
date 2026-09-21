//! Construct verified nonempty edges independently of frontend optimization.
use super::*;
use actionc::mir65816::{self, *};
use actionc::nir::{BlockId, NirBinaryOp, NirCompareOp, TempId};
use actionc::target::ByteSize;

// Verified MIR ensures nonempty parallel edges survive optimization. The source
// frontend currently does not retain this shape for simple branch expressions.
pub fn program(optimize: bool, backedge: bool) -> actionc::compiler::native65816::Prepared {
    let mut p = prepare(
        "CARD a=$7100,b=$7102 CARD out=$7200 CARD FUNC Work(CARD x,y) IF x<y THEN RETURN(x) FI RETURN(y) PROC Main() out=Work(a,b) RETURN",
        optimize,
    );
    let r = p
        .mir
        .routines
        .iter_mut()
        .find(|r| r.name == "Work")
        .unwrap();
    let word = ByteSize::new(2);
    let word_ty = r
        .temps
        .iter()
        .find(|(id, _)| *id == TempId(0))
        .unwrap()
        .1
        .clone();
    let (bool_id, bool_ty) = r
        .temps
        .iter()
        .find(|(id, _)| {
            r.blocks
                .iter()
                .flat_map(|b| &b.ops)
                .any(|op| matches!(op, Mir65816Op::Compare { dest, .. } if dest == id))
        })
        .unwrap()
        .clone();
    let _ = bool_id;
    let mut loads: Vec<_> = r.blocks[0]
        .ops
        .iter()
        .filter(|op| matches!(op, Mir65816Op::Load { .. }))
        .take(2)
        .cloned()
        .collect();
    assert_eq!(loads.len(), 2);
    for (i, op) in loads.iter_mut().enumerate() {
        let Mir65816Op::Load { dest, .. } = op else {
            unreachable!()
        };
        *dest = TempId(i as u32);
    }
    let mut ret = r
        .blocks
        .iter()
        .find_map(|b| {
            matches!(b.terminator, Mir65816Terminator::Return { .. }).then(|| b.terminator.clone())
        })
        .unwrap();
    let val = |id| Mir65816Value::Temp(TempId(id), word);
    let edge = |target, args| Mir65816Edge {
        target: BlockId(target),
        args,
    };
    r.temps = (0..10)
        .map(|id| {
            (
                TempId(id),
                if id == 8 {
                    bool_ty.clone()
                } else {
                    word_ty.clone()
                },
            )
        })
        .collect();
    let Mir65816Terminator::Return { value, .. } = &mut ret else {
        unreachable!()
    };
    *value = Some(val(9));
    loads.push(Mir65816Op::Compare {
        dest: TempId(8),
        width: word,
        signed: false,
        operation: NirCompareOp::Lt,
        left: val(0),
        right: val(1),
    });
    if backedge {
        loads.pop();
        r.blocks = vec![
            Mir65816Block {
                id: BlockId(0),
                params: vec![],
                ops: loads,
                terminator: Mir65816Terminator::Goto(edge(
                    1,
                    vec![val(0), val(1), Mir65816Value::U16(3)],
                )),
            },
            Mir65816Block {
                id: BlockId(1),
                params: vec![(TempId(2), word), (TempId(3), word), (TempId(4), word)],
                ops: vec![
                    Mir65816Op::Binary {
                        dest: TempId(5),
                        width: word,
                        signed: false,
                        operation: NirBinaryOp::Sub,
                        left: val(4),
                        right: Mir65816Value::U16(1),
                    },
                    Mir65816Op::Compare {
                        dest: TempId(8),
                        width: word,
                        signed: false,
                        operation: NirCompareOp::Ne,
                        left: val(4),
                        right: Mir65816Value::U16(0),
                    },
                ],
                terminator: Mir65816Terminator::Branch {
                    condition: Mir65816Value::Temp(TempId(8), ByteSize::ONE),
                    then_edge: edge(1, vec![val(3), val(2), val(5)]),
                    else_edge: edge(2, vec![val(2), val(3)]),
                },
            },
        ];
    } else {
        // Same target, different arguments: bypassing either trampoline is wrong.
        r.temps.retain(|(id, _)| ![2, 3, 4, 5].contains(&id.0));
        r.blocks = vec![Mir65816Block {
            id: BlockId(0),
            params: vec![],
            ops: loads,
            terminator: Mir65816Terminator::Branch {
                condition: Mir65816Value::Temp(TempId(8), ByteSize::ONE),
                then_edge: edge(2, vec![val(1), val(0)]),
                else_edge: edge(2, vec![val(0), val(1)]),
            },
        }];
    }
    r.blocks.push(Mir65816Block {
        id: BlockId(2),
        params: vec![(TempId(6), word), (TempId(7), word)],
        ops: vec![Mir65816Op::Binary {
            dest: TempId(9),
            width: word,
            signed: false,
            operation: NirBinaryOp::Sub,
            left: val(6),
            right: val(7),
        }],
        terminator: ret,
    });
    mir65816::verify_program(&p.mir).unwrap();
    p
}

/// A three-word cycle plus a countdown, repeated and unused parameters. The
/// original x stays live across the loop outside its block-parameter list.
pub fn rotation(optimize: bool, mixed: bool, ordinary: bool) -> native65816::Prepared {
    let mut p = program(optimize, true);
    let r = p
        .mir
        .routines
        .iter_mut()
        .find(|r| r.name == "Work")
        .unwrap();
    let word = ByteSize::new(2);
    let ty = r.temps.iter().find(|(id, _)| id.0 == 0).unwrap().1.clone();
    for id in [10, 11, 12, 13] {
        r.temps.push((TempId(id), ty.clone()));
    }
    let val = |id| Mir65816Value::Temp(TempId(id), word);
    let Mir65816Terminator::Goto(entry) = &mut r.blocks[0].terminator else {
        panic!()
    };
    entry
        .args
        .extend([Mir65816Value::U16(0xa55a), val(0), val(0)]);
    r.blocks[1]
        .params
        .extend([(TempId(10), word), (TempId(11), word), (TempId(12), word)]);
    let Mir65816Terminator::Branch { then_edge, .. } = &mut r.blocks[1].terminator else {
        panic!()
    };
    then_edge.args = vec![val(3), val(10), val(5), val(2), val(11), val(12)];
    // Three rotations give x,y,$A55A again. Retain an independent live-in x.
    r.blocks[2].ops.push(Mir65816Op::Binary {
        dest: TempId(13),
        width: word,
        signed: false,
        operation: NirBinaryOp::Add,
        left: val(9),
        right: val(0),
    });
    let Mir65816Terminator::Return { value, .. } = &mut r.blocks[2].terminator else {
        panic!()
    };
    *value = Some(val(13));
    if mixed {
        for (id, bytes, value) in [
            (14, 1, Mir65816Value::U8(0xf1)),
            (15, 3, Mir65816Value::U24(0xabcdef)),
            (16, 4, Mir65816Value::U32(0x89abcdef)),
        ] {
            let width = ByteSize::new(bytes);
            let mut t = ty.clone();
            t.width = Some(width);
            r.temps.push((TempId(id), t));
            r.blocks[1].params.push((TempId(id), width));
            let Mir65816Terminator::Goto(edge) = &mut r.blocks[0].terminator else {
                panic!()
            };
            edge.args.push(value);
            let Mir65816Terminator::Branch { then_edge, .. } = &mut r.blocks[1].terminator else {
                panic!()
            };
            then_edge.args.push(Mir65816Value::Temp(TempId(id), width));
        }
    }
    if ordinary {
        // Intervening computation preserves the Boolean and forces ordinary
        // dispatch instead of adjacent compare-to-branch fusion.
        r.temps.push((TempId(17), ty));
        r.blocks[1].ops.push(Mir65816Op::Binary {
            dest: TempId(17),
            width: word,
            signed: false,
            operation: NirBinaryOp::Add,
            left: val(0),
            right: val(1),
        });
    }
    mir65816::verify_program(&p.mir).unwrap();
    p
}

/// Insert a verified one-word edge after each captured word load. This keeps
/// external/volatile accesses and call ordering intact while forcing the private
/// capture to cross an edge in both frontend modes.
pub fn split_word_loads(p: &mut native65816::Prepared) {
    for r in &mut p.mir.routines {
        let mut next_temp = r.temps.iter().map(|(id, _)| id.0).max().unwrap_or(0) + 1;
        let mut next_block = r.blocks.iter().map(|b| b.id.0).max().unwrap_or(0) + 1;
        let mut blocks = vec![];
        for block in std::mem::take(&mut r.blocks) {
            let mut current = Mir65816Block {
                ops: vec![],
                ..block.clone()
            };
            for mut op in block.ops {
                if let Mir65816Op::Load { dest, width, .. } = &mut op {
                    if width.get() == 2 {
                        let original = *dest;
                        let width = *width;
                        let fresh = TempId(next_temp);
                        next_temp += 1;
                        *dest = fresh;
                        let ty = r
                            .temps
                            .iter()
                            .find(|(id, _)| *id == original)
                            .unwrap()
                            .1
                            .clone();
                        r.temps.push((fresh, ty));
                        current.ops.push(op);
                        let target = BlockId(next_block);
                        next_block += 1;
                        current.terminator = Mir65816Terminator::Goto(Mir65816Edge {
                            target,
                            args: vec![Mir65816Value::Temp(fresh, width)],
                        });
                        blocks.push(current);
                        current = Mir65816Block {
                            id: target,
                            params: vec![(original, width)],
                            ops: vec![],
                            terminator: block.terminator.clone(),
                        };
                        continue;
                    }
                }
                current.ops.push(op);
            }
            blocks.push(current);
        }
        r.blocks = blocks;
    }
    mir65816::verify_program(&p.mir).unwrap();
}
