mod support;
use actionc::mir65816::{Mir65816Op, o65 as format};
use actionc_vm::native65816::{Access, Inputs, Machine};
use support::*;

#[test]
fn constants_keep_bits_across_all_storage_forms_and_scalar_widths() {
    for (ty, bytes, values) in [
        ("BYTE", 1, vec![0, 255]),
        ("CARD", 2, vec![0, 0x8000, 0xffff]),
        ("INT", 2, vec![0, 0x8000, 0xffff]),
        ("SIZE", 3, vec![0, 0x123412, 0xabcdef, 0xffffff]),
        ("ADDRESS", 3, vec![0, 0x123412, 0xabcdef, 0xffffff]),
        (
            "LONGCARD",
            4,
            vec![0, 255, 0xffff, 0x80000000, 0x12341234, 0xffffffff],
        ),
        (
            "LONGINT",
            4,
            vec![0, 255, 0xffff, 0x80000000, 0x12341234, 0xffffffff],
        ),
    ] {
        for value in values {
            let source = format!(
                r#"
{ty} direct=$7100,global
{ty} ARRAY table(4)
{ty} POINTER destination=$7110
CARD index=$7114
{ty} frameResult=$7120,paramResult=$7130
PROC Store({ty} parameter)
 {ty} local
 local={ty}(${value:X}) parameter={ty}(${value:X})
 direct={ty}(${value:X}) global={ty}(${value:X})
 table(index)={ty}(${value:X}) destination^={ty}(${value:X})
 frameResult=local paramResult=parameter
RETURN
PROC Main() Store(0) RETURN
"#
            );
            for optimize in [false, true] {
                let image = compile(&source, optimize);
                // Exercise checked-out text with either newline convention.
                assert_eq!(
                    image.to_json().unwrap(),
                    compile(&source.replace('\n', "\r\n"), optimize)
                        .to_json()
                        .unwrap()
                );
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller(image.entry), mask);
                    h.bus.map(0x22fffe, &[0xa5; 6], true);
                    h.bus.ram[0x7110..0x7113].copy_from_slice(&[0xff, 0xff, 0x22]);
                    h.bus.ram[0x7114] = 2;
                    let table = context::symbol(&image, "table");
                    let global = context::symbol(&image, "global");
                    h.bus.ram[table as usize..table as usize + 4 * bytes].fill(0xa5);
                    for at in [0x7100, 0x7120, 0x7130] {
                        h.bus.ram[at - 1..at + bytes + 1].fill(0xa5);
                    }
                    for at in [0x7100, 0x22ffff, global, table + 2 * bytes as u32] {
                        h.bus.watched.extend(at..at + bytes as u32);
                    }
                    h.run();
                    h.guards(mask);
                    for at in [
                        0x7100,
                        0x7120,
                        0x7130,
                        global,
                        table + 2 * bytes as u32,
                        0x22ffff,
                    ] {
                        assert_eq!(h.bus.value(at, bytes), value, "{ty}/{value:x}/{at:x}");
                    }
                    for at in [0x7100, 0x7120, 0x7130, 0x22ffff] {
                        assert_eq!(h.bus.ram[at - 1], 0xa5);
                        assert_eq!(h.bus.ram[at + bytes], 0xa5);
                    }
                    assert!(
                        h.bus.ram[table as usize..table as usize + 2 * bytes]
                            .iter()
                            .all(|b| *b == 0xa5)
                    );
                    assert!(
                        h.bus.ram[table as usize + 3 * bytes..table as usize + 4 * bytes]
                            .iter()
                            .all(|b| *b == 0xa5)
                    );
                    let expected: Vec<_> = [0x7100, global, table + 2 * bytes as u32, 0x22ffff]
                        .into_iter()
                        .flat_map(|at| {
                            value.to_le_bytes()[..bytes]
                                .iter()
                                .copied()
                                .enumerate()
                                .map(move |(i, v)| (at + i as u32, Access::Write(v)))
                                .collect::<Vec<_>>()
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
    }
}

#[test]
fn null_pointer_fields_have_no_overlap_or_fourth_byte_access() {
    for offset in [0, 3, 65532, 65533, 65535, 65536] {
        let mut padding = String::new();
        let mut remaining = offset;
        let mut n = 0;
        while remaining != 0 {
            let size = remaining.min(16384);
            padding += &format!("BYTE ARRAY pad{n}({size}) ");
            remaining -= size;
            n += 1;
        }
        let source = format!(
            r#"
TYPE Node=[{padding}Node POINTER next Node POINTER previous]
Node POINTER root=$7110
PROC Clear(Node POINTER p)
 p.next=Node POINTER(0) p.previous=Node POINTER(0)
RETURN
PROC Main() Clear(root) RETURN
"#
        );
        for optimize in [false, true] {
            let image = compile(&source, optimize);
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller(image.entry), mask);
                let root = 0x32ffffu32 - offset;
                h.bus.ram[0x7110..0x7113].copy_from_slice(&root.to_le_bytes()[..3]);
                // Map only the declared fields. Padding and a seventh byte are inaccessible.
                h.bus.map(0x32ffff, &[0xa5; 6], true);
                h.bus.watched.extend(0x32ffff..0x330005);
                h.run();
                h.guards(mask);
                assert_eq!(
                    h.bus
                        .trace
                        .iter()
                        .map(|&(_, a, k)| (a, k))
                        .collect::<Vec<_>>(),
                    (0x32ffff..0x330005)
                        .map(|a| (a, Access::Write(0)))
                        .collect::<Vec<_>>()
                );
            }
        }
    }
}

#[test]
fn native_zero_stores_match_ca65_and_reduce_bytes_and_cycles() {
    let source = "CARD word=$BFFF LONGCARD wide=$CFFF BYTE POINTER link=$EFFF PROC WordZero() word=0 RETURN PROC LongZero() wide=0 RETURN PROC PointerZero() link=BYTE POINTER(0) RETURN PROC Main() WordZero() LongZero() PointerZero() RETURN";
    // Constant propagation supplies the literal MIR operands in this source.
    // The other tests also exercise the retained raw capture/store paths.
    for optimize in [true] {
        let p = prepare(source, optimize);
        let c = p.compile(&layout()).unwrap();
        for (name, at, bytes, native_bytes) in [
            ("WordZero", 0xbfff, 2, 7),
            ("LongZero", 0xcfff, 4, 11),
            ("PointerZero", 0xefff, 3, 13),
        ] {
            let r = c
                .machine
                .prepared
                .routines
                .iter()
                .find(|r| r.name == name)
                .unwrap();
            let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
            let linked = c.image.routines.iter().find(|m| m.id == r.id.0).unwrap();
            let (block, index) = r
                .blocks
                .iter()
                .find_map(|b| {
                    b.ops.iter().enumerate().find_map(|(i, op)| {
                        matches!(op, Mir65816Op::Store { .. }).then_some((b.id, i))
                    })
                })
                .unwrap();
            let span = &m.code.mir_spans[&(block, index)];
            let start = linked.address + span.start as u32;
            let end = linked.address + span.end as u32;
            let mut native = format!("lda #0\nsta f:${at:06x}\n");
            if bytes == 3 {
                native += "sep #$20\n.a8\n";
            }
            if bytes > 2 {
                native += &format!("sta f:${:06x}\n", at + 2);
            }
            let native = assemble(&native, start);
            assert_eq!(native.len(), native_bytes);
            let mut old = String::from("sep #$20\n.a8\n");
            for i in 0..bytes {
                old += &format!("lda #0\nsta f:${:06x}\n", at + i);
            }
            let old = assemble(&old, 0x600000);
            for mask in [0, 4] {
                let mut h = Harness::new(&c.image, &caller(c.image.entry), mask);
                for (at, bytes) in [(0xbfff, 2), (0xcfff, 4), (0xefff, 3)] {
                    h.bus.map(at, &vec![0xa5; bytes], true);
                }
                assert!(
                    h.cpu
                        .run_until(
                            &mut h.bus,
                            100000,
                            |_| Inputs::default(),
                            |cpu| cpu.is_instruction_boundary() && cpu.pc() == start
                        )
                        .unwrap()
                );
                assert_eq!(&h.bus.ram[start as usize..end as usize], native);
                let before = h.cpu.registers();
                assert_eq!(before.p & 0x20, 0);
                let mut old_bus = h.bus.clone();
                old_bus.map(0x600000, &old, false);
                let mut registers = before;
                registers.pbr = 0x60;
                registers.pc = 0;
                let mut old_cpu = Machine::start_at(registers);
                assert!(
                    old_cpu
                        .run_until(
                            &mut old_bus,
                            1000,
                            |_| Inputs::default(),
                            |cpu| cpu.is_instruction_boundary()
                                && cpu.pc() == 0x600000 + old.len() as u32
                        )
                        .unwrap()
                );
                let cycles = h.cpu.cycles();
                let writes = h.bus.writes.len();
                let reads = h.bus.reads.len();
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
                assert_eq!(
                    h.bus.writes[writes..],
                    (at..at + bytes).map(|a| (a, 0)).collect::<Vec<_>>()
                );
                assert!(
                    !h.bus.reads[reads..]
                        .iter()
                        .any(|a| (at..at + bytes).contains(a))
                );
                assert_eq!(
                    h.bus.value(at, bytes as usize),
                    old_bus.value(at, bytes as usize)
                );
                let after = h.cpu.registers();
                assert_eq!(
                    (
                        before.s,
                        before.d,
                        before.dbr,
                        before.x,
                        before.y,
                        before.p & 0x0c
                    ),
                    (
                        after.s,
                        after.d,
                        after.dbr,
                        after.x,
                        after.y,
                        after.p & 0x0c
                    )
                );
                assert!(h.cpu.cycles() - cycles < old_cpu.cycles());
                if mask == 0 {
                    eprintln!(
                        "{name} optimized={optimize}: {}->{} bytes, {}->{} cycles",
                        old.len(),
                        native.len(),
                        old_cpu.cycles(),
                        h.cpu.cycles() - cycles
                    );
                }
                h.run();
                h.guards(mask);
            }
        }
    }
}

#[test]
fn volatile_constants_retain_byte_instructions_and_ascending_writes() {
    let source = "VOLATILE CARD word=$D000 VOLATILE ADDRESS banked=$D004 VOLATILE LONGCARD wide=$D008 VOLATILE ADDRESS link=$D00C PROC Main() word=$BEEF banked=$AB1234 wide=LONGCARD($12341234) link=0 RETURN";
    for optimize in [false, true] {
        let c = prepare(source, optimize).compile(&layout()).unwrap();
        for m in &c.machine.routines {
            let r = c
                .machine
                .prepared
                .routines
                .iter()
                .find(|r| r.id == m.id)
                .unwrap();
            for b in &r.blocks {
                for (i, op) in b.ops.iter().enumerate() {
                    if let Mir65816Op::Store {
                        width,
                        volatile: true,
                        ..
                    } = op
                    {
                        let span = &m.code.mir_spans[&(b.id, i)];
                        let bytes = &m.code.bytes[span.clone()];
                        // Optional SEP, followed by one LDA8/STA-long per byte.
                        let body = if bytes.starts_with(&[0xe2, 0x20]) {
                            &bytes[2..]
                        } else {
                            bytes
                        };
                        assert_eq!(body.len(), width.get() as usize * 6);
                        assert!(
                            body.chunks_exact(6)
                                .all(|c| matches!(c[0], 0xa9 | 0xa3 | 0xa5) && c[2] == 0x8f)
                        );
                    }
                }
            }
        }
        for mask in [0, 4] {
            let mut h = Harness::new(&c.image, &caller(c.image.entry), mask);
            h.bus.map(0xcfff, &[0xa5; 17], true);
            h.bus.watched.extend(0xcfff..0xd010);
            h.run();
            h.guards(mask);
            let expected: Vec<_> = [
                (0xd000, 0xbeefu32, 2),
                (0xd004, 0xab1234, 3),
                (0xd008, 0x12341234, 4),
                (0xd00c, 0, 3),
            ]
            .into_iter()
            .flat_map(|(a, v, n)| {
                v.to_le_bytes()[..n]
                    .iter()
                    .copied()
                    .enumerate()
                    .map(move |(i, v)| (a + i as u32, Access::Write(v)))
                    .collect::<Vec<_>>()
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
fn constant_stores_and_symbolic_fallback_execute_after_o65_relocation() {
    let source = "CARD word ADDRESS banked LONGCARD wide CARD POINTER ptr,cleared PROC POINTER cb PROC ApplyValue() word=$BEEF RETURN PROC Main() word=1 banked=$AB1234 wide=LONGCARD($12341234) ptr=@word cleared=CARD POINTER(0) cb=@ApplyValue cb() RETURN";
    for optimize in [false, true] {
        let bytes = o65::compile(source, optimize, vec![]);
        for variant in 0..2 {
            let image = format::relocate(
                &bytes,
                &o65::placement(&bytes, variant, vec![o65::fault(variant)]),
            )
            .unwrap();
            for mask in [0, 4] {
                let mut h = Harness::new_o65(&image, &caller(image.entry()), mask);
                h.run();
                h.guards(mask);
                for (name, size, value) in [
                    ("word", 2, 0xbeef),
                    ("banked", 3, 0xab1234),
                    ("wide", 4, 0x12341234),
                    ("ptr", 3, o65::object(&image, "word")),
                    ("cleared", 3, 0),
                ] {
                    assert_eq!(h.bus.value(o65::object(&image, name), size), value);
                }
            }
        }
    }
}
