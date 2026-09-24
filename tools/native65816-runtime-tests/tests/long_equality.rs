mod support;
use actionc::mir65816::{abi, image::AssemblyImport, o65 as format};
use actionc_vm::native65816::Access;
use support::*;

fn source(ty: &str) -> String {
    format!(
        r#"
{ty} a=$7100,b=$7104
BYTE ARRAY out=$7200
BYTE FUNC Equal({ty} x,y) RETURN(x=y)
BYTE FUNC NotEqual({ty} x,y) RETURN(x#y)
PROC Save(BYTE value) out(16)=value RETURN
PROC Work({ty} x,y)
  BYTE saved
  out(0)=(x=y) out(2)=(x#y)
  IF x=y THEN out(4)=1 ELSE out(4)=0 FI
  IF x#y THEN out(6)=1 ELSE out(6)=0 FI
  out(8)=(x={ty}(0)) out(10)=({ty}(0)#x)
  saved=(x#y) out(12)=Equal(x,y) out(14)=saved Save(saved)
  IF x={ty}(0) THEN out(18)=1 ELSE out(18)=0 FI
  IF {ty}(0)#x THEN out(20)=1 ELSE out(20)=0 FI
  out(22)=(x={ty}($80010001)) out(24)=NotEqual(x,y)
  out(26)=(x<y) out(28)=(x>=y)
  out(30)=({ty}(0)=x) out(32)=(x#{ty}(0))
  x={ty}(0) out(34)=(x=y)
RETURN
PROC Main() Work(a,b) RETURN
"#
    )
}

fn check_results(h: &Harness, a: u32, b: u32, signed: bool) {
    let less = if signed {
        (a as i32) < (b as i32)
    } else {
        a < b
    };
    for (i, truth) in [
        a == b,
        a != b,
        a == b,
        a != b,
        a == 0,
        a != 0,
        a == b,
        a != b,
        a != b,
        a == 0,
        a != 0,
        a == 0x80010001,
        a != b,
        less,
        !less,
        a == 0,
        a != 0,
        b == 0,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            h.bus.ram[0x7200 + 2 * i],
            u8::from(truth),
            "{a:08x}/{b:08x}/{signed}/{i}"
        );
        assert_eq!(h.bus.ram[0x7201 + 2 * i], 0xa5);
    }
}

#[test]
fn long_equality_covers_halves_sign_bits_zero_and_all_boolean_consumers() {
    let values = [
        0u32, 1, 0xff, 0x100, 0xffff, 0x10000, 0x10001, 0x7fffffff, 0x80000000, 0x80010001,
        0xff000000, 0xffffffff,
    ];
    let mut pairs: Vec<_> = values
        .into_iter()
        .flat_map(|a| values.map(|b| (a, b)))
        .collect();
    // Every individual bit must contribute to equality and zero decisions.
    for bit in 0..32 {
        pairs.extend([(1 << bit, 0), (0, 1 << bit), (1 << bit, 1 << bit)]);
    }
    let mut seed = 0x816e9a71u32;
    for _ in 0..32 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let a = seed;
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        pairs.extend([(a, seed), (a, a), (a, a ^ 0x10000)]);
    }
    for ty in ["LONGCARD", "LONGINT"] {
        let source = source(ty);
        for optimize in [false, true] {
            narrow_comparison::check_shape(&source, optimize, 4);
            let image = compile(&source, optimize);
            assert_eq!(
                image.to_json().unwrap(),
                compile(&source.replace('\n', "\r\n"), optimize)
                    .to_json()
                    .unwrap()
            );
            let caller = caller(image.entry);
            for &(a, b) in &pairs {
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller, mask);
                    h.bus.ram[0x7100..0x7104].copy_from_slice(&a.to_le_bytes());
                    h.bus.ram[0x7104..0x7108].copy_from_slice(&b.to_le_bytes());
                    h.bus.ram[0x7200..0x7224].fill(0xa5);
                    h.run();
                    h.guards(mask);
                    check_results(&h, a, b, ty == "LONGINT");
                    if a == 0x10000 && b == 0 && mask == 0 {
                        narrow_comparison::record(
                            &format!("long-equality-{ty}"),
                            optimize,
                            &source,
                            &image,
                            &h,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn long_equality_preserves_volatile_bank_crossing_loads_and_call_clobbers() {
    let source = r#"
MODULE TEST
PUBLIC EXTERNAL PROC Smash()
VOLATILE LONGCARD io=$D000
BYTE ARRAY out=$7200
PROC Main()
  LONGCARD POINTER cell
  LONGCARD saved,observed
  BYTE flag
  PROC POINTER cb
  cell=LONGCARD POINTER($12FFFF) cb=@Smash
  saved=cell^ observed=io flag=(saved=observed)
  Smash() out(0)=flag out(2)=(saved=observed) out(4)=(io=observed)
  IF cell^=LONGCARD($12340034) THEN out(6)=1 ELSE out(6)=0 FI
  cell^=LONGCARD($FFFFFFFF) cb() out(8)=(cell^=LONGCARD($12340034))
  IF saved#observed THEN out(10)=1 ELSE out(10)=0 FI
RETURN
ENDMODULE
"#;
    let smash = assemble(
        "sep #$20\n.a8\nlda #$34\nsta f:$12ffff\nlda #0\nsta f:$130000\nlda #$34\nsta f:$130001\nlda #$12\nsta f:$130002\nldx #63\nlda #$a7\nagain: sta 0,x\ndex\nbpl again\nrep #$20\n.a16\nlda #$9876\nldx #$beef\nldy #$dead\nsep #$41\nrtl",
        0x041000,
    );
    for optimize in [false, true] {
        let p = prepare(source, optimize);
        let symbol = actionc::nir::runtime_symbol_id("TEST.Smash");
        let signature = p
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
        let image = p.compile(&options).unwrap().image;
        let caller = caller(image.entry);
        for value in [0u32, 0x10000, 0x80000000, 0xffffffff] {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                h.bus.map(0x041000, &smash, false);
                h.bus.map(0xd000, &value.to_le_bytes(), true);
                h.bus.map(
                    0x12fffe,
                    &[
                        0xa5,
                        value as u8,
                        (value >> 8) as u8,
                        (value >> 16) as u8,
                        (value >> 24) as u8,
                        0x5a,
                    ],
                    true,
                );
                h.bus.watched.extend(0xd000..0xd004);
                h.bus.watched.extend(0x12ffff..0x130003);
                h.bus.ram[0x7200..0x720c].fill(0xa5);
                h.run();
                h.guards(mask);
                for (i, result) in [1, 1, 1, 1, 1, 0].into_iter().enumerate() {
                    assert_eq!(h.bus.ram[0x7200 + 2 * i], result);
                    assert_eq!(h.bus.ram[0x7201 + 2 * i], 0xa5);
                }
                let read_trace: Vec<_> = h
                    .bus
                    .trace
                    .iter()
                    .filter(|(_, _, access)| *access == Access::Read)
                    .map(|&(_, a, _)| a)
                    .collect();
                let expected: Vec<_> = (0x12ffff..0x130003)
                    .chain(0xd000..0xd004)
                    .chain(0xd000..0xd004)
                    .chain(0x12ffff..0x130003)
                    .chain(0x12ffff..0x130003)
                    .collect();
                assert_eq!(read_trace, expected);
                assert_eq!(h.bus.ram[0x12fffe], 0xa5);
                assert_eq!(h.bus.ram[0x130003], 0x5a);
                assert!((0x2000..0x2040).all(|a| h.bus.writes.contains(&(a, 0xa7))));
            }
        }
    }
}

#[test]
fn long_equality_and_zero_tests_execute_after_o65_relocation() {
    for ty in ["LONGCARD", "LONGINT"] {
        let source = source(ty).replace("a=$7100,b=$7104", "a,b");
        for optimize in [false, true] {
            let bytes = o65::compile(&source, optimize, vec![]);
            for variant in 0..2 {
                let placement = o65::placement(&bytes, variant, vec![o65::fault(variant)]);
                let image = format::relocate(&bytes, &placement).unwrap();
                let caller = caller(image.entry());
                for (a, b) in [
                    (0u32, 0u32),
                    (0, 0x10000),
                    (0xffffffff, 0xffffffff),
                    (0x80000000, 0),
                    (0x80010001, 0x80000001),
                ] {
                    for mask in [0, 4] {
                        let mut h = Harness::new_o65(&image, &caller, mask);
                        for (name, value) in [("a", a), ("b", b)] {
                            let at = o65::object(&image, name) as usize;
                            h.bus.ram[at..at + 4].copy_from_slice(&value.to_le_bytes());
                        }
                        h.bus.ram[0x7200..0x7224].fill(0xa5);
                        h.run();
                        h.guards(mask);
                        check_results(&h, a, b, ty == "LONGINT");
                        o65::record(
                            &format!("long-equality-{ty}"),
                            optimize,
                            &bytes,
                            &placement,
                            &image,
                            h.cpu.cycles(),
                            None,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn long_equality_keeps_same_target_parallel_edges_and_loop_backedges() {
    use actionc::mir65816::{self, Mir65816Op, Mir65816Value};
    use actionc::nir::{NirCastKind, NirCompareOp, TempId};
    use actionc::target::ByteSize;
    let typed = prepare(
        "BYTE FUNC F(LONGCARD a,b) RETURN(a=b) PROC Main() RETURN",
        false,
    );
    let ty = typed.mir.routines[0]
        .temps
        .iter()
        .find(|(_, t)| t.width == Some(ByteSize::new(4)))
        .unwrap()
        .1
        .clone();
    for optimize in [false, true] {
        for backedge in [false, true] {
            let mut p = edges::program(optimize, backedge);
            let r = p
                .mir
                .routines
                .iter_mut()
                .find(|r| r.name == "Work")
                .unwrap();
            r.temps.push((TempId(100), ty.clone()));
            for block in &mut r.blocks {
                let Some(Mir65816Op::Compare { left, .. }) = block.ops.last() else {
                    continue;
                };
                let input = left.clone();
                let mut compare = block.ops.pop().unwrap();
                block.ops.push(Mir65816Op::Cast {
                    dest: TempId(100),
                    from: ByteSize::new(2),
                    from_signed: false,
                    to: ByteSize::new(4),
                    kind: NirCastKind::Integer,
                    value: input,
                });
                let Mir65816Op::Compare {
                    left,
                    right,
                    width,
                    operation,
                    ..
                } = &mut compare
                else {
                    unreachable!()
                };
                *left = Mir65816Value::Temp(TempId(100), ByteSize::new(4));
                *right = Mir65816Value::U32(0);
                *width = ByteSize::new(4);
                *operation = if backedge {
                    NirCompareOp::Ne
                } else {
                    NirCompareOp::Eq
                };
                block.ops.push(compare);
            }
            mir65816::verify_program(&p.mir).unwrap();
            let image = p.compile(&layout()).unwrap().image;
            let caller = caller(image.entry);
            for (a, b) in [(0u16, 41u16), (0x100, 0x1234), (0xffff, 1)] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller, mask);
                    h.bus.ram[0x7100..0x7102].copy_from_slice(&a.to_le_bytes());
                    h.bus.ram[0x7102..0x7104].copy_from_slice(&b.to_le_bytes());
                    h.run();
                    h.guards(mask);
                    assert_eq!(
                        h.bus.value(0x7200, 2),
                        u32::from(if backedge || a == 0 {
                            b.wrapping_sub(a)
                        } else {
                            a.wrapping_sub(b)
                        })
                    );
                }
            }
        }
    }
}

#[test]
fn native_long_comparisons_match_ca65_and_touch_only_captured_word_parts() {
    use actionc::mir65816::{Mir65816Op, Mir65816Value};
    use actionc_vm::native65816::Inputs;
    for optimize in [false, true] {
        let p = prepare(&source("LONGCARD"), optimize);
        let compiled = p.compile(&layout()).unwrap();
        for name in ["Equal", "NotEqual"] {
            let r = p.mir.routines.iter().find(|r| r.name == name).unwrap();
            let m = compiled
                .machine
                .routines
                .iter()
                .find(|m| m.id == r.id)
                .unwrap();
            let linked = compiled
                .image
                .routines
                .iter()
                .find(|m| m.id == r.id.0)
                .unwrap();
            let (block, index, dest, left, right) = r
                .blocks
                .iter()
                .find_map(|b| {
                    b.ops.iter().enumerate().find_map(|(i, op)| match op {
                        Mir65816Op::Compare {
                            dest,
                            left: Mir65816Value::Temp(left, _),
                            right: Mir65816Value::Temp(right, _),
                            ..
                        } => Some((b.id, i, *dest, *left, *right)),
                        _ => None,
                    })
                })
                .unwrap();
            let span = &m.code.mir_spans[&(block, index)];
            let start = linked.address + span.start as u32;
            let end = linked.address + span.end as u32;
            let left = m.frame.temps[&left].stack().unwrap().offset;
            let right = m.frame.temps[&right].stack().unwrap().offset;
            let dest = m.frame.temps[&dest].stack().unwrap().offset;
            let caller = caller(compiled.image.entry);
            for (a, b) in [
                (0u32, 0u32),
                (0x80000000, 0x00000000),
                (0x12345678, 0x12345679),
                (0xffffffff, 0xffffffff),
            ] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&compiled.image, &caller, mask);
                    h.bus.ram[0x7100..0x7104].copy_from_slice(&a.to_le_bytes());
                    h.bus.ram[0x7104..0x7108].copy_from_slice(&b.to_le_bytes());
                    assert!(
                        h.cpu
                            .run_until(
                                &mut h.bus,
                                100_000,
                                |_| Inputs::default(),
                                |cpu| cpu.is_instruction_boundary() && cpu.pc() == start
                            )
                            .unwrap()
                    );
                    let before = h.cpu.registers();
                    assert_eq!(before.p & 0x20, 0);
                    let unequal = if name == "Equal" { "no" } else { "yes" };
                    let branch = if name == "Equal" { "beq" } else { "bne" };
                    // Independently assembled relaxed conditional transfers, including
                    // the conservative mode setup at each materialization join.
                    let reference = assemble(
                        &format!(
                            "lda {left},s\ncmp {right},s\nbne {unequal}\nlda {},s\ncmp {},s\n{branch} yes\n{}sep #$20\n.a8\nlda #0\nbra done\nyes:\nsep #$20\nlda #1\ndone:\nsep #$20\nsta {dest},s\n",
                            left + 2,
                            right + 2,
                            if name == "Equal" { "no:\n" } else { "" }
                        ),
                        start,
                    );
                    assert_eq!(&h.bus.ram[start as usize..end as usize], reference);
                    let reads = h.bus.reads.len();
                    let writes = h.bus.writes.len();
                    let s = u32::from(before.s);
                    assert!(
                        h.cpu
                            .run_until(
                                &mut h.bus,
                                1_000,
                                |_| Inputs::default(),
                                |cpu| cpu.is_instruction_boundary() && cpu.pc() == end
                            )
                            .unwrap()
                    );
                    let mut expected = vec![
                        s + u32::from(left),
                        s + u32::from(left) + 1,
                        s + u32::from(right),
                        s + u32::from(right) + 1,
                    ];
                    if a as u16 == b as u16 {
                        expected.extend([
                            s + u32::from(left) + 2,
                            s + u32::from(left) + 3,
                            s + u32::from(right) + 2,
                            s + u32::from(right) + 3,
                        ]);
                    }
                    assert_eq!(
                        h.bus.reads[reads..]
                            .iter()
                            .copied()
                            .filter(|a| (0x4000..0x6000).contains(a))
                            .collect::<Vec<_>>(),
                        expected
                    );
                    assert_eq!(
                        &h.bus.writes[writes..],
                        &[(
                            s + u32::from(dest),
                            u8::from(if name == "Equal" { a == b } else { a != b })
                        )]
                    );
                    assert!(
                        !h.bus.reads[reads..]
                            .iter()
                            .any(|a| (0x2000..0x2040).contains(a))
                    );
                    h.run();
                    h.guards(mask);
                }
            }
        }
    }
}
