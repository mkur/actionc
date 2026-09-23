mod support;
use support::*;

#[test]
fn pointer_captures_and_boolean_results_survive_alias_and_call_clobbers() {
    narrow_comparison::captured_values_survive_clobbers(true);
}

const SOURCE: &str = "BYTE POINTER a=$7100,b=$7103\nBYTE ARRAY out=$7200\n\
    BYTE FUNC Equal(BYTE POINTER x,y) RETURN(x=y)\n\
    PROC Save(BYTE value) out(16)=value RETURN\n\
    PROC Work(BYTE POINTER x,y) BYTE saved\n\
    out(0)=(x=y) out(2)=(x#y)\n\
    IF x=y THEN out(4)=1 ELSE out(4)=0 FI\n\
    IF x#y THEN out(6)=1 ELSE out(6)=0 FI\n\
    out(8)=(x=BYTE POINTER(0)) out(10)=(BYTE POINTER(0)#x)\n\
    saved=(x#y) out(12)=Equal(x,y) out(14)=saved Save(saved)\n\
    IF x=BYTE POINTER(0) THEN out(18)=1 ELSE out(18)=0 FI\n\
    IF BYTE POINTER(0)#x THEN out(20)=1 ELSE out(20)=0 FI\n\
    out(22)=(x=BYTE POINTER($010001)) x=BYTE POINTER(0) out(24)=(x=y)\n\
    RETURN\nPROC Main() Work(a,b) RETURN\n";

#[test]
fn pointer_equality_and_null_tests_cover_each_byte_and_boolean_consumer() {
    let values = [
        0u32, 1, 0xffff, 0x10000, 0x10001, 0x12ffff, 0x130000, 0xffffff,
    ];
    for optimize in [false, true] {
        narrow_comparison::check_shape(SOURCE, optimize, 3);
        let image = compile(SOURCE, optimize);
        assert!(
            image
                .routines
                .iter()
                .find(|r| r.name == "Equal")
                .unwrap()
                .size
                <= 170
        );
        assert!(
            image.routines.iter().map(|r| r.size).sum::<u32>() < if optimize { 2901 } else { 2909 }
        );
        assert_eq!(
            image.to_json().unwrap(),
            compile(&SOURCE.replace('\n', "\r\n"), optimize)
                .to_json()
                .unwrap()
        );
        let caller = caller(image.entry);
        for a in values {
            for b in values {
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller, mask);
                    h.bus.ram[0x7100..0x7103].copy_from_slice(&a.to_le_bytes()[..3]);
                    h.bus.ram[0x7103..0x7106].copy_from_slice(&b.to_le_bytes()[..3]);
                    h.bus.ram[0x7200..0x721a].fill(0xa5);
                    h.run();
                    h.guards(mask);
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
                        a == 0x10001,
                        0 == b,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        assert_eq!(
                            h.bus.ram[0x7200 + 2 * i],
                            u8::from(truth),
                            "{optimize}/{a:x}/{b:x}/{mask}/{i}"
                        );
                        assert_eq!(h.bus.ram[0x7201 + 2 * i], 0xa5);
                    }
                    if a == 0x10000 && b == 0 && mask == 0 {
                        narrow_comparison::record(
                            "pointer-comparisons",
                            optimize,
                            SOURCE,
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
fn emitted_pointer_equality_reads_only_the_low_word_and_needed_bank_bytes() {
    use actionc::mir65816::{Mir65816Op, Mir65816Value};
    use actionc_vm::native65816::Inputs;
    for optimize in [false, true] {
        let p = prepare(SOURCE, optimize);
        let compiled = p.compile(&layout()).unwrap();
        let r = p.mir.routines.iter().find(|r| r.name == "Equal").unwrap();
        let machine = compiled
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
        let span = &machine.code.mir_spans[&(block, index)];
        let start = linked.address + span.start as u32;
        let end = linked.address + span.end as u32;
        let left = machine.frame.temps[&left].stack().unwrap().offset;
        let right = machine.frame.temps[&right].stack().unwrap().offset;
        let dest = machine.frame.temps[&dest].stack().unwrap().offset;
        let caller = caller(compiled.image.entry);
        for (a, b) in [
            (0x123456u32, 0xabcdefu32),
            (0x12ffff, 0x13ffff),
            (0x12ffff, 0x12ffff),
            (0, 0x10000),
        ] {
            for mask in [0, 4] {
                let mut h = Harness::new(&compiled.image, &caller, mask);
                h.bus.ram[0x7100..0x7103].copy_from_slice(&a.to_le_bytes()[..3]);
                h.bus.ram[0x7103..0x7106].copy_from_slice(&b.to_le_bytes()[..3]);
                for _ in 0..100_000 {
                    if h.cpu.is_instruction_boundary() && h.cpu.pc() == start {
                        break;
                    }
                    h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                }
                assert_eq!(h.cpu.pc(), start);
                let s = u32::from(h.cpu.registers().s);
                let reads = h.bus.reads.len();
                let writes = h.bus.writes.len();
                for _ in 0..1_000 {
                    h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                    if h.cpu.is_instruction_boundary() && h.cpu.pc() == end {
                        break;
                    }
                }
                assert_eq!(h.cpu.pc(), end);
                let mut expected = vec![
                    s + u32::from(left),
                    s + u32::from(left) + 1,
                    s + u32::from(right),
                    s + u32::from(right) + 1,
                ];
                if a & 0xffff == b & 0xffff {
                    expected.extend([s + u32::from(left) + 2, s + u32::from(right) + 2]);
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
                    &[(s + u32::from(dest), u8::from(a == b))]
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

#[test]
fn relocated_byte_and_pointer_comparisons_use_full_data_and_code_addresses() {
    use actionc::mir65816::o65 as format;
    let source = "BYTE input\nBYTE POINTER a,b\nBYTE ARRAY output(12)\n\
        PROC Target() RETURN\nPROC Main() BYTE POINTER captured\nPROC POINTER callback\n\
        captured=@input callback=@Target\n\
        output(0)=(a=b) output(1)=(a#b) output(2)=(a=BYTE POINTER(0))\n\
        output(3)=(BYTE POINTER(0)#b)\n\
        IF a=b THEN output(4)=1 ELSE output(4)=0 FI\n\
        IF a#b THEN output(5)=1 ELSE output(5)=0 FI\n\
        output(6)=(captured=BYTE POINTER(@input)) output(7)=(callback=@Target)\n\
        output(8)=(input<BYTE(128)) IF input>=BYTE(128) THEN output(9)=1 ELSE output(9)=0 FI\n\
        output(10)=(captured=a) output(11)=(a=BYTE POINTER($010000))\nRETURN\n";
    for optimize in [false, true] {
        let bytes = o65::compile(source, optimize, vec![]);
        for variant in 0..2 {
            let placement = o65::placement(&bytes, variant, vec![o65::fault(variant)]);
            let image = format::relocate(&bytes, &placement).unwrap();
            let input = o65::object(&image, "input");
            let output = o65::object(&image, "output") as usize;
            let caller = caller(image.entry());
            for (a, b, n) in [
                (0, 0, 0),
                (0x10000, 0, 128),
                (0x12ffff, 0x13ffff, 255),
                (input, input, 127),
                (input, 0, 129),
            ] {
                for mask in [0, 4] {
                    let mut h = Harness::new_o65(&image, &caller, mask);
                    h.bus.ram[input as usize] = n;
                    for (name, value) in [("a", a), ("b", b)] {
                        let at = o65::object(&image, name) as usize;
                        h.bus.ram[at..at + 3].copy_from_slice(&value.to_le_bytes()[..3]);
                    }
                    h.run();
                    h.guards(mask);
                    assert_eq!(
                        &h.bus.ram[output..output + 12],
                        &[
                            a == b,
                            a != b,
                            a == 0,
                            b != 0,
                            a == b,
                            a != b,
                            true,
                            true,
                            n < 128,
                            n >= 128,
                            input == a,
                            a == 0x10000
                        ]
                        .map(u8::from)
                    );
                    o65::record(
                        "byte-pointer-comparisons",
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

#[test]
fn byte_and_pointer_conditions_preserve_same_target_parallel_edges_and_backedges() {
    use actionc::mir65816::{self, Mir65816Op, Mir65816Value};
    use actionc::nir::{NirCastKind, NirCompareOp, NirTypeKind, TempId};
    use actionc::target::ByteSize;
    let typed = prepare(
        "BYTE FUNC F(BYTE POINTER a,b) RETURN(a=b) PROC Main() RETURN",
        false,
    );
    let pointer_type = typed.mir.routines[0]
        .temps
        .iter()
        .find(|(_, t)| t.width == Some(ByteSize::new(3)))
        .unwrap()
        .1
        .clone();
    for optimize in [false, true] {
        for backedge in [false, true] {
            for width in [1, 3] {
                let mut p = edges::program(optimize, backedge);
                let r = p
                    .mir
                    .routines
                    .iter_mut()
                    .find(|r| r.name == "Work")
                    .unwrap();
                let mut ty = pointer_type.clone();
                if width == 1 {
                    ty.width = Some(ByteSize::ONE);
                    ty.kind = NirTypeKind::U8;
                    ty.pointer = false;
                    ty.summary = "BYTE".into();
                }
                r.temps.push((TempId(100), ty));
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
                        to: ByteSize::new(width),
                        kind: if width == 3 {
                            NirCastKind::IntegerToPointer
                        } else {
                            NirCastKind::Integer
                        },
                        value: input,
                    });
                    let Mir65816Op::Compare {
                        left,
                        right,
                        width: w,
                        operation,
                        ..
                    } = &mut compare
                    else {
                        unreachable!()
                    };
                    *left = Mir65816Value::Temp(TempId(100), ByteSize::new(width));
                    *right = if width == 3 {
                        Mir65816Value::Null(ByteSize::new(3))
                    } else {
                        Mir65816Value::U8(0)
                    };
                    *w = ByteSize::new(width);
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
                for (a, b) in [(0u16, 41u16), (0x100, 0x1234), (0xffff, 1), (0x80, 0xff)] {
                    for mask in [0, 4] {
                        let mut h = Harness::new(&image, &caller, mask);
                        h.bus.ram[0x7100..0x7102].copy_from_slice(&a.to_le_bytes());
                        h.bus.ram[0x7102..0x7104].copy_from_slice(&b.to_le_bytes());
                        h.run();
                        h.guards(mask);
                        let truth = if width == 1 { a as u8 == 0 } else { a == 0 };
                        assert_eq!(
                            h.bus.value(0x7200, 2),
                            u32::from(if backedge || truth {
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
}
