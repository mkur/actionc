mod support;
use actionc::mir65816::{abi, image::AssemblyImport, image::Image};
use actionc::nir::runtime_symbol_id;
use actionc_vm::native65816::{Access, Inputs, Machine, Registers};
use support::*;

#[test]
fn word_relations_materialize_boolean_bytes_for_all_boundary_pairs() {
    let values = [0u16, 1, 0xff, 0x100, 0x7fff, 0x8000, 0xffff];
    for ty in ["CARD", "INT"] {
        let mut source = format!(
            "{ty} a=$7100,b=$7102\nBYTE ARRAY out=$7200\n\
             BYTE FUNC Less({ty} x,y) RETURN(x<y)\n\
             PROC Work({ty} x,y)\nBYTE saved\n\
             out(30)=BYTE(x)+1\n"
        );
        for (i, op) in ["=", "#", "<", "<=", ">", ">="].iter().enumerate() {
            source.push_str(&format!("out({})=(x{op}y)\n", 2 * i));
        }
        source.push_str(&format!(
            "out(12)=(x<{ty}(255)) out(14)=({ty}(255)<x)\n\
             out(16)=(x<=y) out(18)=(x=x)\n\
             saved=(x>=y) out(20)=Less(x,y) out(22)=saved\n\
             IF x>y THEN out(24)=1 ELSE out(24)=0 FI\n\
             out(26)=(x={ty}(BYTE(255)))\n\
             x==+1 out(28)=(x>y)\nRETURN\n\
             PROC Main() Work(a,b) RETURN\n"
        ));
        for optimize in [false, true] {
            let image = compile(&source, optimize);
            assert_eq!(
                image.to_json().unwrap(),
                compile(&source.replace('\n', "\r\n"), optimize)
                    .to_json()
                    .unwrap()
            );
            let caller = caller(image.entry);
            let interpret = |n: u16| {
                if ty == "INT" {
                    i32::from(n as i16)
                } else {
                    i32::from(n)
                }
            };
            for a in values {
                for b in values {
                    for mask in [0, 4] {
                        let mut h = Harness::new(&image, &caller, mask);
                        h.bus.ram[0x7100..0x7102].copy_from_slice(&a.to_le_bytes());
                        h.bus.ram[0x7102..0x7104].copy_from_slice(&b.to_le_bytes());
                        h.bus.ram[0x7200..0x7220].fill(0xa5);
                        h.run();
                        h.guards(mask);
                        let (x, y) = (interpret(a), interpret(b));
                        for (i, truth) in [
                            x == y,
                            x != y,
                            x < y,
                            x <= y,
                            x > y,
                            x >= y,
                            x < 255,
                            255 < x,
                            x <= y,
                            true,
                            x < y,
                            x >= y,
                            x > y,
                            x == 255,
                            interpret(a.wrapping_add(1)) > y,
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            assert_eq!(
                                h.bus.ram[0x7200 + i * 2],
                                u8::from(truth),
                                "{ty}/{optimize}/{a:x}/{b:x}/{mask}/{i}"
                            );
                        }
                        assert_eq!(h.bus.ram[0x721e], (a as u8).wrapping_add(1));
                        assert!((0..16).all(|i| h.bus.ram[0x7201 + i * 2] == 0xa5));
                    }
                }
            }
        }
    }
}

#[test]
fn captured_words_and_booleans_survive_alias_mutation_and_full_call_clobbers() {
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
  out(0)=flag out(2)=(saved<CARD($8000)) out(4)=(saved=observed)
  out(6)=(p^=CARD($1234)) out(8)=(io=observed)
  p^=CARD($FFFF)
  cb()
  out(10)=(p^=CARD($1234)) out(12)=(saved#CARD($1234))
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

#[test]
fn independent_word_cmp_encodings_execute_with_arbitrary_incoming_carry_and_overflow() {
    for immediate in [false, true] {
        let code = assemble(
            &format!(
                "rep #$20\nlda 2,s\n{}\nphp\nsep #$20\n.a8\npla\nsta f:$007200\nrep #$20\n.a16\nstp\nnop",
                if immediate { "cmp #$8000" } else { "cmp 4,s" }
            ),
            0x040000,
        );
        let mut expected = vec![0xc2, 0x20, 0xa3, 2];
        expected.extend(if immediate {
            vec![0xc9, 0, 0x80]
        } else {
            vec![0xc3, 4]
        });
        expected.extend([
            0x08, 0xe2, 0x20, 0x68, 0x8f, 0, 0x72, 0, 0xc2, 0x20, 0xdb, 0xea,
        ]);
        assert_eq!(code, expected);
        for (a, b) in [
            (0u16, 0u16),
            (0xffff, 1),
            (0x8000, 0x7fff),
            (0x100, 0xff),
            (0, 0xffff),
            (0x8000, 0x8000),
        ] {
            let b = if immediate { 0x8000 } else { b };
            for p in [0, 1, 0x40, 0x41, 0x24, 0x65] {
                let mut bus = Bus::new();
                bus.map(0x040000, &code, false);
                bus.map(0x4000, &[0xa5; 0x2000], true);
                bus.map(0x7200, &[0xa5, 0x5a], true);
                bus.ram[0x5fe2..0x5fe4].copy_from_slice(&a.to_le_bytes());
                bus.ram[0x5fe4..0x5fe6].copy_from_slice(&b.to_le_bytes());
                let initial = Registers {
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
                };
                let mut cpu = Machine::start_at(initial);
                assert!(
                    cpu.run_until(&mut bus, 500, |_| Inputs::default(), |c| c.is_stopped())
                        .unwrap()
                );
                assert_eq!(
                    bus.ram[0x7200] & 0xc3,
                    (p & 0x40)
                        | u8::from(a >= b)
                        | (u8::from(a == b) << 1)
                        | ((a.wrapping_sub(b) >> 8) as u8 & 0x80)
                );
                assert_eq!(bus.ram[0x7201], 0x5a);
                let end = cpu.registers();
                assert_eq!(
                    (end.s, end.d, end.dbr, end.x, end.y),
                    (initial.s, initial.d, initial.dbr, initial.x, initial.y)
                );
                assert_eq!(end.p & 0x3c, p & 4);
            }
        }
    }
}

#[test]
fn executed_comparisons_read_complete_sources_and_write_one_boolean_without_scratch() {
    let source = r#"
CARD a=$7100,b=$7102
BYTE ARRAY out=$7200
BYTE FUNC Eq(CARD x,y) RETURN(x=y)
BYTE FUNC Ne(CARD x,y) RETURN(x#y)
BYTE FUNC Lt(CARD x,y) RETURN(x<y)
BYTE FUNC Le(CARD x,y) RETURN(x<=y)
BYTE FUNC Gt(CARD x,y) RETURN(x>y)
BYTE FUNC Ge(CARD x,y) RETURN(x>=y)
BYTE FUNC Left(CARD x) RETURN(CARD($8000)<x)
BYTE FUNC Right(CARD x) RETURN(x<CARD($8000))
PROC Main()
  out(0)=Eq(a,b) out(1)=Ne(a,b) out(2)=Lt(a,b) out(3)=Le(a,b)
  out(4)=Gt(a,b) out(5)=Ge(a,b) out(6)=Left(a) out(7)=Right(a)
RETURN
"#;
    for optimize in [false, true] {
        let image = compile(source, optimize);
        let caller = caller(image.entry);
        let mut records = vec![];
        for (a, b) in [(0u16, 0u16), (0xffff, 1), (0x8000, 0x7fff), (0, 0xffff)] {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                h.bus.ram[0x7100..0x7102].copy_from_slice(&a.to_le_bytes());
                h.bus.ram[0x7102..0x7104].copy_from_slice(&b.to_le_bytes());
                let expected = [
                    a == b,
                    a != b,
                    a < b,
                    a <= b,
                    a > b,
                    a >= b,
                    0x8000 < a,
                    a < 0x8000,
                ];
                let mut count = 0;
                for _ in 0..100_000 {
                    if h.cpu.is_stopped() {
                        break;
                    }
                    if h.cpu.is_instruction_boundary() {
                        if let Some(w) = comparison::window(&h.cpu, &h.bus, &image.routines) {
                            let r = h.cpu.registers();
                            let start_reads = h.bus.reads.len();
                            let start_writes = h.bus.writes.len();
                            let stack_reads: Vec<_> = w
                                .sources
                                .iter()
                                .filter(|s| s.0)
                                .flat_map(|s| {
                                    [
                                        u32::from(r.s) + u32::from(s.1),
                                        u32::from(r.s) + u32::from(s.1) + 1,
                                    ]
                                })
                                .collect();
                            assert!(
                                h.cpu
                                    .run_until(
                                        &mut h.bus,
                                        100,
                                        |_| Inputs::default(),
                                        |cpu| cpu.is_instruction_boundary() && cpu.pc() == w.end
                                    )
                                    .unwrap()
                            );
                            let end = h.cpu.registers();
                            assert_eq!(
                                (end.s, end.d, end.dbr, end.x, end.y, end.p & 0x3c),
                                (r.s, r.d, r.dbr, r.x, r.y, mask | 0x20)
                            );
                            assert_eq!(
                                &h.bus.writes[start_writes..],
                                &[(
                                    u32::from(r.s) + u32::from(w.dest),
                                    u8::from(expected[count])
                                )]
                            );
                            let reads = &h.bus.reads[start_reads..];
                            assert!(!reads.iter().any(|p| (0x2000..0x2040).contains(p)));
                            assert_eq!(
                                reads
                                    .iter()
                                    .copied()
                                    .filter(|p| (0x4000..0x6000).contains(p))
                                    .collect::<Vec<_>>(),
                                stack_reads
                            );
                            records.push(serde_json::json!({"args":[a,b],"mask":mask,"load":w.load,"cmp":w.cmp,
                                "end":w.end,"sources":w.sources,"stack_reads":stack_reads,
                                "writes":h.bus.writes[start_writes..],"dp_reads_writes":0}));
                            count += 1;
                        }
                    }
                    h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                }
                assert!(h.cpu.is_stopped());
                h.guards(mask);
                assert_eq!(count, 8);
                assert_eq!(&h.bus.ram[0x7200..0x7208], expected.map(u8::from));
            }
        }
        if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
            std::fs::write(
                std::path::Path::new(&directory)
                    .join(format!("comparison-traffic-{optimize}.json")),
                serde_json::to_vec_pretty(&records).unwrap(),
            )
            .unwrap();
        }
    }
}

#[test]
fn representative_word_comparison_kernels_keep_cycle_and_stack_budgets() {
    for optimize in [false, true] {
        for (name, source, outgoing, answer, byte_limit, cycle_limit, frame) in [
            (
                "maximum",
                "CARD FUNC Work(CARD x,y) IF x>y THEN RETURN(x) FI RETURN(y) PROC Main() RETURN",
                5,
                41,
                155,
                160,
                6,
            ),
            (
                "sum-loop",
                "CARD FUNC Work(CARD n) CARD total total=0 WHILE n#0 DO total==+n n==-1 OD RETURN(total) PROC Main() RETURN",
                3,
                91,
                220,
                if optimize { 2500 } else { 2600 },
                if optimize { 16 } else { 14 },
            ),
        ] {
            let image = compile(source, optimize);
            let work = image.routines.iter().find(|r| r.name == "Work").unwrap();
            let caller = assemble_artifact(
                &format!(
                    "tsc\nsec\nsbc #{outgoing}\ntcs\nsep #$20\n.a8\nlda #0\nsta {outgoing},s\nrep #$20\n.a16\nlda f:$007100\nsta 1,s\n{}jsl ${:06x}\n.export returned\nreturned: sta f:$007200\ntsc\nclc\nadc #{outgoing}\ntcs\nstp\nnop",
                    if outgoing == 5 {
                        "lda f:$007102\nsta 3,s\n"
                    } else {
                        ""
                    },
                    work.address
                ),
                0x040000,
            );
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller.bytes, mask);
                h.bus.ram[0x7100..0x7104].copy_from_slice(&[13, 0, 41, 0]);
                assert!(
                    h.cpu
                        .run_until(
                            &mut h.bus,
                            1000,
                            |_| Inputs::default(),
                            |c| c.is_instruction_boundary() && c.pc() == work.address
                        )
                        .unwrap()
                );
                let start = h.cpu.cycles();
                let entry_s = h.cpu.registers().s;
                let mut lowest_s = entry_s;
                assert!(
                    h.cpu
                        .run_until(
                            &mut h.bus,
                            10_000,
                            |_| Inputs::default(),
                            |c| {
                                lowest_s = lowest_s.min(c.registers().s);
                                c.is_instruction_boundary() && c.pc() == caller.symbols["returned"]
                            }
                        )
                        .unwrap()
                );
                let cycles = h.cpu.cycles() - start;
                assert_eq!(h.cpu.registers().a, answer);
                assert!(
                    work.size <= byte_limit && cycles <= cycle_limit,
                    "{name}/{optimize}: {} bytes/{cycles} cycles",
                    work.size
                );
                assert_eq!(
                    (work.fixed_frame, work.local_stack_peak, entry_s - lowest_s),
                    (frame, frame, frame as u16)
                );
                if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
                    std::fs::write(std::path::Path::new(&directory).join(format!("comparison-budget-{name}-{optimize}.json")),
                        serde_json::to_vec_pretty(&serde_json::json!({"code_bytes":work.size,"cycles":cycles,"frame":frame,
                            "observed_stack":entry_s-lowest_s,"byte_limit":byte_limit,"cycle_limit":cycle_limit})).unwrap()).unwrap();
                }
                h.run();
                h.guards(mask);
            }
        }
    }
}
