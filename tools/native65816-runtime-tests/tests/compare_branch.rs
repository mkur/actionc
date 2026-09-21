mod support;
use actionc::mir65816::{
    self,
    image::{AssemblyImport, Image},
    *,
};
use actionc::nir::runtime_symbol_id;
use actionc::nir::{BlockId, NirBinaryOp, NirCompareOp, TempId};
use actionc::target::ByteSize;
use actionc_vm::native65816::Access;
use support::*;

#[test]
fn conditional_word_results_distinguish_all_relations_and_boundary_pairs() {
    let values = [0u16, 1, 0xff, 0x100, 0x7fff, 0x8000, 0xffff];
    for ty in ["CARD", "INT", "BYTE", "LONGCARD"] {
        let mut source = String::from("CARD a=$7100,b=$7102\nCARD ARRAY out=$7200\n");
        for (i, op) in ["=", "#", "<", "<=", ">", ">="].iter().enumerate() {
            source.push_str(&format!(
                "CARD FUNC F{i}({ty} x,y) IF x{op}y THEN RETURN($A55A) FI RETURN($5AA5)\n"
            ));
        }
        source.push_str("PROC Main()\n");
        for i in 0..6 {
            source.push_str(&format!("out({i})=F{i}({ty}(a),{ty}(b))\n"));
        }
        source.push_str("RETURN\n");
        for optimize in [false, true] {
            let prepared = prepare(&source, optimize);
            // Verify the actual adjacent shape survives both frontend modes.
            for r in prepared
                .mir
                .routines
                .iter()
                .filter(|r| r.name.starts_with('F'))
            {
                assert!(r.blocks.iter().any(|b| matches!((&b.ops.last(), &b.terminator),
                    (Some(Mir65816Op::Compare { dest, .. }), Mir65816Terminator::Branch { condition: Mir65816Value::Temp(id, _), .. }) if dest == id)));
            }
            let image = compile(&source, optimize);
            assert_eq!(
                image.to_json().unwrap(),
                compile(&source.replace('\n', "\r\n"), optimize)
                    .to_json()
                    .unwrap()
            );
            let caller = caller(image.entry);
            let interpret = |v: u16| match ty {
                "INT" => i32::from(v as i16),
                "BYTE" => i32::from(v as u8),
                _ => i32::from(v),
            };
            for a in values {
                for b in values {
                    for mask in [0, 4] {
                        let mut h = Harness::new(&image, &caller, mask);
                        h.bus.ram[0x7100..0x7102].copy_from_slice(&a.to_le_bytes());
                        h.bus.ram[0x7102..0x7104].copy_from_slice(&b.to_le_bytes());
                        h.run();
                        h.guards(mask);
                        let (x, y) = (interpret(a), interpret(b));
                        for (i, truth) in [x == y, x != y, x < y, x <= y, x > y, x >= y]
                            .into_iter()
                            .enumerate()
                        {
                            assert_eq!(
                                h.bus.value(0x7200 + 2 * i as u32, 2),
                                if truth { 0xa55a } else { 0x5aa5 },
                                "{ty}/{optimize}/{a}/{b}/{i}"
                            );
                        }
                    }
                }
            }
        }
    }
}

// Verified MIR ensures nonempty parallel edges survive optimization. The source
// frontend currently does not retain this shape for simple branch expressions.
fn edge_program(optimize: bool, backedge: bool) -> actionc::compiler::native65816::Prepared {
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

#[test]
fn conditional_parallel_edges_and_backedge_rotations_execute() {
    for optimize in [false, true] {
        for backedge in [false, true] {
            let p = edge_program(optimize, backedge);
            let compiled = p.compile(&layout()).unwrap();
            let image = image::Image::from_json(&compiled.image.to_json().unwrap()).unwrap();
            let caller = caller(image.entry);
            for (a, b) in [(0u16, 0u16), (0xffff, 1), (1, 0xffff), (0x8000, 0x7fff)] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller, mask);
                    h.bus.ram[0x7100..0x7102].copy_from_slice(&a.to_le_bytes());
                    h.bus.ram[0x7102..0x7104].copy_from_slice(&b.to_le_bytes());
                    h.run();
                    h.guards(mask);
                    let expected = if backedge || a < b {
                        b.wrapping_sub(a)
                    } else {
                        a.wrapping_sub(b)
                    };
                    assert_eq!(h.bus.value(0x7200, 2), u32::from(expected));
                }
            }
        }
    }
}

#[test]
fn branch_inputs_preserve_volatile_alias_and_call_clobber_boundaries() {
    let source = r#"
MODULE TEST
PUBLIC EXTERNAL PROC Smash()
VOLATILE CARD io=$D000
BYTE ARRAY out=$7200
PROC Main()
  CARD POINTER p
  CARD saved,observed
  BYTE flag
  PROC POINTER cb
  p=CARD POINTER($12FFFF) cb=@Smash
  saved=p^ observed=io flag=(saved<CARD($8000))
  Smash()
  out(0)=flag IF saved<CARD($8000) THEN out(2)=1 ELSE out(2)=0 FI IF saved=observed THEN out(4)=1 ELSE out(4)=0 FI
  IF p^=CARD($1234) THEN out(6)=1 ELSE out(6)=0 FI IF io=observed THEN out(8)=1 ELSE out(8)=0 FI
  p^=CARD($FFFF)
  cb()
  IF p^=CARD($1234) THEN out(10)=1 ELSE out(10)=0 FI IF saved#CARD($1234) THEN out(12)=1 ELSE out(12)=0 FI
RETURN
ENDMODULE
"#;
    let smash = assemble(
        r#"
        sep #$20
        .a8
        lda #$34
        sta f:$12ffff
        lda #$12
        sta f:$130000
        ldx #63
        lda #$a7
    again:
        sta 0,x
        dex
        bpl again
        rep #$20
        .a16
        lda #$9876
        ldx #$beef
        ldy #$dead
        sec
        rtl
    "#,
        0x041000,
    );
    for optimize in [false, true] {
        let prepared = prepare(source, optimize);
        let symbol = runtime_symbol_id("TEST.Smash");
        let signature = prepared
            .mir
            .routines
            .iter()
            .find(|r| r.entry.external_symbol == Some(symbol))
            .unwrap()
            .signature
            .0;
        let mut options = layout();
        options.imports.push(AssemblyImport {
            symbol: symbol.0,
            signature,
            abi: abi::generated::ABI_NAME.into(),
            address: 0x041000,
            size: smash.len() as u32,
            stack_peak: 0,
            checks_stack: true,
            irq_effect: Default::default(),
        });
        let image = Image::from_json(&prepared.compile(&options).unwrap().image.to_json().unwrap())
            .unwrap();
        let caller = caller(image.entry);
        for value in [0u16, 0x1234, 0x8000, 0xffff] {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                h.bus.map(0x041000, &smash, false);
                h.bus.map(0xd000, &value.to_le_bytes(), true);
                h.bus.watched.extend(0xd000..0xd002);
                let [lo, hi] = value.to_le_bytes();
                h.bus.map(0x12fffe, &[0xa5, lo, hi, 0x5a], true);
                h.bus.ram[0x7200..0x720e].fill(0xa5);
                h.run();
                h.guards(mask);
                for (i, truth) in [
                    value < 0x8000,
                    value < 0x8000,
                    true,
                    true,
                    true,
                    true,
                    value != 0x1234,
                ]
                .into_iter()
                .enumerate()
                {
                    assert_eq!(h.bus.ram[0x7200 + 2 * i], u8::from(truth));
                    assert_eq!(h.bus.ram[0x7201 + 2 * i], 0xa5);
                }
                assert_eq!(&h.bus.ram[0x12fffe..0x130002], &[0xa5, 0x34, 0x12, 0x5a]);
                assert_eq!(
                    h.bus
                        .trace
                        .iter()
                        .map(|&(_, a, access)| (a, access))
                        .collect::<Vec<_>>(),
                    [
                        (0xd000, Access::Read),
                        (0xd001, Access::Read),
                        (0xd000, Access::Read),
                        (0xd001, Access::Read)
                    ]
                );
                assert!((0x2000..0x2040).all(|a| h.bus.writes.contains(&(a, 0xa7))));
            }
        }
    }
}
