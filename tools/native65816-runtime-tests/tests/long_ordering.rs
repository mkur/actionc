mod support;
use actionc::mir65816::{Mir65816Op, Mir65816Value, o65 as format};
use actionc_vm::native65816::{Access, Inputs};
use support::*;

const SIGN_SOURCE: &str = r#"
LONGINT input=$7100
BYTE ARRAY out=$7200
BYTE FUNC Negative(LONGINT x) RETURN(x<0)
BYTE FUNC Nonnegative(LONGINT x) RETURN(x>=0)
PROC Save(BYTE b) out(16)=b RETURN
PROC Main()
 LONGINT x
 BYTE saved
 x=input
 out(0)=(x<0) out(2)=(x>=0)
 out(4)=(0>x) out(6)=(0<=x)
 IF x<0 THEN out(8)=1 ELSE out(8)=0 FI
 IF 0<=x THEN out(10)=1 ELSE out(10)=0 FI
 saved=Negative(x) out(12)=saved out(14)=Nonnegative(x) Save(saved)
 out(18)=(x<=0) out(20)=(x>0)
 x=0 out(22)=(x<0) out(24)=(x>=0)
RETURN
"#;

fn sign_results(h: &Harness, value: u32) {
    let n = (value as i32) < 0;
    for (i, truth) in [
        n,
        !n,
        n,
        !n,
        n,
        !n,
        n,
        !n,
        n,
        (value as i32) <= 0,
        (value as i32) > 0,
        false,
        true,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            h.bus.ram[0x7200 + 2 * i],
            u8::from(truth),
            "{value:08x}/{i}"
        );
        assert_eq!(h.bus.ram[0x7201 + 2 * i], 0xa5);
    }
}

#[test]
fn long_sign_tests_cover_all_top_bytes_and_boolean_consumers() {
    let values: Vec<u32> = (0..256)
        .flat_map(|top| [(top << 24), (top << 24) | 0xffffff])
        .chain([0, 1, 0x7fffffff, 0x80000000, 0xffffffff])
        .collect();
    for optimize in [false, true] {
        let image = compile(SIGN_SOURCE, optimize);
        assert_eq!(
            image.to_json().unwrap(),
            compile(&SIGN_SOURCE.replace('\n', "\r\n"), optimize)
                .to_json()
                .unwrap()
        );
        let caller = caller(image.entry);
        for &value in &values {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                h.bus.ram[0x7100..0x7104].copy_from_slice(&value.to_le_bytes());
                h.bus.ram[0x7200..0x721a].fill(0xa5);
                h.run();
                h.guards(mask);
                sign_results(&h, value);
                if value == 0x80000000 && mask == 0 {
                    narrow_comparison::record("long-sign", optimize, SIGN_SOURCE, &image, &h);
                }
            }
        }
    }
}

#[test]
fn long_sign_tests_keep_complete_volatile_reads_and_mutation_across_calls() {
    let source = "LONGINT POINTER input BYTE ARRAY out=$7200 PROC Change() input^=0 RETURN PROC Main() LONGINT saved input=LONGINT POINTER($12ffff) saved=input^ out(0)=(saved<0) Change() out(2)=(saved>=0) out(4)=(input^<0) RETURN";
    for optimize in [false, true] {
        let mut p = prepare(source, optimize);
        for op in p
            .mir
            .routines
            .iter_mut()
            .flat_map(|r| &mut r.blocks)
            .flat_map(|b| &mut b.ops)
        {
            match op {
                Mir65816Op::Load {
                    width, volatile, ..
                }
                | Mir65816Op::Store {
                    width, volatile, ..
                } if width.get() == 4 => *volatile = true,
                _ => {}
            }
        }
        let image = p.compile(&layout()).unwrap().image;
        let caller = caller(image.entry);
        for value in [0u32, 1, 0x7fffffff, 0x80000000, 0xffffffff] {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                h.bus.map(0x12fffe, &[0xa5, 0, 0, 0, 0, 0xa5], true);
                h.bus.ram[0x12ffff..0x130003].copy_from_slice(&value.to_le_bytes());
                h.bus.watched.extend(0x12fffe..0x130004);
                h.run();
                h.guards(mask);
                assert_eq!(
                    [h.bus.ram[0x7200], h.bus.ram[0x7202], h.bus.ram[0x7204]],
                    [
                        u8::from((value as i32) < 0),
                        u8::from((value as i32) >= 0),
                        0
                    ]
                );
                let expected: Vec<_> = (0x12ffff..0x130003)
                    .map(|a| (a, Access::Read))
                    .chain((0x12ffff..0x130003).map(|a| (a, Access::Write(0))))
                    .chain((0x12ffff..0x130003).map(|a| (a, Access::Read)))
                    .collect();
                assert_eq!(
                    h.bus
                        .trace
                        .iter()
                        .map(|&(_, a, k)| (a, k))
                        .collect::<Vec<_>>(),
                    expected
                );
                assert_eq!((h.bus.ram[0x12fffe], h.bus.ram[0x130003]), (0xa5, 0xa5));
            }
        }
    }
}

#[test]
fn long_sign_tests_execute_after_independent_o65_placements() {
    let source = SIGN_SOURCE.replace("input=$7100", "input");
    for optimize in [false, true] {
        let bytes = o65::compile(&source, optimize, vec![]);
        for variant in 0..2 {
            let image = format::relocate(
                &bytes,
                &o65::placement(&bytes, variant, vec![o65::fault(variant)]),
            )
            .unwrap();
            let caller = caller(image.entry());
            for value in [0u32, 1, 0x80000000, 0xffffffff] {
                for mask in [0, 4] {
                    let mut h = Harness::new_o65(&image, &caller, mask);
                    let at = o65::object(&image, "input") as usize;
                    h.bus.ram[at..at + 4].copy_from_slice(&value.to_le_bytes());
                    h.bus.ram[0x7200..0x721a].fill(0xa5);
                    h.run();
                    h.guards(mask);
                    sign_results(&h, value);
                }
            }
        }
    }
}

#[test]
fn long_sign_materialization_matches_ca65_and_reads_only_the_private_top_byte() {
    for optimize in [false, true] {
        let p = prepare(SIGN_SOURCE, optimize);
        let c = p.compile(&layout()).unwrap();
        for name in ["Negative", "Nonnegative"] {
            let r = p.mir.routines.iter().find(|r| r.name == name).unwrap();
            let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
            let linked = c.image.routines.iter().find(|r| r.id == m.id.0).unwrap();
            let (block, i, dest, source) = r
                .blocks
                .iter()
                .find_map(|b| {
                    b.ops.iter().enumerate().find_map(|(i, op)| match op {
                        Mir65816Op::Compare {
                            dest,
                            left: Mir65816Value::Temp(source, _),
                            ..
                        } => Some((b.id, i, *dest, *source)),
                        _ => None,
                    })
                })
                .unwrap();
            let span = &m.code.mir_spans[&(block, i)];
            let compare = r
                .blocks
                .iter()
                .find(|b| b.id == block)
                .unwrap()
                .ops
                .get(i)
                .unwrap();
            if !matches!(
                compare,
                Mir65816Op::Compare {
                    right: Mir65816Value::U32(0),
                    ..
                }
            ) {
                // Raw lowering may retain a constant-widening temp. Its ordinary
                // comparison path stays covered by the execution tests above.
                assert!(!optimize);
                assert!(span.len() > 14);
                continue;
            }
            let start = linked.address + span.start as u32;
            let end = linked.address + span.end as u32;
            let input = m.frame.temps[&source].stack().unwrap().offset + 3;
            let dest = m.frame.temps[&dest].stack().unwrap().offset;
            let reference = assemble(
                &format!(
                    "sep #$20\n.a8\nlda {input},s\ncmp #$80\nlda #0\nadc #0\n{}sta {dest},s",
                    if name == "Negative" { "" } else { "eor #1\n" }
                ),
                start,
            );
            assert_eq!(&m.code.bytes[span.clone()], reference);
            for value in [0u32, 0x7fffffff, 0x80000000, 0xffffffff] {
                let mut h = Harness::new(&c.image, &caller(c.image.entry), 0);
                h.bus.ram[0x7100..0x7104].copy_from_slice(&value.to_le_bytes());
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
                let reads = h.bus.reads.len();
                let writes = h.bus.writes.len();
                let stack = u32::from(h.cpu.registers().s);
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
                assert_eq!(
                    h.bus.reads[reads..]
                        .iter()
                        .copied()
                        .filter(|a| (0x4000..0x6000).contains(a))
                        .collect::<Vec<_>>(),
                    vec![stack + u32::from(input)]
                );
                assert_eq!(
                    &h.bus.writes[writes..],
                    &[(
                        stack + u32::from(dest),
                        u8::from(if name == "Negative" {
                            (value as i32) < 0
                        } else {
                            (value as i32) >= 0
                        })
                    )]
                );
            }
        }
    }
}

fn order_source(ty: &str) -> String {
    format!(
        r#"
{ty} a=$7100,b=$7104
BYTE ARRAY out=$7200
BYTE FUNC Less({ty} x,y) RETURN(x<y)
BYTE FUNC AtMost({ty} x,y) RETURN(x<=y)
BYTE FUNC Greater({ty} x,y) RETURN(x>y)
BYTE FUNC AtLeast({ty} x,y) RETURN(x>=y)
PROC Work({ty} x,y)
 BYTE saved
 out(0)=(x<y) out(2)=(x<=y) out(4)=(x>y) out(6)=(x>=y)
 IF x<y THEN out(8)=1 ELSE out(8)=0 FI
 IF x<=y THEN out(10)=1 ELSE out(10)=0 FI
 IF x>y THEN out(12)=1 ELSE out(12)=0 FI
 IF x>=y THEN out(14)=1 ELSE out(14)=0 FI
 saved=(x<y) out(16)=Less(x,y) out(18)=AtMost(x,y)
 out(20)=Greater(x,y) out(22)=AtLeast(x,y) out(24)=saved
 out(26)=(x<{ty}($80000000)) out(28)=({ty}($7fffffff)<y)
 out(30)=(x<=0) out(32)=(x>0)
 x=0 out(34)=(x<y) out(36)=(x>=y)
RETURN
PROC Main() Work(a,b) RETURN
"#
    )
}
fn order_results(h: &Harness, a: u32, b: u32, signed: bool) {
    let ord = |x: u32, y: u32| {
        if signed {
            (x as i32).cmp(&(y as i32))
        } else {
            x.cmp(&y)
        }
    };
    let c = ord(a, b);
    for (i, truth) in [
        c.is_lt(),
        c.is_le(),
        c.is_gt(),
        c.is_ge(),
        c.is_lt(),
        c.is_le(),
        c.is_gt(),
        c.is_ge(),
        c.is_lt(),
        c.is_le(),
        c.is_gt(),
        c.is_ge(),
        c.is_lt(),
        ord(a, 0x80000000).is_lt(),
        ord(0x7fffffff, b).is_lt(),
        ord(a, 0).is_le(),
        ord(a, 0).is_gt(),
        ord(0, b).is_lt(),
        ord(0, b).is_ge(),
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
fn exercise_order(ty: &str) {
    let values = [
        0u32, 1, 0xffff, 0x10000, 0x10001, 0x1ffff, 0x20000, 0x7fffffff, 0x80000000, 0x8000ffff,
        0xff000000, 0xffffffff,
    ];
    let mut pairs: Vec<_> = values
        .into_iter()
        .flat_map(|a| values.map(|b| (a, b)))
        .collect();
    let mut seed = 0x8160cafeu32;
    for _ in 0..32 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let a = seed;
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        pairs.extend([(a, seed), (a, a), (a, a ^ 0x80000000)]);
    }
    for optimize in [false, true] {
        let source = order_source(ty);
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
                h.bus.ram[0x7200..0x7226].fill(0xa5);
                h.run();
                h.guards(mask);
                order_results(&h, a, b, ty == "LONGINT");
                if a == 0x80000000 && b == 0 && mask == 0 {
                    narrow_comparison::record(
                        &format!("long-order-{ty}"),
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
#[test]
fn unsigned_long_ordering_covers_all_relations_halves_and_consumers() {
    exercise_order("LONGCARD");
}

fn relocated_order(ty: &str) {
    let source = order_source(ty).replace("a=$7100,b=$7104", "a,b");
    for optimize in [false, true] {
        let bytes = o65::compile(&source, optimize, vec![]);
        for variant in 0..2 {
            let image = format::relocate(
                &bytes,
                &o65::placement(&bytes, variant, vec![o65::fault(variant)]),
            )
            .unwrap();
            let caller = caller(image.entry());
            for (a, b) in [
                (0u32, 0u32),
                (0xffff, 0x10000),
                (0x7fffffff, 0x80000000),
                (0xffffffff, 0),
                (0x80000000, 0xffffffff),
            ] {
                for mask in [0, 4] {
                    let mut h = Harness::new_o65(&image, &caller, mask);
                    for (name, value) in [("a", a), ("b", b)] {
                        let at = o65::object(&image, name) as usize;
                        h.bus.ram[at..at + 4].copy_from_slice(&value.to_le_bytes());
                    }
                    h.bus.ram[0x7200..0x7226].fill(0xa5);
                    h.run();
                    h.guards(mask);
                    order_results(&h, a, b, ty == "LONGINT");
                }
            }
        }
    }
}
#[test]
fn unsigned_long_ordering_executes_after_o65_relocation() {
    relocated_order("LONGCARD");
}

#[test]
fn unsigned_long_ordering_matches_ca65_and_short_circuits_only_private_reads() {
    for optimize in [false, true] {
        let p = prepare(&order_source("LONGCARD"), optimize);
        let c = p.compile(&layout()).unwrap();
        for name in ["Less", "AtMost", "Greater", "AtLeast"] {
            let r = p.mir.routines.iter().find(|r| r.name == name).unwrap();
            let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
            let linked = c.image.routines.iter().find(|r| r.id == m.id.0).unwrap();
            let (block, i, dest, a, b) = r
                .blocks
                .iter()
                .find_map(|block| {
                    block.ops.iter().enumerate().find_map(|(i, op)| match op {
                        Mir65816Op::Compare {
                            dest,
                            left: Mir65816Value::Temp(a, _),
                            right: Mir65816Value::Temp(b, _),
                            ..
                        } => Some((block.id, i, *dest, *a, *b)),
                        _ => None,
                    })
                })
                .unwrap();
            let span = &m.code.mir_spans[&(block, i)];
            let start = linked.address + span.start as u32;
            let end = linked.address + span.end as u32;
            let mut left = m.frame.temps[&a].stack().unwrap().offset;
            let mut right = m.frame.temps[&b].stack().unwrap().offset;
            let dest = m.frame.temps[&dest].stack().unwrap().offset;
            if matches!(name, "AtMost" | "Greater") {
                std::mem::swap(&mut left, &mut right);
            }
            let branch = if matches!(name, "Less" | "Greater") {
                "bcc"
            } else {
                "bcs"
            };
            let reference = assemble(
                &format!(
                    "lda {},s\ncmp {},s\nbne decide\nlda {left},s\ncmp {right},s\ndecide:\n{branch} yes\nsep #$20\n.a8\nlda #0\nbra done\nyes:\nsep #$20\nlda #1\ndone:\nsep #$20\nsta {dest},s",
                    left + 2,
                    right + 2
                ),
                start,
            );
            assert_eq!(&m.code.bytes[span.clone()], reference);
            for (a, b) in [(0u32, 0u32), (0x10000, 0xffff), (0x10001, 0x10000)] {
                let mut h = Harness::new(&c.image, &caller(c.image.entry), 0);
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
                let reads = h.bus.reads.len();
                let stack = u32::from(h.cpu.registers().s);
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
                    stack + u32::from(left) + 2,
                    stack + u32::from(left) + 3,
                    stack + u32::from(right) + 2,
                    stack + u32::from(right) + 3,
                ];
                if a >> 16 == b >> 16 {
                    expected.extend([
                        stack + u32::from(left),
                        stack + u32::from(left) + 1,
                        stack + u32::from(right),
                        stack + u32::from(right) + 1,
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
            }
        }
    }
}
