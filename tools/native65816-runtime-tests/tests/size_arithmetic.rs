mod support;
use actionc::mir65816::{
    Mir65816AbiHome, Mir65816AddressBase, Mir65816Op, Mir65816Value, o65 as format,
};
use actionc_vm::native65816::{Inputs, Machine};
use support::*;

const MASK: u32 = 0xffffff;

fn source(ty: &str) -> String {
    format!(
        r#"
{ty} a=$7100,b=$7104
{ty} ARRAY out=$7200
{ty} FUNC Add24({ty} x,y) RETURN(x+y)
{ty} FUNC Sub24({ty} x,y) RETURN(x-y)
SIZE FUNC Difference(ADDRESS x,y) RETURN(x-y)
PROC Work({ty} x,y)
  out(0)=x+y out(2)=x-y
  out(4)=x&y out(6)=x OR y out(8)=x XOR y
  out(10)=x+{ty}($810001) out(12)={ty}($810001)-x
  out(14)=(x+y)-(x-y)
  x==+y out(16)=x x==-y out(18)=x
  out(20)=Add24(x,y) out(22)=Sub24(x,y)
  out(24)=x&{ty}($810001) out(26)=x OR {ty}($810001)
  out(28)=x XOR {ty}($810001) out(30)=x+BYTE(255)
  out(32)=x+x out(34)=x+CARD(65535)
  out(36)=Difference(ADDRESS(x),ADDRESS(y))
RETURN
PROC Main() Work(a,b) RETURN
"#
    )
}

fn initialize(h: &mut Harness, a: u32, b: u32, a_at: u32, b_at: u32) {
    for (at, value) in [(a_at, a), (b_at, b)] {
        h.bus.ram[at as usize..at as usize + 3].copy_from_slice(&value.to_le_bytes()[..3]);
        h.bus.ram[at as usize + 3] = 0xa5;
    }
    h.bus.ram[0x7200..0x7272].fill(0xa5);
}

fn check(h: &Harness, a: u32, b: u32) {
    for (i, expected) in [
        a.wrapping_add(b),
        a.wrapping_sub(b),
        a & b,
        a | b,
        a ^ b,
        a.wrapping_add(0x810001),
        0x810001u32.wrapping_sub(a),
        b.wrapping_mul(2),
        a.wrapping_add(b),
        a,
        a.wrapping_add(b),
        a.wrapping_sub(b),
        a & 0x810001,
        a | 0x810001,
        a ^ 0x810001,
        a.wrapping_add(255),
        a.wrapping_mul(2),
        a.wrapping_add(65535),
        a.wrapping_sub(b),
    ]
    .into_iter()
    .enumerate()
    {
        let at = 0x7200 + i as u32 * 6;
        assert_eq!(
            h.bus.value(at, 3),
            expected & MASK,
            "{a:06x}/{b:06x}/output {i}"
        );
        assert_eq!(h.bus.value(at + 3, 3), 0xa5a5a5);
    }
}

#[test]
fn size_arithmetic_wraps_at_24_bits_and_preserves_exact_extents() {
    let mut pairs = vec![];
    for a in [
        0, 1, 0xff, 0x100, 0xffff, 0x10000, 0x7fffff, 0x800000, 0xff0000, MASK,
    ] {
        pairs.extend([(a, 1), (1, a), (a, a), (a, 0x810001)]);
    }
    let mut seed = 0x816add24u32;
    for _ in 0..16 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let a = seed & MASK;
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        pairs.push((a, seed & MASK));
    }
    for optimize in [false, true] {
        let source = source("SIZE");
        let image = compile(&source, optimize);
        assert_eq!(
            image.to_json().unwrap(),
            compile(&source.replace('\n', "\r\n"), optimize)
                .to_json()
                .unwrap()
        );
        for &(a, b) in &pairs {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller(image.entry), mask);
                initialize(&mut h, a, b, 0x7100, 0x7104);
                h.run();
                h.guards(mask);
                check(&h, a, b);
                for at in [0x7100, 0x7104] {
                    assert_eq!(h.bus.ram[at + 3], 0xa5);
                    assert!(!h.bus.reads.contains(&(at as u32 + 3)));
                    assert_eq!(
                        h.bus
                            .reads
                            .iter()
                            .filter(|&&p| (at as u32..at as u32 + 3).contains(&p))
                            .count(),
                        3
                    );
                }
            }
        }
    }
}

#[test]
fn size_add_sub_match_ca65_without_scratch_or_a_fourth_byte() {
    for optimize in [false, true] {
        // Array stores retain complete result homes for this materialized-path
        // oracle. Direct A/X returns are covered by home_demand.rs.
        let source = source("SIZE")
            .replace(
                "RETURN(x+y)",
                "SIZE ARRAY captured(1) captured(0)=x+y RETURN(captured(0))",
            )
            .replace(
                "RETURN(x-y)",
                "SIZE ARRAY captured(1) captured(0)=x-y RETURN(captured(0))",
            );
        let p = prepare(&source, optimize);
        let c = p.compile(&layout()).unwrap();
        for (name, carry, alu) in [("Add24", "clc", "adc"), ("Sub24", "sec", "sbc")] {
            let r = p.mir.routines.iter().find(|r| r.name == name).unwrap();
            let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
            let linked = c.image.routines.iter().find(|m| m.id == r.id.0).unwrap();
            let (block, index, dest, left, right) = r
                .blocks
                .iter()
                .find_map(|block| {
                    block.ops.iter().enumerate().find_map(|(i, op)| match op {
                        Mir65816Op::Binary {
                            dest,
                            left: Mir65816Value::Temp(a, _),
                            right: Mir65816Value::Temp(b, _),
                            ..
                        } => Some((block.id, i, *dest, *a, *b)),
                        _ => None,
                    })
                })
                .unwrap();
            let mut span = m.code.mir_spans[&(block, index)].clone();
            // Array setup may leave A8. Compare the arithmetic after its
            // explicit A16 entry so both implementations start in that mode.
            if m.code.bytes[span.clone()].starts_with(&[0xc2, 0x20]) {
                span.start += 2;
            }
            let start = linked.address + span.start as u32;
            let end = linked.address + span.end as u32;
            // An omitted capture borrows the immutable parameter's original
            // stack bytes. Derive that address from typed MIR and ABI metadata,
            // independently of the selected arithmetic instruction operands.
            let source_home = |id| {
                for block in &r.blocks {
                    for (i, op) in block.ops.iter().enumerate() {
                        if let Mir65816Op::Load {
                            dest,
                            address,
                            width,
                            volatile: false,
                        } = op
                            && *dest == id
                            && m.code.mir_spans[&(block.id, i)].is_empty()
                        {
                            let Mir65816AddressBase::Parameter(param) = address.base else {
                                panic!()
                            };
                            assert_eq!(width.get(), 3);
                            assert!(address.index.is_none());
                            assert_eq!(address.displacement.get(), 0);
                            let p = r
                                .frame
                                .parameters
                                .iter()
                                .find(|p| p.param == param)
                                .unwrap();
                            assert!(p.frame_object.is_none());
                            let Mir65816AbiHome::StackArgument { offset, .. } = p.incoming else {
                                panic!()
                            };
                            return m.frame.extent + 4 + u16::try_from(offset.get()).unwrap();
                        }
                    }
                }
                m.frame.temps[&id].stack().unwrap().offset
            };
            let a = source_home(left);
            let b = source_home(right);
            let dest = m.frame.temps[&dest].stack().unwrap().offset;
            let reference = assemble(
                &format!(
                    "lda {a},s\n{carry}\n{alu} {b},s\nsta {dest},s\nsep #$20\n.a8\nlda {},s\n{alu} {},s\nsta {},s",
                    a + 2,
                    b + 2,
                    dest + 2
                ),
                start,
            );
            assert_eq!(reference.len(), 15);
            assert_eq!(m.code.bytes[span.clone()], reference);
            let mut old_source = format!("sep #$20\n.a8\n{carry}\n");
            for i in 0..3 {
                old_source += &format!(
                    "lda {},s\nsta $90\nlda {},s\n{alu} $90\nsta {},s\n",
                    b + i,
                    a + i,
                    dest + i
                );
            }
            let old = assemble(&old_source, 0x600000);
            assert_eq!(old.len(), 33);
            for (x, y) in [
                (0u32, 1),
                (0xffff, 1),
                (0x10000, 1),
                (MASK, 1),
                (0x800000, MASK),
            ] {
                let mut h = Harness::new(&c.image, &caller(c.image.entry), 0);
                initialize(&mut h, x, y, 0x7100, 0x7104);
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
                let mut old_bus = h.bus.clone();
                old_bus.map(0x600000, &old, false);
                let mut old_regs = before;
                old_regs.pbr = 0x60;
                old_regs.pc = 0;
                let mut old_cpu = Machine::start_at(old_regs);
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
                let reads = h.bus.reads.len();
                let writes = h.bus.writes.len();
                let cycles = h.cpu.cycles();
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
                let expected = if name == "Add24" {
                    x.wrapping_add(y)
                } else {
                    x.wrapping_sub(y)
                } & MASK;
                let s = u32::from(before.s);
                assert_eq!(h.bus.value(s + u32::from(dest), 3), expected);
                assert_eq!(old_bus.value(s + u32::from(dest), 3), expected);
                let expected_reads: Vec<_> = [a, a + 1, b, b + 1, a + 2, b + 2]
                    .into_iter()
                    .map(|p| s + u32::from(p))
                    .collect();
                assert_eq!(
                    h.bus.reads[reads..]
                        .iter()
                        .copied()
                        .filter(|p| (0x4000..0x6000).contains(p))
                        .collect::<Vec<_>>(),
                    expected_reads
                );
                assert_eq!(
                    h.bus.writes[writes..],
                    expected.to_le_bytes()[..3]
                        .iter()
                        .enumerate()
                        .map(|(i, &v)| (s + u32::from(dest) + i as u32, v))
                        .collect::<Vec<_>>()
                );
                assert!(
                    !h.bus.reads[reads..]
                        .iter()
                        .any(|p| (0x2080..0x20c0).contains(p))
                );
                let after = h.cpu.registers();
                assert_eq!(
                    (
                        after.s,
                        after.d,
                        after.dbr,
                        after.x,
                        after.y,
                        after.p & 0x1c
                    ),
                    (
                        before.s,
                        before.d,
                        before.dbr,
                        before.x,
                        before.y,
                        before.p & 0x1c
                    )
                );
                assert!(h.cpu.cycles() - cycles < old_cpu.cycles());
                if x == 0xffff {
                    eprintln!(
                        "{name} optimized={optimize}: 33->15 bytes, {}->{} cycles",
                        old_cpu.cycles(),
                        h.cpu.cycles() - cycles
                    );
                }
                h.run();
                h.guards(0);
                check(&h, x, y);
            }
        }
    }
}

#[test]
fn size_arithmetic_executes_after_o65_relocation() {
    let source = source("SIZE").replace("a=$7100,b=$7104", "a,b");
    for optimize in [false, true] {
        let bytes = o65::compile(&source, optimize, vec![]);
        for variant in 0..2 {
            let placement = o65::placement(&bytes, variant, vec![o65::fault(variant)]);
            let image = format::relocate(&bytes, &placement).unwrap();
            let a_at = o65::object(&image, "a");
            let b_at = o65::object(&image, "b");
            for (a, b) in [(0, 1), (0xffff, 1), (0x10000, 1), (MASK, 1)] {
                for mask in [0, 4] {
                    let mut h = Harness::new_o65(&image, &caller(image.entry()), mask);
                    initialize(&mut h, a, b, a_at, b_at);
                    h.run();
                    h.guards(mask);
                    check(&h, a, b);
                }
            }
        }
    }
}
