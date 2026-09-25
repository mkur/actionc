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
