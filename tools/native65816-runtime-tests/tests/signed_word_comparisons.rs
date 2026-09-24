mod support;
use actionc::mir65816::image::Image;
use support::*;

const VALUES: [i16; 11] = [-32768, -32767, -257, -256, -1, 0, 1, 255, 256, 32766, 32767];

fn source() -> String {
    let mut s = String::from(
        "INT a=$7100,b=$7102\nBYTE ARRAY out=$7200\n\
         BYTE FUNC Ret(INT x,y) RETURN(x<y)\n\
         PROC Save(BYTE value) out(24)=value RETURN\n\
         PROC Work(INT x,y) BYTE saved\n",
    );
    for (i, op) in ["<", "<=", ">", ">="].iter().enumerate() {
        s.push_str(&format!(
            "out({})=(x{op}y)\nIF x{op}y THEN out({})=1 ELSE out({})=0 FI\n",
            2 * i,
            8 + 2 * i,
            8 + 2 * i
        ));
    }
    s.push_str(
        "out(16)=(x<INT(255)) out(18)=(INT(255)<x)\n\
         saved=(x>=y) out(20)=Ret(x,y) out(22)=saved Save(saved)\n\
         IF saved THEN out(26)=1 ELSE out(26)=0 FI\n\
         IF saved THEN out(28)=1 ELSE out(28)=0 FI\n\
         x==+1 out(30)=(x>y) RETURN\nPROC Main() Work(a,b) RETURN\n",
    );
    s
}

fn pairs() -> Vec<(i16, i16)> {
    let mut pairs: Vec<_> = VALUES
        .into_iter()
        .flat_map(|a| VALUES.map(|b| (a, b)))
        .collect();
    let mut seed = 0x8162_2026u32;
    for _ in 0..64 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        pairs.push((seed as i16, (seed >> 16) as i16));
        pairs.push(((seed >> 16) as i16, seed as i16));
    }
    pairs
}

#[test]
fn signed_predicates_and_boolean_consumers_cover_runtime_boundary_pairs() {
    let s = source();
    for optimize in [false, true] {
        narrow_comparison::check_shape(&s, optimize, 2);
        let p = prepare(&s, optimize);
        let c = p.compile(&layout()).unwrap();
        assert!(signed_comparison::check(&p, &c) >= 10);
        let image = Image::from_json(&c.image.to_json().unwrap()).unwrap();
        assert_eq!(
            image.to_json().unwrap(),
            compile(&s.replace('\n', "\r\n"), optimize)
                .to_json()
                .unwrap()
        );
        let caller = caller(image.entry);
        for (a, b) in pairs() {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                h.bus.ram[0x7100..0x7102].copy_from_slice(&a.to_le_bytes());
                h.bus.ram[0x7102..0x7104].copy_from_slice(&b.to_le_bytes());
                h.bus.ram[0x7200..0x7220].fill(0xa5);
                h.run();
                h.guards(mask);
                let relations = [a < b, a <= b, a > b, a >= b];
                let expected = relations.into_iter().chain(relations).chain([
                    a < 255,
                    255 < a,
                    a < b,
                    a >= b,
                    a >= b,
                    a >= b,
                    a >= b,
                    a.wrapping_add(1) > b,
                ]);
                for (i, truth) in expected.enumerate() {
                    assert_eq!(
                        h.bus.ram[0x7200 + 2 * i],
                        u8::from(truth),
                        "{optimize}/{a}/{b}/{mask}/{i}"
                    );
                    assert_eq!(h.bus.ram[0x7201 + 2 * i], 0xa5);
                }
                if a == 32767 && b == -1 && mask == 0 {
                    narrow_comparison::record("signed-word-comparisons", optimize, &s, &image, &h);
                }
            }
        }
    }
}

#[test]
fn signed_size_probes_keep_frames_and_source_newlines_stable() {
    let source =
        include_str!("../../../docs/benchmarks/65816-signed-word-comparisons/planning-probes.act")
            .replace("\r\n", "\n");
    for optimize in [false, true] {
        let image = compile(&source, optimize);
        assert_eq!(
            image.to_json().unwrap(),
            compile(&source.replace('\n', "\r\n"), optimize)
                .to_json()
                .unwrap()
        );
        for r in &image.routines {
            if r.name.starts_with("Ret") || r.name.starts_with("Branch") {
                assert_eq!(r.fixed_frame, 6);
                assert!(
                    r.size <= if r.name.starts_with("Ret") { 130 } else { 150 },
                    "{}: {}",
                    r.name,
                    r.size
                );
            } else if r.name == "Imm" {
                assert!(r.size < if optimize { 153 } else { 161 });
            }
        }
        let mut h = Harness::new(&image, &caller(image.entry), 0);
        h.run();
        h.guards(0);
        narrow_comparison::record("signed-size-probes", optimize, &source, &image, &h);
    }
}

#[test]
fn independent_a16_signed_subtraction_corrects_overflow_before_testing_sign() {
    use actionc_vm::native65816::{Inputs, Machine, Registers};
    for immediate in [false, true] {
        let code = assemble(
            &format!(
                "lda 2,s\nsec\n{}\nbvs overflow\njml corrected\noverflow: eor #$8000\ncorrected: bmi yes\nlda #0\nstp\nyes: lda #1\nstp\nnop",
                if immediate { "sbc #$ffff" } else { "sbc 4,s" }
            ),
            0x040000,
        );
        let n = usize::from(immediate);
        assert_eq!(&code[..3], &[0xa3, 2, 0x38]);
        assert_eq!(
            &code[3..5 + n],
            if immediate {
                &[0xe9, 0xff, 0xff][..]
            } else {
                &[0xe3, 4][..]
            }
        );
        assert_eq!(
            &code[5 + n..14 + n],
            &[0x70, 4, 0x5c, (14 + n) as u8, 0, 4, 0x49, 0, 0x80]
        );
        assert_eq!(code[14 + n], 0x30);
        for (a, b) in pairs() {
            let b = if immediate { -1 } else { b };
            for p in [0, 1, 0x40, 0x41, 4, 5, 0x44, 0x45] {
                let mut bus = Bus::new();
                bus.map(0x040000, &code, false);
                bus.map(0x4000, &[0xa5; 0x2000], true);
                bus.ram[0x5fe2..0x5fe4].copy_from_slice(&a.to_le_bytes());
                bus.ram[0x5fe4..0x5fe6].copy_from_slice(&b.to_le_bytes());
                let mut cpu = Machine::start_at(Registers {
                    a: 0xabcd,
                    x: 0x1234,
                    y: 0x5678,
                    s: 0x5fe0,
                    d: 0x2000,
                    dbr: 0,
                    pbr: 4,
                    pc: 0,
                    p,
                    emulation_mode: false,
                });
                assert!(
                    cpu.run_until(&mut bus, 100, |_| Inputs::default(), |c| c.is_stopped())
                        .unwrap()
                );
                let r = cpu.registers();
                assert_eq!(r.a, u16::from(a < b), "{a}/{b}/{p}");
                assert_eq!(
                    (r.x, r.y, r.s, r.d, r.dbr, r.p & 0x3c),
                    (0x1234, 0x5678, 0x5fe0, 0x2000, 0, p & 4)
                );
                assert!(bus.writes.is_empty());
            }
        }
    }
}

#[test]
fn signed_captures_survive_alias_mutation_and_complete_call_clobbers() {
    use actionc::mir65816::{abi, image::AssemblyImport};
    use actionc_vm::native65816::Access;
    let source = "MODULE TEST\nPUBLIC EXTERNAL PROC Smash()\nVOLATILE INT io=$D000\nBYTE ARRAY out=$7200\nPROC Main() INT POINTER p\nINT saved,observed\nBYTE flag\nPROC POINTER cb\np=INT POINTER($12FFFF) cb=@Smash\nsaved=p^ observed=io flag=(saved<INT(0))\nSmash() out(0)=flag out(2)=(saved<observed) out(4)=(saved>=observed)\nIF p^<saved THEN out(6)=1 ELSE out(6)=0 FI\nout(8)=(io<=observed)\np^=INT($7FFF) cb() out(10)=(p^>=INT(0))\nIF saved>INT($8000) THEN out(12)=1 ELSE out(12)=0 FI\nRETURN\nENDMODULE\n";
    let smash = assemble(
        "sep #$20\n.a8\nlda #0\nsta f:$12ffff\nlda #$80\nsta f:$130000\nldx #63\nlda #$a7\nagain: sta 0,x\ndex\nbpl again\nrep #$20\n.a16\nlda #$9876\nldx #$beef\nldy #$dead\nsep #$41\nrtl",
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
        let c = p.compile(&options).unwrap();
        assert_eq!(signed_comparison::check(&p, &c), 7);
        let image = Image::from_json(&c.image.to_json().unwrap()).unwrap();
        let caller = caller(image.entry);
        for value in VALUES {
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
                    value < 0,
                    false,
                    true,
                    i16::MIN < value,
                    true,
                    false,
                    value > i16::MIN,
                ]
                .into_iter()
                .enumerate()
                {
                    assert_eq!(h.bus.ram[0x7200 + 2 * i], u8::from(truth), "{value}/{i}");
                    assert_eq!(h.bus.ram[0x7201 + 2 * i], 0xa5);
                }
                assert_eq!(&h.bus.ram[0x12fffe..0x130002], &[0xa5, 0, 0x80, 0x5a]);
                assert_eq!(
                    h.bus
                        .trace
                        .iter()
                        .map(|&(_, a, k)| (a, k))
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

#[test]
fn signed_same_target_parallel_edges_and_backedges_preserve_values() {
    use actionc::mir65816::{self, Mir65816Op};
    use actionc::nir::NirCompareOp;
    for optimize in [false, true] {
        for backedge in [false, true] {
            let mut p = edges::program(optimize, backedge);
            for op in p
                .mir
                .routines
                .iter_mut()
                .filter(|r| r.name == "Work")
                .flat_map(|r| &mut r.blocks)
                .flat_map(|b| &mut b.ops)
            {
                if let Mir65816Op::Compare {
                    signed, operation, ..
                } = op
                {
                    *signed = true;
                    *operation = if backedge {
                        NirCompareOp::Gt
                    } else {
                        NirCompareOp::Lt
                    };
                }
            }
            mir65816::verify_program(&p.mir).unwrap();
            let c = p.compile(&layout()).unwrap();
            assert_eq!(signed_comparison::check(&p, &c), 1);
            let caller = caller(c.image.entry);
            for (a, b) in [
                (i16::MIN, i16::MAX),
                (i16::MAX, -1),
                (-1, i16::MAX),
                (0, 0),
                (1, -1),
            ] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&c.image, &caller, mask);
                    h.bus.ram[0x7100..0x7102].copy_from_slice(&a.to_le_bytes());
                    h.bus.ram[0x7102..0x7104].copy_from_slice(&b.to_le_bytes());
                    h.run();
                    h.guards(mask);
                    assert_eq!(
                        h.bus.value(0x7200, 2),
                        u32::from(if backedge || a < b {
                            b.wrapping_sub(a)
                        } else {
                            a.wrapping_sub(b)
                        } as u16)
                    );
                }
            }
        }
    }
}

#[test]
fn signed_materialization_and_fusion_relocate_across_banks() {
    use actionc::mir65816::o65 as format;
    let source = "INT a,b\nBYTE ARRAY output(8)\nPROC Main()\noutput(0)=(a<b) output(1)=(a<=b) output(2)=(a>b) output(3)=(a>=b)\nIF a<b THEN output(4)=1 ELSE output(4)=0 FI\nIF a<=b THEN output(5)=1 ELSE output(5)=0 FI\nIF a>b THEN output(6)=1 ELSE output(6)=0 FI\nIF a>=b THEN output(7)=1 ELSE output(7)=0 FI\nRETURN\n";
    for optimize in [false, true] {
        let bytes = o65::compile(source, optimize, vec![]);
        for variant in 0..2 {
            let placement = o65::placement(&bytes, variant, vec![o65::fault(variant)]);
            let image = format::relocate(&bytes, &placement).unwrap();
            let caller = caller(image.entry());
            let output = o65::object(&image, "output") as usize;
            for (a, b) in [
                (i16::MAX, -1),
                (i16::MIN, 1),
                (i16::MIN, i16::MAX),
                (i16::MAX, i16::MIN),
                (0, 0),
            ] {
                for mask in [0, 4] {
                    let mut h = Harness::new_o65(&image, &caller, mask);
                    for (name, value) in [("a", a), ("b", b)] {
                        let at = o65::object(&image, name) as usize;
                        h.bus.ram[at..at + 2].copy_from_slice(&value.to_le_bytes());
                    }
                    h.run();
                    h.guards(mask);
                    assert_eq!(
                        &h.bus.ram[output..output + 8],
                        &[a < b, a <= b, a > b, a >= b, a < b, a <= b, a > b, a >= b].map(u8::from)
                    );
                    o65::record(
                        "signed-word-comparisons",
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
fn signed_windows_read_both_private_words_and_fusion_omits_the_boolean_store() {
    use actionc::mir65816::{Mir65816Op, Mir65816Value};
    use actionc_vm::native65816::Inputs;
    for branch in [false, true] {
        let source = format!(
            "INT a=$7100,b=$7102\nBYTE out=$7200\nBYTE FUNC Test(INT x,y) {}\nPROC Main() out=Test(a,b) RETURN\n",
            if branch {
                "IF x<y THEN RETURN(1) FI RETURN(0)"
            } else {
                "RETURN(x<y)"
            }
        );
        for optimize in [false, true] {
            let p = prepare(&source, optimize);
            let c = p.compile(&layout()).unwrap();
            assert_eq!(signed_comparison::check(&p, &c), 1);
            let r = p.mir.routines.iter().find(|r| r.name == "Test").unwrap();
            let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
            let linked = c.image.routines.iter().find(|r| r.id == m.id.0).unwrap();
            let (block, i, dest, left, right) = r
                .blocks
                .iter()
                .find_map(|b| {
                    b.ops.iter().enumerate().find_map(|(i, op)| match op {
                        Mir65816Op::Compare {
                            dest,
                            left: Mir65816Value::Temp(a, _),
                            right: Mir65816Value::Temp(c, _),
                            ..
                        } => Some((b.id, i, *dest, *a, *c)),
                        _ => None,
                    })
                })
                .unwrap();
            let span = &m.code.mir_spans[&(block, i)];
            let start = linked.address + span.start as u32;
            let finish = linked.address
                + if branch {
                    m.code
                        .conditional_branches
                        .iter()
                        .find(|s| s.dispatch && span.contains(&s.offset))
                        .unwrap()
                        .offset as u32
                } else {
                    span.end as u32
                };
            let caller = caller(c.image.entry);
            for (a, b) in [
                (i16::MAX, -1),
                (i16::MIN, 1),
                (i16::MIN, i16::MAX),
                (i16::MAX, i16::MIN),
                (0, 0),
            ] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&c.image, &caller, mask);
                    h.bus.ram[0x7100..0x7102].copy_from_slice(&a.to_le_bytes());
                    h.bus.ram[0x7102..0x7104].copy_from_slice(&b.to_le_bytes());
                    assert!(
                        h.cpu
                            .run_until(
                                &mut h.bus,
                                10000,
                                |_| Inputs::default(),
                                |cpu| cpu.is_instruction_boundary() && cpu.pc() == start
                            )
                            .unwrap()
                    );
                    let s = u32::from(h.cpu.registers().s);
                    let reads = h.bus.reads.len();
                    let writes = h.bus.writes.len();
                    assert!(
                        h.cpu
                            .run_until(
                                &mut h.bus,
                                1000,
                                |_| Inputs::default(),
                                |cpu| cpu.is_instruction_boundary() && cpu.pc() == finish
                            )
                            .unwrap()
                    );
                    let expected: Vec<_> = [left, right]
                        .into_iter()
                        .flat_map(|id| {
                            let at = s + u32::from(m.frame.temps[&id].stack().unwrap().offset);
                            [at, at + 1]
                        })
                        .collect();
                    assert_eq!(
                        h.bus.reads[reads..]
                            .iter()
                            .copied()
                            .filter(|a| (0x4000..0x6000).contains(a))
                            .collect::<Vec<_>>(),
                        expected
                    );
                    let expected_write = if branch {
                        vec![]
                    } else {
                        vec![(
                            s + u32::from(m.frame.temps[&dest].stack().unwrap().offset),
                            u8::from(a < b),
                        )]
                    };
                    assert_eq!(&h.bus.writes[writes..], &expected_write);
                    assert!(
                        !h.bus.reads[reads..]
                            .iter()
                            .any(|a| (0x2000..0x2040).contains(a))
                    );
                    h.run();
                    h.guards(mask);
                    assert_eq!(h.bus.ram[0x7200], u8::from(a < b));
                }
            }
        }
    }
}
