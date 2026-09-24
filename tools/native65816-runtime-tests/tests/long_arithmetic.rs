mod support;
use actionc::mir65816::{Mir65816Op, Mir65816Value, abi, image::AssemblyImport, o65 as format};
use actionc_vm::native65816::{Access, Inputs, Machine};
use support::*;

fn source(ty: &str) -> String {
    format!(
        r#"
{ty} a=$7100,b=$7104
BYTE small=$7108
INT narrow=$710A
{ty} ARRAY out=$7200
{ty} FUNC AddLong({ty} x,y) RETURN(x+y)
{ty} FUNC SubLong({ty} x,y) RETURN(x-y)
PROC Work({ty} x,y)
  BYTE i,marker
  marker=small+1
  out(0)=x+y out(2)=x-y
  out(4)=x+1 out(6)=x-1 out(8)={ty}($12345678)-x
  out(10)=x+x out(12)=(x+y)-(x-y) out(14)=x+BYTE(255)
  IF x=0 THEN out(16)=y+1 ELSE out(16)=y-1 FI
  x==+y out(18)=x x==-y out(20)=x
  out(22)={ty}($80010001)+x
  out(24)=AddLong(x,y) out(26)=SubLong(x,y)
  FOR i=0 TO 2 DO x==+y OD
  out(28)=x
  out(30)=x+small out(32)=x+{ty}(LONGINT(narrow))
  small=marker-1
RETURN
PROC Main() Work(a,b) RETURN
"#
    )
}

fn check(h: &Harness, a: u32, b: u32) {
    for (i, value) in [
        a.wrapping_add(b),
        a.wrapping_sub(b),
        a.wrapping_add(1),
        a.wrapping_sub(1),
        0x12345678u32.wrapping_sub(a),
        a.wrapping_add(a),
        b.wrapping_add(b),
        a.wrapping_add(255),
        if a == 0 {
            b.wrapping_add(1)
        } else {
            b.wrapping_sub(1)
        },
        a.wrapping_add(b),
        a,
        0x80010001u32.wrapping_add(a),
        a.wrapping_add(b),
        a.wrapping_sub(b),
        a.wrapping_add(b.wrapping_mul(3)),
        a.wrapping_add(b.wrapping_mul(3)).wrapping_add(255),
        a.wrapping_add(b.wrapping_mul(3)).wrapping_sub(1),
    ]
    .into_iter()
    .enumerate()
    {
        let at = 0x7200 + i as u32 * 8;
        assert_eq!(h.bus.value(at, 4), value, "{a:08x}/{b:08x}/output {i}");
        assert_eq!(h.bus.value(at + 4, 4), 0xa5a5a5a5);
    }
    assert_eq!(h.bus.ram[0x7108], 255);
}

fn initialize(h: &mut Harness, a: u32, b: u32) {
    h.bus.ram[0x7100..0x7104].copy_from_slice(&a.to_le_bytes());
    h.bus.ram[0x7104..0x7108].copy_from_slice(&b.to_le_bytes());
    h.bus.ram[0x7108] = 255;
    h.bus.ram[0x710a..0x710c].copy_from_slice(&0xffffu16.to_le_bytes());
    h.bus.ram[0x7200..0x7288].fill(0xa5);
}

#[test]
fn long_arithmetic_boundaries_carries_borrows_casts_and_mutable_parameters() {
    let values = [
        0u32, 1, 0xff, 0x100, 0xffff, 0x10000, 0x10001, 0x7fffffff, 0x80000000, 0x80010001,
        0xffff0000, 0xffffffff,
    ];
    let mut pairs: Vec<_> = values
        .into_iter()
        .flat_map(|a| values.map(|b| (a, b)))
        .collect();
    let mut seed = 0x816add32u32;
    for _ in 0..32 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let a = seed;
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        pairs.push((a, seed));
    }
    for ty in ["LONGCARD", "LONGINT"] {
        let source = source(ty);
        for optimize in [false, true] {
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
                    initialize(&mut h, a, b);
                    h.run();
                    h.guards(mask);
                    check(&h, a, b);
                }
            }
        }
    }
}

#[test]
fn long_arithmetic_selected_words_match_ca65_and_use_no_scratch() {
    for optimize in [false, true] {
        let p = prepare(&source("LONGCARD"), optimize);
        let c = p.compile(&layout()).unwrap();
        let caller = caller(c.image.entry);
        for (name, carry, alu) in [("AddLong", "clc", "adc"), ("SubLong", "sec", "sbc")] {
            let r = p.mir.routines.iter().find(|r| r.name == name).unwrap();
            let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
            let linked = c.image.routines.iter().find(|m| m.id == r.id.0).unwrap();
            let (block, index, dest, left, right) = r
                .blocks
                .iter()
                .find_map(|b| {
                    b.ops.iter().enumerate().find_map(|(i, op)| match op {
                        Mir65816Op::Binary {
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
            let a = m.frame.temps[&left].stack().unwrap().offset;
            let b = m.frame.temps[&right].stack().unwrap().offset;
            let dest = m.frame.temps[&dest].stack().unwrap().offset;
            let reference = assemble(
                &format!(
                    "lda {a},s\n{carry}\n{alu} {b},s\nsta {dest},s\nlda {},s\n{alu} {},s\nsta {},s",
                    a + 2,
                    b + 2,
                    dest + 2
                ),
                start,
            );
            assert_eq!(reference.len(), 13);
            let mut old_source = format!("sep #$20\n.a8\n{carry}\n");
            for i in 0..4 {
                old_source += &format!(
                    "lda {},s\nsta $10\nlda {},s\n{alu} $10\nsta {},s\n",
                    b + i,
                    a + i,
                    dest + i
                );
            }
            let old = assemble(&old_source, 0x600000);
            assert_eq!(old.len(), 43);
            for (x, y) in [
                (0u32, 1u32),
                (0xffff, 1),
                (0x7fffffff, 1),
                (0x80000000, 0xffffffff),
                (0xffffffff, 1),
            ] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&c.image, &caller, mask);
                    initialize(&mut h, x, y);
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
                    assert_eq!(&h.bus.ram[start as usize..end as usize], reference);
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
                    let s = u32::from(before.s);
                    let expected = if name == "AddLong" {
                        x.wrapping_add(y)
                    } else {
                        x.wrapping_sub(y)
                    };
                    assert_eq!(h.bus.value(s + u32::from(dest), 4), expected);
                    assert_eq!(old_bus.value(s + u32::from(dest), 4), expected);
                    let expected_reads: Vec<_> = [a, b, a + 2, b + 2]
                        .into_iter()
                        .flat_map(|at| [s + u32::from(at), s + u32::from(at) + 1])
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
                        expected
                            .to_le_bytes()
                            .into_iter()
                            .enumerate()
                            .map(|(i, v)| (s + u32::from(dest) + i as u32, v))
                            .collect::<Vec<_>>()
                    );
                    assert!(
                        !h.bus.reads[reads..]
                            .iter()
                            .any(|a| (0x2000..0x2040).contains(a))
                    );
                    let after = h.cpu.registers();
                    assert_eq!(
                        (
                            after.s,
                            after.d,
                            after.dbr,
                            after.x,
                            after.y,
                            after.p & 0x3c
                        ),
                        (
                            before.s,
                            before.d,
                            before.dbr,
                            before.x,
                            before.y,
                            before.p & 0x3c
                        )
                    );
                    assert!(h.cpu.cycles() - cycles < old_cpu.cycles());
                    if x == 0xffff && mask == 0 {
                        eprintln!(
                            "{name} optimized={optimize}: 43->13 bytes, {}->{} cycles",
                            old_cpu.cycles(),
                            h.cpu.cycles() - cycles
                        );
                    }
                    h.run();
                    h.guards(mask);
                    check(&h, x, y);
                }
            }
        }
    }
}

#[test]
fn long_arithmetic_preserves_external_accesses_aliases_and_bank_carry() {
    let source = r#"
VOLATILE LONGCARD io=$D000
LONGCARD before=$7100,after=$7104,observed=$7108
LONGCARD POINTER p=$7110,q=$7113
PROC Change(LONGCARD POINTER left,right LONGCARD amount)
  left^=left^+amount right^=right^-1 observed=left^
RETURN
PROC Main() before=io+1 io=before-2 after=io+1 Change(p,q,1) RETURN
"#;
    for optimize in [false, true] {
        let image = compile(source, optimize);
        let caller = caller(image.entry);
        for alias in [false, true] {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                h.bus.map(0xd000, &u32::MAX.to_le_bytes(), true);
                let mut memory = [0xa5; 14];
                memory[1..5].copy_from_slice(&0x0000ffffu32.to_le_bytes());
                memory[9..13].copy_from_slice(&0x80000000u32.to_le_bytes());
                h.bus.map(0x12fffe, &memory, true);
                for (at, value) in [
                    (0x7110, 0x12ffffu32),
                    (0x7113, if alias { 0x12ffff } else { 0x130007 }),
                ] {
                    h.bus.ram[at..at + 3].copy_from_slice(&value.to_le_bytes()[..3]);
                }
                h.bus.watched.extend(0xd000..0xd004);
                h.bus.watched.extend(0x12fffe..0x13000c);
                h.run();
                h.guards(mask);
                assert_eq!(h.bus.value(0x7100, 4), 0);
                assert_eq!(h.bus.value(0x7104, 4), u32::MAX);
                let final_left = if alias { 0xffffu32 } else { 0x10000 };
                assert_eq!(h.bus.value(0x7108, 4), final_left);
                memory[1..5].copy_from_slice(&final_left.to_le_bytes());
                if !alias {
                    memory[9..13].copy_from_slice(&0x7fffffffu32.to_le_bytes());
                }
                assert_eq!(&h.bus.ram[0x12fffe..0x13000c], memory);
                let mut expected: Vec<_> = (0xd000..0xd004).map(|a| (a, Access::Read)).collect();
                expected.extend(
                    0xfffffffeu32
                        .to_le_bytes()
                        .into_iter()
                        .enumerate()
                        .map(|(i, v)| (0xd000 + i as u32, Access::Write(v))),
                );
                expected.extend((0xd000..0xd004).map(|a| (a, Access::Read)));
                let right = if alias { 0x12ffff } else { 0x130007 };
                for (at, value) in [
                    (0x12ffff, 0x10000u32),
                    (right, if alias { 0xffff } else { 0x7fffffff }),
                ] {
                    expected.extend((at..at + 4).map(|a| (a, Access::Read)));
                    expected.extend(
                        value
                            .to_le_bytes()
                            .into_iter()
                            .enumerate()
                            .map(|(i, v)| (at + i as u32, Access::Write(v))),
                    );
                }
                expected.extend((0x12ffff..0x130003).map(|a| (a, Access::Read)));
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

#[test]
fn long_arithmetic_survives_direct_and_indirect_calls_that_clobber_all_scratch() {
    let source = r#"
MODULE TEST
PUBLIC EXTERNAL LONGCARD FUNC Smash(LONGCARD ignored)
LONGCARD input=$7100,result=$7104
LONGCARD FUNC Work(LONGCARD n)
  LONGCARD FUNC POINTER cb(LONGCARD x)
  cb=@Smash
RETURN(((n+1)-Smash(n))+(n-cb(n)))
PROC Main() result=Work(input) RETURN
ENDMODULE
"#;
    let smash = assemble(
        "sep #$20\n.a8\nldx #63\nlda #$a7\nagain: sta 0,x\ndex\nbpl again\nrep #$20\n.a16\nldy #$dead\nldx #$89ab\nlda #$cdef\nrtl",
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
        for n in [0u32, 1, 0xffff, 0x10000, 0x7fffffff, 0x80000000, 0xffffffff] {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                h.bus.map(0x041000, &smash, false);
                h.bus.ram[0x7100..0x7104].copy_from_slice(&n.to_le_bytes());
                h.run();
                h.guards(mask);
                assert_eq!(
                    h.bus.value(0x7104, 4),
                    n.wrapping_add(1)
                        .wrapping_sub(0x89abcdef)
                        .wrapping_add(n.wrapping_sub(0x89abcdef))
                );
                assert!((0x2000..0x2040).all(|a| h.bus.writes.contains(&(a, 0xa7))));
            }
        }
    }
}

#[test]
fn long_arithmetic_executes_after_o65_relocation() {
    for ty in ["LONGCARD", "LONGINT"] {
        let source = source(ty).replace("a=$7100,b=$7104", "a,b");
        for optimize in [false, true] {
            let bytes = o65::compile(&source, optimize, vec![]);
            for variant in 0..2 {
                let placement = o65::placement(&bytes, variant, vec![o65::fault(variant)]);
                let image = format::relocate(&bytes, &placement).unwrap();
                let caller = caller(image.entry());
                for (a, b) in [
                    (0u32, 1u32),
                    (0xffff, 1),
                    (0x10000, 1),
                    (0x80000000, 0xffffffff),
                    (0xffffffff, 1),
                ] {
                    for mask in [0, 4] {
                        let mut h = Harness::new_o65(&image, &caller, mask);
                        initialize(&mut h, a, b);
                        for (name, value) in [("a", a), ("b", b)] {
                            let at = o65::object(&image, name) as usize;
                            h.bus.ram[at..at + 4].copy_from_slice(&value.to_le_bytes());
                        }
                        h.run();
                        h.guards(mask);
                        check(&h, a, b);
                    }
                }
            }
        }
    }
}
