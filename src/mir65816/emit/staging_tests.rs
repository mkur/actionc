use super::*;

fn routine(widths: &[u8]) -> Mir65816Routine {
    let mut r = word_tests::program().routines.remove(0);
    let ty = r.temps[0].1.clone();
    r.temps = widths
        .iter()
        .enumerate()
        .map(|(i, &w)| {
            let mut ty = ty.clone();
            ty.width = Some(ByteSize::new(w.into()));
            (TempId(i as u32), ty)
        })
        .collect();
    let params = widths
        .iter()
        .enumerate()
        .map(|(i, &w)| (TempId(i as u32), ByteSize::new(w.into())))
        .collect();
    let mut ret = r.blocks[0].terminator.clone();
    let Mir65816Terminator::Return { value, .. } = &mut ret else {
        unreachable!()
    };
    *value = Some(Mir65816Value::U16(0));
    r.blocks = vec![
        Mir65816Block {
            id: BlockId(0),
            params: vec![],
            ops: vec![],
            terminator: Mir65816Terminator::Goto(Mir65816Edge {
                target: BlockId(1),
                args: widths
                    .iter()
                    .map(|&w| {
                        if w == 2 {
                            Mir65816Value::U16(42)
                        } else {
                            Mir65816Value::Null(ByteSize::new(w.into()))
                        }
                    })
                    .collect(),
            }),
        },
        Mir65816Block {
            id: BlockId(1),
            params,
            ops: vec![],
            terminator: ret,
        },
    ];
    r
}
fn cycle(r: &mut Mir65816Routine) {
    let n = r.temps.len();
    r.blocks[1].terminator = Mir65816Terminator::Goto(Mir65816Edge {
        target: BlockId(1),
        args: (0..n)
            .map(|i| Mir65816Value::Temp(TempId(((i + 1) % n) as u32), ByteSize::new(2)))
            .collect(),
    });
}

#[test]
fn direct_edge_reservations_are_absent_for_both_parameter_home_kinds() {
    for mutable in [false, true] {
        let mut r = routine(&[2, 2]);
        if mutable {
            // Use a real mutable-parameter frame from lowering.
            let p = crate::parser::parse(
                &crate::lexer::tokenize("CARD FUNC F(CARD n) n==+1 RETURN(n)").unwrap(),
            )
            .unwrap();
            let m = crate::semantic::analyze_with_options(
                &p,
                crate::semantic::SemanticOptions::modern()
                    .with_target(crate::target::TargetId::Wdc65816Native),
            )
            .unwrap();
            let n = crate::nir::lower_program(&crate::semantic::ir::lower_program(&p, &m));
            crate::nir::verify_program(&n).unwrap();
            r.frame = super::super::super::lower_program(&n)
                .unwrap()
                .routines
                .remove(0)
                .frame;
            assert!(r.frame.parameters[0].frame_object.is_some());
        }
        let Mir65816Terminator::Goto(e) = &mut r.blocks[0].terminator else {
            unreachable!()
        };
        e.args[0] = Mir65816Value::Param(r.frame.parameters[0].param);
        let f = AllocatedFrame::stack(&r).unwrap();
        assert!(f.edge_copies.is_empty());
        f.verify_stack(&r).unwrap();
        let mut corrupt = f.clone();
        corrupt.edge_copies.push(Slot {
            offset: f.extent + 2,
            width: 2,
        });
        assert!(corrupt.verify_stack(&r).unwrap_err().contains("slot count"));
    }
}

#[test]
fn cyclic_reservations_are_exact_words_and_verified_independently() {
    let mut r = routine(&[2, 2, 2]);
    cycle(&mut r);
    let f = AllocatedFrame::stack(&r).unwrap();
    // Forward rotation only endangers the final source. Move 2 uses pool 0.
    assert_eq!(
        f.edge_copies,
        [Slot {
            offset: 8,
            width: 2
        }]
    );
    assert_eq!(f.extent, 10);
    for problem in 0..6 {
        let mut corrupt = f.clone();
        match problem {
            0 => corrupt.edge_copies.clear(),
            1 => corrupt.edge_copies[0].width = 1,
            2 => corrupt.edge_copies[0].width = 4,
            3 => corrupt.edge_copies.push(corrupt.edge_copies[0]),
            4 => corrupt.edge_copies[0].offset = f.temps[&TempId(0)].slot().offset,
            5 => corrupt.extent += 2,
            _ => unreachable!(),
        }
        assert!(corrupt.verify_stack(&r).is_err(), "{problem}");
    }
}

#[test]
fn cyclic_pointer_reservations_reuse_one_capture_and_keep_a_separate_save_word() {
    for count in [2, 3, 6] {
        let mut r = routine(&vec![3; count]);
        // The entry has no captures; only the loop determines staging needs.
        r.blocks[0].terminator = Mir65816Terminator::Exit;
        r.blocks[1].terminator = Mir65816Terminator::Goto(Mir65816Edge {
            target: BlockId(1),
            args: (0..count)
                .map(|i| {
                    Mir65816Value::Temp(
                        TempId(if count == 6 {
                            (i ^ 1) as u32
                        } else {
                            ((i + 1) % count) as u32
                        }),
                        ByteSize::new(3),
                    )
                })
                .collect(),
        });
        let f = AllocatedFrame::stack(&r).unwrap();
        assert_eq!(
            f.edge_copies.iter().map(|s| s.width).collect::<Vec<_>>(),
            [2, 3]
        );
        f.verify_stack(&r).unwrap();
        for problem in 0..5 {
            let mut bad = f.clone();
            match problem {
                0 => {
                    bad.edge_copies.pop();
                }
                1 => bad.edge_copies[1].width = 2,
                2 => bad.edge_copies[1].offset = bad.edge_copies[0].offset,
                3 => bad.edge_copies[1].offset = bad.temps[&TempId(0)].slot().offset,
                4 => bad.extent += 2,
                _ => unreachable!(),
            }
            assert!(bad.verify_stack(&r).is_err(), "{count}/{problem}");
        }
    }
}

#[test]
fn fallback_reserves_maximum_actual_width_per_argument_and_keeps_alignment() {
    let mut r = routine(&[1, 2, 3, 4]);
    let mut other = routine(&[4, 3, 2, 1]).blocks.remove(1);
    other.id = BlockId(2);
    for (i, (id, w)) in other.params.iter_mut().enumerate() {
        *id = TempId(4 + i as u32);
        let mut ty = r.temps[0].1.clone();
        ty.width = Some(*w);
        r.temps.push((*id, ty));
    }
    r.blocks[1].terminator = Mir65816Terminator::Goto(Mir65816Edge {
        target: BlockId(2),
        args: [4, 3, 2, 1]
            .into_iter()
            .map(|w| Mir65816Value::Null(ByteSize::new(w)))
            .collect(),
    });
    r.blocks.push(other);
    let f = AllocatedFrame::stack(&r).unwrap();
    assert_eq!(
        f.edge_copies.iter().map(|s| s.width).collect::<Vec<_>>(),
        [4, 3, 3, 4]
    );
    assert!(f.edge_copies.iter().all(|s| s.offset % 2 == 0));
    for w in [1, 2, 3, 4] {
        let mut r = routine(&[w]);
        let Mir65816Terminator::Goto(e) = &mut r.blocks[0].terminator else {
            unreachable!()
        };
        e.args[0] = Mir65816Value::Null(ByteSize::new(w.into()));
        let f = AllocatedFrame::stack(&r).unwrap();
        assert_eq!(f.edge_copies[0].width, w);
        f.verify_stack(&r).unwrap();
    }
}

#[test]
fn removal_avoids_provisional_overflow_but_keeps_incoming_and_fixed_frame_limits() {
    // Two word parameters: last byte is S+255 at extent 248. The old four-byte
    // reservations would fail long before this otherwise legal direct edge.
    let r = routine(&vec![2; 123]);
    let f = AllocatedFrame::stack(&r).unwrap();
    assert_eq!(f.extent, 248);
    assert!(f.edge_copies.is_empty());
    assert!(AllocatedFrame::stack(&routine(&vec![2; 124])).is_err());
    let mut r = routine(&vec![2; 63]);
    r.frame.parameters.clear();
    cycle(&mut r);
    let f = AllocatedFrame::stack(&r).unwrap();
    assert_eq!(f.extent, 130);
    let mut r = routine(&vec![2; 64]);
    r.frame.parameters.clear();
    cycle(&mut r);
    assert_eq!(AllocatedFrame::stack(&r).unwrap().extent, 132);
    // Reverse rotation needs n-1 captures, so 64 words really do overflow.
    for n in [63, 64] {
        let mut r = routine(&vec![2; n]);
        r.frame.parameters.clear();
        // Two frame-object bytes make the 63-word case exactly 254 bytes.
        r.frame.extent = ByteSize::new(2);
        cycle(&mut r);
        let Mir65816Terminator::Goto(e) = &mut r.blocks[1].terminator else {
            unreachable!()
        };
        e.args = (0..n)
            .map(|i| Mir65816Value::Temp(TempId(((i + n - 1) % n) as u32), ByteSize::new(2)))
            .collect();
        if n == 63 {
            let f = AllocatedFrame::stack(&r).unwrap();
            assert_eq!(f.extent, 254);
            assert_eq!(f.edge_copies.len(), 62);
        } else {
            assert!(AllocatedFrame::stack(&r).is_err());
        }
    }
}

#[test]
fn capture_ordinals_pack_sparse_moves_and_take_mixed_edge_maxima() {
    let mut r = routine(&[2; 5]);
    cycle(&mut r);
    let Mir65816Terminator::Goto(e) = &mut r.blocks[1].terminator else {
        unreachable!()
    };
    e.args = [1, 0, 2, 4, 3]
        .into_iter()
        .map(|id| Mir65816Value::Temp(TempId(id), ByteSize::new(2)))
        .collect();
    let f = AllocatedFrame::stack(&r).unwrap();
    let Mir65816Terminator::Goto(e) = &r.blocks[1].terminator else {
        unreachable!()
    };
    let plan = f.word_copies(&r, e, 0).unwrap().unwrap();
    assert_eq!(plan.captures().unwrap(), [1, 4]);
    assert_eq!(f.edge_copies.len(), 2);
    assert_eq!(
        f.word_staging(&plan, 0).unwrap(),
        [
            (1, f.edge_copies[0].offset as u8),
            (4, f.edge_copies[1].offset as u8)
        ]
    );
    let mut corrupt = f.clone();
    corrupt.edge_copies[1].offset = corrupt.edge_copies[0].offset;
    assert!(corrupt.verify_stack(&r).is_err());
    assert!(corrupt.word_staging(&plan, 0).is_err());
    let mut extra = routine(&[1, 3]).blocks.remove(1);
    extra.id = BlockId(2);
    for (i, (id, w)) in extra.params.iter_mut().enumerate() {
        *id = TempId(5 + i as u32);
        let mut ty = r.temps[0].1.clone();
        ty.width = Some(*w);
        r.temps.push((*id, ty));
    }
    let Mir65816Terminator::Goto(then_edge) = r.blocks[1].terminator.clone() else {
        unreachable!()
    };
    r.blocks[1].terminator = Mir65816Terminator::Branch {
        condition: Mir65816Value::U8(1),
        then_edge,
        else_edge: Mir65816Edge {
            target: BlockId(2),
            args: vec![Mir65816Value::U8(0), Mir65816Value::U24(0)],
        },
    };
    r.blocks.push(extra);
    let f = AllocatedFrame::stack(&r).unwrap();
    assert_eq!(
        f.edge_copies.iter().map(|s| s.width).collect::<Vec<_>>(),
        [2, 3]
    );
    assert!(f.edge_copies.iter().all(|s| s.offset % 2 == 0));
    f.verify_stack(&r).unwrap();
}
