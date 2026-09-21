mod support;
use actionc::mir65816::{
    self,
    image::{AssemblyImport, Image},
    *,
};
use actionc::nir::runtime_symbol_id;
use actionc::nir::{BlockId, NirBinaryOp, TempId};
use actionc::target::ByteSize;
use actionc_vm::native65816::{Access, Inputs};
use support::*;

#[test]
fn conditional_word_results_distinguish_all_relations_and_boundary_pairs() {
    let values = [0u16, 1, 0xff, 0x100, 0x7fff, 0x8000, 0xffff];
    for ty in ["CARD", "INT", "BYTE", "LONGCARD"] {
        let mut source = String::from("CARD a=$7100,b=$7102\nCARD ARRAY out=$7200\n");
        for (i, op) in ["=", "#", "<", "<=", ">", ">="].iter().enumerate() {
            source.push_str(&format!(
                "CARD FUNC F{i}({ty} x,y) IF x{op}y THEN RETURN($A55A) FI RETURN($5AA5)\n"
            ));
        }
        source.push_str("PROC Main()\n");
        for i in 0..6 {
            source.push_str(&format!("out({i})=F{i}({ty}(a),{ty}(b))\n"));
        }
        source.push_str("RETURN\n");
        for optimize in [false, true] {
            let prepared = prepare(&source, optimize);
            // Verify the actual adjacent shape survives both frontend modes.
            for r in prepared
                .mir
                .routines
                .iter()
                .filter(|r| r.name.starts_with('F'))
            {
                assert!(r.blocks.iter().any(|b| matches!((&b.ops.last(), &b.terminator),
                    (Some(Mir65816Op::Compare { dest, .. }), Mir65816Terminator::Branch { condition: Mir65816Value::Temp(id, _), .. }) if dest == id)));
            }
            let image = compile(&source, optimize);
            assert_eq!(
                image.to_json().unwrap(),
                compile(&source.replace('\n', "\r\n"), optimize)
                    .to_json()
                    .unwrap()
            );
            let caller = caller(image.entry);
            let interpret = |v: u16| match ty {
                "INT" => i32::from(v as i16),
                "BYTE" => i32::from(v as u8),
                _ => i32::from(v),
            };
            for a in values {
                for b in values {
                    for mask in [0, 4] {
                        let mut h = Harness::new(&image, &caller, mask);
                        h.bus.ram[0x7100..0x7102].copy_from_slice(&a.to_le_bytes());
                        h.bus.ram[0x7102..0x7104].copy_from_slice(&b.to_le_bytes());
                        let windows = run_checked_fusions(&mut h, &image);
                        assert_eq!(
                            windows.len(),
                            match ty {
                                "CARD" => 6,
                                "INT" => 2,
                                _ => 0,
                            }
                        );
                        h.guards(mask);
                        let (x, y) = (interpret(a), interpret(b));
                        for (i, truth) in [x == y, x != y, x < y, x <= y, x > y, x >= y]
                            .into_iter()
                            .enumerate()
                        {
                            assert_eq!(
                                h.bus.value(0x7200 + 2 * i as u32, 2),
                                if truth { 0xa55a } else { 0x5aa5 },
                                "{ty}/{optimize}/{a}/{b}/{i}"
                            );
                        }
                    }
                }
            }
        }
    }
}

use support::edges::program as edge_program;

#[test]
fn conditional_parallel_edges_and_backedge_rotations_execute() {
    for optimize in [false, true] {
        for backedge in [false, true] {
            let p = edge_program(optimize, backedge);
            let compiled = p.compile(&layout()).unwrap();
            let image = image::Image::from_json(&compiled.image.to_json().unwrap()).unwrap();
            let caller = caller(image.entry);
            for (a, b) in [(0u16, 0u16), (0xffff, 1), (1, 0xffff), (0x8000, 0x7fff)] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller, mask);
                    h.bus.ram[0x7100..0x7102].copy_from_slice(&a.to_le_bytes());
                    h.bus.ram[0x7102..0x7104].copy_from_slice(&b.to_le_bytes());
                    h.run();
                    h.guards(mask);
                    let expected = if backedge || a < b {
                        b.wrapping_sub(a)
                    } else {
                        a.wrapping_sub(b)
                    };
                    assert_eq!(h.bus.value(0x7200, 2), u32::from(expected));
                }
            }
        }
    }
}

#[test]
fn branch_inputs_preserve_volatile_alias_and_call_clobber_boundaries() {
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
  out(0)=flag IF saved<CARD($8000) THEN out(2)=1 ELSE out(2)=0 FI IF saved=observed THEN out(4)=1 ELSE out(4)=0 FI
  IF p^=CARD($1234) THEN out(6)=1 ELSE out(6)=0 FI IF io=observed THEN out(8)=1 ELSE out(8)=0 FI
  p^=CARD($FFFF)
  cb()
  IF p^=CARD($1234) THEN out(10)=1 ELSE out(10)=0 FI IF saved#CARD($1234) THEN out(12)=1 ELSE out(12)=0 FI
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
        let mut prepared = prepare(source, optimize);
        edges::split_word_loads(&mut prepared);
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
                let mut copies = 0;
                for _ in 0..100000 {
                    if h.cpu.is_stopped() {
                        break;
                    }
                    if h.cpu.is_instruction_boundary()
                        && word_edge::reached(&h.cpu, &h.bus, &image.routines).is_some()
                    {
                        copies += 1;
                    }
                    h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                }
                assert!(h.cpu.is_stopped() && copies >= 2);
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

fn run_checked_fusions(h: &mut Harness, image: &Image) -> Vec<serde_json::Value> {
    use actionc_vm::native65816::Inputs;
    let mut records = vec![];
    for _ in 0..200_000 {
        if h.cpu.is_stopped() {
            break;
        }
        if h.cpu.is_instruction_boundary() {
            if let Some(w) = comparison::fused_window(&h.cpu, &h.bus, &image.routines) {
                let r = h.cpu.registers();
                let reads = h.bus.reads.len();
                let writes = h.bus.writes.len();
                let expected: Vec<_> = w
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
                            |c| c.is_instruction_boundary() && [w.no, w.yes].contains(&c.pc())
                        )
                        .unwrap()
                );
                let end = h.cpu.registers();
                assert_eq!(
                    (end.s, end.d, end.dbr, end.x, end.y, end.p & 0x3c),
                    (r.s, r.d, r.dbr, r.x, r.y, r.p & 4)
                );
                assert!(h.bus.writes[writes..].is_empty());
                let actual: Vec<_> = h.bus.reads[reads..]
                    .iter()
                    .copied()
                    .filter(|a| (0x4000..0x6000).contains(a))
                    .collect();
                assert_eq!(actual, expected);
                assert!(
                    !h.bus.reads[reads..]
                        .iter()
                        .any(|a| (0x2000..0x2040).contains(a))
                );
                let selected = usize::from(h.cpu.pc() == w.yes);
                // Observe edge copies separately; at the successor the ABI width
                // is restored even when the true label invalidates mode knowledge.
                assert!(
                    h.cpu
                        .run_until(
                            &mut h.bus,
                            2000,
                            |_| Inputs::default(),
                            |c| c.is_instruction_boundary() && c.pc() == w.targets[selected]
                        )
                        .unwrap()
                );
                assert_eq!(h.cpu.registers().p & 0x3c, r.p & 4);
                records.push(serde_json::json!({"load":w.load,"cmp":w.cmp,"branch":w.branch,"true":selected==1,
                    "source_reads":actual,"comparison_writes":0,"comparison_dp_accesses":0,
                    "edge_writes":h.bus.writes[writes..],"edge":w.edges[selected]}));
                continue;
            }
        }
        h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
    }
    assert!(h.cpu.is_stopped());
    records
}
#[test]
fn fused_decisions_read_only_the_words_and_keep_parallel_edge_traffic_separate() {
    for optimize in [false, true] {
        let mut records = vec![];
        for backedge in [false, true] {
            let p = edge_program(optimize, backedge);
            let image =
                Image::from_json(&p.compile(&layout()).unwrap().image.to_json().unwrap()).unwrap();
            let caller = caller(image.entry);
            for (a, b) in [(0u16, 0u16), (0xffff, 1), (1, 0xffff)] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller, mask);
                    h.bus.ram[0x7100..0x7102].copy_from_slice(&a.to_le_bytes());
                    h.bus.ram[0x7102..0x7104].copy_from_slice(&b.to_le_bytes());
                    let reached = run_checked_fusions(&mut h, &image);
                    assert_eq!(reached.len(), if backedge { 4 } else { 1 });
                    h.guards(mask);
                    let expected = if backedge || a < b {
                        b.wrapping_sub(a)
                    } else {
                        a.wrapping_sub(b)
                    };
                    assert_eq!(h.bus.value(0x7200, 2), u32::from(expected));
                    records.push(serde_json::json!({"backedge":backedge,"args":[a,b],"mask":mask,"windows":reached}));
                }
            }
        }
        if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
            std::fs::write(
                std::path::Path::new(&directory)
                    .join(format!("fused-branch-traffic-{optimize}.json")),
                serde_json::to_vec_pretty(&records).unwrap(),
            )
            .unwrap();
        }
    }
}

#[test]
fn reused_conditions_and_intervening_operations_keep_materialized_booleans() {
    use actionc::target::{AddressValue, ByteOffset};
    use actionc_vm::native65816::Inputs;
    for optimize in [false, true] {
        for variant in 0..4 {
            let mut p = edge_program(optimize, false);
            let r = p
                .mir
                .routines
                .iter_mut()
                .find(|r| r.name == "Work")
                .unwrap();
            let flag = Mir65816Value::Temp(TempId(8), ByteSize::ONE);
            let address = Mir65816Address {
                base: Mir65816AddressBase::External(Mir65816ExternalAddress::Absolute(
                    AddressValue::data(0x7210),
                )),
                displacement: ByteOffset::ZERO,
                index: None,
                mode: Mir65816AddressMode::External,
            };
            let store = |value| Mir65816Op::Store {
                address: address.clone(),
                value,
                width: ByteSize::ONE,
                volatile: true,
            };
            match variant {
                0 => {
                    let ty = r.temps.iter().find(|(id, _)| id.0 == 8).unwrap().1.clone();
                    r.temps.push((TempId(10), ty));
                    let Mir65816Terminator::Branch {
                        then_edge,
                        else_edge,
                        ..
                    } = &mut r.blocks[0].terminator
                    else {
                        panic!()
                    };
                    then_edge.args.push(flag.clone());
                    else_edge.args.push(flag.clone());
                    r.blocks[1].params.push((TempId(10), ByteSize::ONE));
                    r.blocks[1]
                        .ops
                        .push(store(Mir65816Value::Temp(TempId(10), ByteSize::ONE)));
                }
                1 => r.blocks[1].ops.push(store(flag.clone())),
                2 => {
                    let ret = r.blocks[1].terminator.clone();
                    let edge = Mir65816Edge {
                        target: BlockId(3),
                        args: vec![],
                    };
                    r.blocks[1].terminator = Mir65816Terminator::Branch {
                        condition: flag.clone(),
                        then_edge: edge.clone(),
                        else_edge: edge,
                    };
                    r.blocks.push(Mir65816Block {
                        id: BlockId(3),
                        params: vec![],
                        ops: vec![],
                        terminator: ret,
                    });
                }
                3 => {
                    let ty = r.temps.iter().find(|(id, _)| id.0 == 0).unwrap().1.clone();
                    r.temps.push((TempId(10), ty));
                    r.blocks[0].ops.push(Mir65816Op::Binary {
                        dest: TempId(10),
                        width: ByteSize::new(2),
                        signed: false,
                        operation: NirBinaryOp::Add,
                        left: Mir65816Value::U16(0xffff),
                        right: Mir65816Value::U16(1),
                    });
                }
                _ => unreachable!(),
            }
            mir65816::verify_program(&p.mir).unwrap();
            let image =
                Image::from_json(&p.compile(&layout()).unwrap().image.to_json().unwrap()).unwrap();
            let caller = caller(image.entry);
            for (a, b) in [(1u16, 2u16), (2, 1)] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller, mask);
                    h.bus.ram[0x7100..0x7102].copy_from_slice(&a.to_le_bytes());
                    h.bus.ram[0x7102..0x7104].copy_from_slice(&b.to_le_bytes());
                    let mut materialized = 0;
                    for _ in 0..20000 {
                        if h.cpu.is_stopped() {
                            break;
                        }
                        if h.cpu.is_instruction_boundary() {
                            assert!(
                                comparison::fused_window(&h.cpu, &h.bus, &image.routines).is_none()
                            );
                            if comparison::window(&h.cpu, &h.bus, &image.routines).is_some() {
                                materialized += 1;
                            }
                        }
                        h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                    }
                    assert!(h.cpu.is_stopped());
                    h.guards(mask);
                    assert_eq!(materialized, 1);
                    assert_eq!(h.bus.value(0x7200, 2), 1);
                    if variant < 2 {
                        assert_eq!(h.bus.value(0x7210, 1), u32::from(a < b));
                    }
                }
            }
        }
    }
}

#[test]
fn fused_decoder_rejects_wrong_modes_truncated_windows_and_corrupt_edges() {
    use actionc_vm::native65816::{Inputs, Machine};
    let p = edge_program(false, false);
    let image = Image::from_json(&p.compile(&layout()).unwrap().image.to_json().unwrap()).unwrap();
    let mut h = Harness::new(&image, &caller(image.entry), 0);
    let window = loop {
        assert!(!h.cpu.is_stopped());
        if h.cpu.is_instruction_boundary() {
            if let Some(w) = comparison::fused_window(&h.cpu, &h.bus, &image.routines) {
                break w;
            }
        }
        h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
    };
    let r = image
        .routines
        .iter()
        .find(|r| (r.address..r.address + r.size).contains(&window.load))
        .unwrap();
    for end in [
        window.cmp,
        window.branch + 5,
        window.yes,
        window.edges[1].last().unwrap() + 3,
    ] {
        assert!(comparison::fused_in_range(&h.cpu, &h.bus, r.address..end).is_none());
    }
    for (at, byte) in [
        (window.cmp, 0xe3),
        (window.branch + 1, 3),
        (window.no, 0xc2),
        (window.yes, 0xea),
        (window.branch + 3, 0),
        (window.edges[1][1], 0xea),
    ] {
        let mut bus = h.bus.clone();
        bus.ram[at as usize] = byte;
        assert!(
            comparison::fused_window(&h.cpu, &bus, &image.routines).is_none(),
            "{at:x}"
        );
    }
    let mut registers = h.cpu.registers();
    registers.p |= 0x20;
    assert!(
        comparison::fused_window(&Machine::start_at(registers), &h.bus, &image.routines).is_none()
    );
}
