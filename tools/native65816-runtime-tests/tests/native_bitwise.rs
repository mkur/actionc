mod support;
use actionc::mir65816::{Mir65816Op, Mir65816Value, emit::proof, image, o65 as format};
use actionc_vm::native65816::{Access, Inputs, Machine};
use support::*;
fn source(ty: &str, width: usize) -> String {
    let mask = if width == 2 { 0x55aa } else { 0x55aa33cc };
    format!(
        r#"
{ty} a=$7100,b=$7104
{ty} ARRAY out=$7200
PROC Change() a=0 b=0 RETURN
{ty} FUNC Conjunction({ty} x,y) RETURN(x&y)
{ty} FUNC Disjunction({ty} x,y) RETURN(x%y)
{ty} FUNC Exclusive({ty} x,y) RETURN(x XOR y)
PROC Work({ty} x,y)
 {ty} saved
 out(0)=Conjunction(x,y) out(1)=Disjunction(x,y) out(2)=Exclusive(x,y)
 out(3)=x&{ty}(${mask:x}) out(4)={ty}(${mask:x})%x out(5)=x XOR {ty}(${mask:x})
 out(6)=(x&y)%(x XOR y) saved=x&y Change() out(7)=saved
 x=x XOR y out(8)=x&y out(9)=x%{ty}(${mask:x})
RETURN
PROC Main() Work(a,b) RETURN
"#
    )
}
fn results(h: &Harness, a: u32, b: u32, width: usize) {
    let mask = if width == 2 { 0x55aa } else { 0x55aa33cc };
    for (i, value) in [
        a & b,
        a | b,
        a ^ b,
        a & mask,
        mask | a,
        a ^ mask,
        a | b,
        a & b,
        (a ^ b) & b,
        (a ^ b) | mask,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            h.bus.value(0x7200 + (i * width) as u32, width),
            value,
            "{a:08x}/{b:08x}/{width}/{i}"
        );
    }
    assert_eq!(
        (h.bus.ram[0x71ff], h.bus.ram[0x7200 + 10 * width]),
        (0xa5, 0xa5)
    );
}
fn initialize(h: &mut Harness, a: u32, b: u32, width: usize, at: u32, bt: u32) {
    for (at, value) in [(at, a), (bt, b)] {
        h.bus.ram[at as usize..at as usize + width].copy_from_slice(&value.to_le_bytes()[..width]);
    }
    h.bus.ram[0x71ff..0x7201 + 10 * width].fill(0xa5);
}
#[test]
fn native_bitwise_preserves_signed_unsigned_lanes_constants_and_call_captures() {
    for (ty, width) in [("CARD", 2), ("INT", 2), ("LONGCARD", 4), ("LONGINT", 4)] {
        let mask = if width == 2 { 0xffff } else { u32::MAX };
        let values = [
            0,
            1,
            0xff,
            0xffff,
            0x10000,
            0x80000000,
            0xffff0000,
            u32::MAX,
        ]
        .map(|n| n & mask);
        let mut pairs: Vec<_> = values
            .into_iter()
            .flat_map(|a| values.map(|b| (a, b)))
            .collect();
        pairs.extend((0..8 * width).map(|i| (1 << i, mask ^ (1 << i))));
        let mut seed = 0x816cafeu32;
        for _ in 0..32 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let a = seed & mask;
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            pairs.push((a, seed & mask));
        }
        for optimize in [false, true] {
            let source = source(ty, width);
            let image = compile(&source, optimize);
            assert_eq!(
                image.to_json().unwrap(),
                compile(&source.replace('\n', "\r\n"), optimize)
                    .to_json()
                    .unwrap()
            );
            let caller = caller(image.entry);
            for &(a, b) in &pairs {
                for flags in [0, 4] {
                    let mut h = Harness::new(&image, &caller, flags);
                    initialize(&mut h, a, b, width, 0x7100, 0x7104);
                    h.run();
                    h.guards(flags);
                    results(&h, a, b, width);
                }
            }
        }
    }
}
#[test]
fn native_bitwise_matches_ca65_and_touches_only_complete_private_words() {
    for (ty, width) in [("CARD", 2), ("LONGCARD", 4)] {
        for optimize in [false, true] {
            let p = prepare(&source(ty, width), optimize);
            let c = p.compile(&layout()).unwrap();
            for (name, mnemonic) in [
                ("Conjunction", "and"),
                ("Disjunction", "ora"),
                ("Exclusive", "eor"),
            ] {
                let r = c
                    .machine
                    .prepared
                    .routines
                    .iter()
                    .find(|r| r.name == name)
                    .unwrap();
                let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
                let linked = c.image.routines.iter().find(|r| r.id == m.id.0).unwrap();
                let (block, i, dest, left, right) = r
                    .blocks
                    .iter()
                    .find_map(|block| {
                        block.ops.iter().enumerate().find_map(|(i, op)| match op {
                            Mir65816Op::Binary {
                                dest,
                                left: Mir65816Value::Temp(left, _),
                                right: Mir65816Value::Temp(right, _),
                                ..
                            } => Some((block.id, i, *dest, *left, *right)),
                            _ => None,
                        })
                    })
                    .unwrap();
                let (dest, left, right) = (
                    m.frame.temps[&dest].stack().unwrap().offset,
                    m.frame.temps[&left].stack().unwrap().offset,
                    m.frame.temps[&right].stack().unwrap().offset,
                );
                let span = &m.code.mir_spans[&(block, i)];
                let start = linked.address + span.start as u32;
                let end = linked.address + span.end as u32;
                let asm = (0..width)
                    .step_by(2)
                    .map(|n| {
                        format!(
                            "lda {},s\n{mnemonic} {},s\nsta {},s\n",
                            left + n as u16,
                            right + n as u16,
                            dest + n as u16
                        )
                    })
                    .collect::<String>();
                assert_eq!(&m.code.bytes[span.clone()], assemble(&asm, start));
                for flags in [0, 0x41] {
                    let a = if width == 2 { 0x8001u32 } else { 0x80012345 };
                    let b = if width == 2 { 0x55aau32 } else { 0xff00a55a };
                    let expected = match name {
                        "Conjunction" => a & b,
                        "Disjunction" => a | b,
                        _ => a ^ b,
                    };
                    let mut h = Harness::new(&c.image, &caller(c.image.entry), 0);
                    initialize(&mut h, a, b, width, 0x7100, 0x7104);
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
                    let mut before = h.cpu.registers();
                    before.p = (before.p & !0x41) | flags;
                    h.cpu = Machine::start_at(before);
                    let reads = h.bus.reads.len();
                    let writes = h.bus.writes.len();
                    assert!(
                        h.cpu
                            .run_until(
                                &mut h.bus,
                                1000,
                                |_| Inputs::default(),
                                |cpu| cpu.is_instruction_boundary() && cpu.pc() == end
                            )
                            .unwrap()
                    );
                    let stack = u32::from(before.s);
                    let expected_reads: Vec<_> = (0..width as u32)
                        .step_by(2)
                        .flat_map(|half| {
                            [
                                stack + u32::from(left) + half,
                                stack + u32::from(left) + half + 1,
                                stack + u32::from(right) + half,
                                stack + u32::from(right) + half + 1,
                            ]
                        })
                        .collect();
                    assert_eq!(
                        h.bus.reads[reads..]
                            .iter()
                            .copied()
                            .filter(|a| (0x4000..0x6000).contains(a))
                            .collect::<Vec<_>>(),
                        expected_reads
                    );
                    assert_eq!(
                        h.bus.writes[writes..],
                        (0..width)
                            .map(|i| (
                                stack + u32::from(dest) + i as u32,
                                expected.to_le_bytes()[i]
                            ))
                            .collect::<Vec<_>>()
                    );
                    let after = h.cpu.registers();
                    assert_eq!(after.p & 0x41, flags);
                    assert_eq!(
                        (after.x, after.y, after.d, after.s, after.dbr),
                        (before.x, before.y, before.d, before.s, before.dbr)
                    );
                }
            }
        }
    }
}
#[test]
fn native_bitwise_keeps_complete_volatile_bank_crossing_source_reads() {
    for (ty, width) in [("CARD", 2), ("LONGCARD", 4)] {
        for optimize in [false, true] {
            let source = format!(
                "VOLATILE {ty} io=$d000 {ty} ARRAY out=$7200 PROC Main() {ty} POINTER p p={ty} POINTER($12ffff) out(0)=p^&io out(1)=p^%io out(2)=p^ XOR io RETURN"
            );
            let mut p = prepare(&source, optimize);
            for op in p
                .mir
                .routines
                .iter_mut()
                .flat_map(|r| &mut r.blocks)
                .flat_map(|b| &mut b.ops)
            {
                if let Mir65816Op::Load {
                    width: w, volatile, ..
                } = op
                {
                    if w.get() == width as u32 {
                        *volatile = true;
                    }
                }
            }
            let image = p.compile(&layout()).unwrap().image;
            let mut h = Harness::new(&image, &caller(image.entry), 0);
            let (a, b) = if width == 2 {
                (0x8001u32, 0x55aau32)
            } else {
                (0x800100ff, 0xffff55aa)
            };
            h.bus.map(0xd000, &b.to_le_bytes()[..width], true);
            h.bus.map(0x12fffe, &[0xa5; 6], true);
            h.bus.ram[0x12ffff..0x12ffff + width].copy_from_slice(&a.to_le_bytes()[..width]);
            h.bus.watched.extend(0xd000..0xd000 + width as u32);
            h.bus.watched.extend(0x12fffe..0x130004);
            h.run();
            h.guards(0);
            for (i, v) in [a & b, a | b, a ^ b].into_iter().enumerate() {
                assert_eq!(h.bus.value(0x7200 + (width * i) as u32, width), v);
            }
            let expected: Vec<_> = (0..3)
                .flat_map(|_| {
                    (0x12ffff..0x12ffff + width as u32)
                        .chain(0xd000..0xd000 + width as u32)
                        .map(|a| (a, Access::Read))
                })
                .collect();
            assert_eq!(
                h.bus
                    .trace
                    .iter()
                    .map(|&(_, a, k)| (a, k))
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }
}
#[test]
fn native_bitwise_executes_after_independent_o65_relocation() {
    for (ty, width) in [("CARD", 2), ("LONGCARD", 4)] {
        for optimize in [false, true] {
            let source = source(ty, width).replace("a=$7100,b=$7104", "a,b");
            let bytes = o65::compile(&source, optimize, vec![]);
            for variant in 0..2 {
                let image = format::relocate(
                    &bytes,
                    &o65::placement(&bytes, variant, vec![o65::fault(variant)]),
                )
                .unwrap();
                for flags in [0, 4] {
                    let mut h = Harness::new_o65(&image, &caller(image.entry()), flags);
                    let (a, b) = if width == 2 {
                        (0x8001, 0x55aa)
                    } else {
                        (0x800100ff, 0xffff55aa)
                    };
                    initialize(
                        &mut h,
                        a,
                        b,
                        width,
                        o65::object(&image, "a"),
                        o65::object(&image, "b"),
                    );
                    h.run();
                    h.guards(flags);
                    results(&h, a, b, width);
                }
            }
        }
    }
}
#[test]
fn native_bitwise_replays_instruction_effects_and_linked_bytes_exactly() {
    for (ty, width) in [("CARD", 2), ("LONGCARD", 4)] {
        for optimize in [false, true] {
            let p = prepare(&source(ty, width), optimize);
            let c = p.compile(&layout()).unwrap();
            let (a, old) = proof::materialize_reference(&p.mir, true).unwrap();
            let (b, new) = proof::materialize_replayed(&p.mir, true).unwrap();
            assert_eq!(
                image::link(&p.mir, &a, &layout())
                    .unwrap()
                    .to_json()
                    .unwrap(),
                c.image.to_json().unwrap()
            );
            for ((a, b), (old, new)) in a.routines.iter().zip(&b.routines).zip(old.iter().zip(&new))
            {
                proof::compare_replay_output(&a.code, &b.code).unwrap();
                assert_eq!(old.snapshots, new.snapshots);
            }
        }
    }
}
