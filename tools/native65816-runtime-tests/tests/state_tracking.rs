mod support;
use actionc::mir65816::emit::proof::{self, Snapshot, Value, Width};
use actionc_vm::native65816::{Inputs, Machine, Registers};
use support::*;

fn known(value: Value, entry_s: u16) -> Option<(u16, u16)> {
    match value {
        Value::Constant(v, w) => Some((v, if w == Width::Byte { 0xff } else { 0xffff })),
        Value::StackAddress(offset) => Some((entry_s.wrapping_add(offset as u16), 0xffff)),
        _ => None,
    }
}
fn check(s: &Snapshot, r: Registers, entry_s: u16, bus: &Bus, irq: u8) {
    assert!(!r.emulation_mode && s.native);
    if let Some(d) = s.decimal {
        assert_eq!(r.p & 8 != 0, d);
    }
    if let Some(b) = s.dbr {
        assert_eq!(r.dbr, b);
    }
    if s.current_domain {
        assert_eq!(r.d, 0x2000);
    }
    if s.irq_preserved {
        assert_eq!(r.p & 4, irq);
    }
    assert_eq!(
        (r.p & 0x20 != 0, r.p & 0x10 != 0),
        (s.m == Width::Byte, s.index == Width::Byte),
        "{s:?}"
    );
    assert_eq!(r.s, entry_s.wrapping_sub(s.depth as u16));
    for (fact, actual) in [(s.a, r.a), (s.x, r.x), (s.y, r.y)] {
        if let Some((v, mask)) = known(fact, entry_s) {
            assert_eq!(actual & mask, v, "{s:?}");
        }
    }
    // Compare simultaneous facts, never values from different loop iterations.
    let mut observed = vec![(s.a, r.a), (s.x, r.x), (s.y, r.y)];
    for h in &s.homes {
        let at = entry_s
            .wrapping_sub(s.anchor.unwrap_or(0) as u16)
            .wrapping_add(h.offset);
        observed.push((
            h.value,
            bus.value(u32::from(at), usize::from(h.width)) as u16,
        ));
    }
    for (i, (fact, actual)) in observed.iter().enumerate() {
        if let Some((v, mask)) = known(*fact, entry_s) {
            assert_eq!(actual & mask, v, "home/register: {s:?}");
        }
        if let Value::Opaque(_, width) = fact {
            let mask = if *width == Width::Byte { 0xff } else { 0xffff };
            for (other, value) in &observed[i + 1..] {
                if other == fact {
                    assert_eq!(actual & mask, value & mask, "relation: {s:?}");
                }
            }
        }
    }
    let nz = known(s.nz, entry_s).or_else(|| match s.nz {
        Value::Opaque(_, w) => observed.iter().find(|(v, _)| *v == s.nz).map(|(_, v)| {
            (
                *v & if w == Width::Byte { 0xff } else { 0xffff },
                if w == Width::Byte { 0xff } else { 0xffff },
            )
        }),
        _ => None,
    });
    if let Some((v, mask)) = nz {
        assert_eq!(r.p & 2 != 0, v == 0, "Z: {s:?}");
        assert_eq!(r.p & 0x80 != 0, v & ((mask >> 1) + 1) != 0, "N: {s:?}");
    }
    if let Some(c) = s.carry {
        assert_eq!(r.p & 1 != 0, c, "C: {s:?}");
    }
    if let Some(v) = s.overflow {
        assert_eq!(r.p & 0x40 != 0, v, "V: {s:?}");
    }
}
#[test]
fn typed_facade_encodings_and_claims_match_independent_ca65_and_vm() {
    for (left, right) in [
        (0, 0),
        (0xffff, 1),
        (0x7fff, 1),
        (0x8000, 0xffff),
        (0x100, 0xff),
    ] {
        let (code, trace) = proof::arithmetic_probe(left, right);
        let independent = assemble(
            &format!(
                "rep #$20\nlda #{left}\nclc\nadc #{right}\nsta 2,s\ntax\nsec\nsbc #{right}\ncmp #{left}\ntay\nsep #$20\n.a8\nlda #$80\nxba\nrep #$20\n.a16\ntsc\nclc\nadc #0\ntcs\nstp\nnop"
            ),
            0x40000,
        );
        assert_eq!(code.bytes, independent[..code.bytes.len()]);
        for irq in [0, 4] {
            let mut bus = Bus::new();
            bus.map(0x40000, &independent, false);
            bus.map(0x4000, &[0; 0x2000], true);
            let entry_s = 0x5fe0;
            let mut cpu = Machine::start_at(Registers {
                a: 0xabcd,
                x: 0x1234,
                y: 0x5678,
                s: entry_s,
                d: 0x2000,
                dbr: 0,
                pbr: 4,
                pc: 0,
                p: irq,
                emulation_mode: false,
            });
            for snapshot in &trace {
                assert!(
                    cpu.run_until(
                        &mut bus,
                        1000,
                        |_| Inputs::default(),
                        |c| c.is_instruction_boundary() && c.pc() == 0x40000 + snapshot.pc as u32
                    )
                    .unwrap()
                );
                check(snapshot, cpu.registers(), entry_s, &bus, irq);
            }
            assert_eq!(
                bus.value(u32::from(entry_s) + 2, 2),
                u32::from(left.wrapping_add(right))
            );
        }
    }
}

#[test]
fn memory_index_rmw_and_full_status_effects_match_independent_execution() {
    for byte in [false, true] {
        let (code, trace) = proof::memory_probe(byte);
        let independent = assemble(
            &format!(
                r#"
{mode}
lda #{value}
tax
sta 2,s
lda 2,s
clc
adc 2,s
sec
sbc 2,s
cmp 2,s
sta $08
lda $08
clc
adc $08
sec
sbc $08
cmp $08
and $08
ora $08
eor $08
asl $08
rol $08
lsr $08
ror $08
ldx $08
ldy #2
lda [$00]
sta [$00]
lda [$00],y
sta [$00],y
lda f:$120004
sta f:$120006
txa
tay
tya
dex
dec a
{word}
and #$ff
sep #$20
.a8
eor #$80
clc
adc #1
sec
sbc #1
cmp #$80
sep #$df
rep #$ff
sep #$20
.a8
tsc
tax
tcs
rep #$20
.a16
nop
stp
nop
"#,
                mode = if byte { "sep #$20\n.a8" } else { "rep #$20" },
                value = if byte { 0x81 } else { 0x8001 },
                word = if byte { "rep #$20\n.a16" } else { "" }
            ),
            0x40000,
        );
        assert_eq!(code.bytes, independent[..code.bytes.len()]);
        for irq in [0, 4] {
            let mut bus = Bus::new();
            bus.map(0x40000, &independent, false);
            bus.map(0x4000, &[0; 0x2000], true);
            bus.map(0x2000, &[0; 256], true);
            bus.ram[0x2000..0x2003].copy_from_slice(&[0, 0, 0x12]);
            bus.map(0x120000, &[0x55, 0xaa, 0, 0x80, 0xff, 0x7f, 0, 0], true);
            let entry_s = 0x5fe0;
            let mut cpu = Machine::start_at(Registers {
                a: 0xabcd,
                x: 0x1234,
                y: 0x5678,
                s: entry_s,
                d: 0x2000,
                dbr: 0,
                pbr: 4,
                pc: 0,
                p: irq,
                emulation_mode: false,
            });
            for snapshot in &trace {
                assert!(
                    cpu.run_until(
                        &mut bus,
                        2000,
                        |_| Inputs::default(),
                        |c| c.is_instruction_boundary() && c.pc() == 0x40000 + snapshot.pc as u32
                    )
                    .unwrap()
                );
                check(snapshot, cpu.registers(), entry_s, &bus, irq);
            }
        }
    }
}

#[test]
fn actual_raw_and_optimized_traces_survive_linking_loops_and_o65_rebasing() {
    trace_loop(
        "MODULE Probe PUBLIC CARD FUNC Work(CARD n) CARD total total=0 WHILE n#0 DO total==+n n==-1 OD RETURN(total) PROC Main() RETURN ENDMODULE",
        0,
    );
}
#[test]
fn frame_forwarding_traces_preserve_generation_and_nz_claims_in_images_and_o65() {
    trace_loop(&fixture("code_quality/loop_rotation.act"), 1);
}
#[test]
fn incoming_parameter_traces_verify_read_and_capture_generations() {
    trace_loop(
        "MODULE Probe PUBLIC CARD FUNC WorkParamPair(CARD pad,x) RETURN(x+x) PROC Main() RETURN ENDMODULE",
        2,
    );
}
fn trace_loop(source: &str, kind: u8) {
    use actionc::mir65816::{emit, image, o65 as format};
    use std::collections::BTreeMap;
    for optimize in [false, true] {
        let prepared = if kind == 2 {
            parameter_forwarding::prepared(source, optimize)
        } else {
            prepare(source, optimize)
        };
        let ordinary = emit::materialize(&prepared.mir).unwrap();
        let (traced, traces) = proof::materialize_with_trace(&prepared.mir).unwrap();
        let plain = image::link(&prepared.mir, &ordinary, &layout()).unwrap();
        let image = image::link(&prepared.mir, &traced, &layout()).unwrap();
        assert_eq!(plain.to_json().unwrap(), image.to_json().unwrap());
        for (a, b) in ordinary.routines.iter().zip(&traced.routines) {
            assert_eq!(a.code.bytes, b.code.bytes);
            assert_eq!(a.code.labels, b.code.labels);
            assert_eq!(a.code.return_fixups, b.code.return_fixups);
            assert_eq!(a.code.mir_spans, b.code.mir_spans);
            assert_eq!(
                format!("{:?}", a.code.fixups),
                format!("{:?}", b.code.fixups)
            );
        }
        let work = image
            .routines
            .iter()
            .find(|r| r.name.to_lowercase().contains("work"))
            .unwrap();
        let routine = traced.routines.iter().find(|r| r.id.0 == work.id).unwrap();
        let trace = &traces
            .iter()
            .find(|t| t.routine == routine.id)
            .unwrap()
            .snapshots;
        let at: BTreeMap<_, _> = trace
            .iter()
            .filter(|s| {
                s.pc < routine.code.bytes.len() && s.event != proof::Event::IndirectTransfer
            })
            .map(|s| (s.pc, s))
            .collect();
        let object =
            format::write(&format::prepare(&prepared.mir, &Default::default()).unwrap()).unwrap();
        assert_eq!(
            object,
            prepared.compile_o65(&Default::default()).unwrap().bytes
        );
        for variant in 0..3 {
            let relocated = if variant > 0 {
                Some(
                    format::relocate(
                        &object,
                        &support::o65::placement(
                            &object,
                            variant - 1,
                            vec![support::o65::fault(variant - 1)],
                        ),
                    )
                    .unwrap(),
                )
            } else {
                None
            };
            let base = relocated.as_ref().map_or(work.address, |r| {
                support::o65::routine(r, if kind == 2 { "WorkParamPair" } else { "Work" })
            });
            let overflow = relocated
                .as_ref()
                .map_or(layout().stack_overflow, |r| r.stack_overflow());
            let mut expected = routine.code.bytes.clone();
            for fixup in &routine.code.fixups {
                assert_eq!(fixup.byte, None);
                let target = match fixup.target {
                    emit::Target::Label(l) => base + routine.code.labels[&l] as u32,
                    emit::Target::StackOverflow => overflow,
                    _ => panic!("unexpected leaf relocation"),
                } + fixup.addend;
                expected[fixup.offset..fixup.offset + 3]
                    .copy_from_slice(&target.to_le_bytes()[..3]);
            }
            for n in if kind == 2 {
                vec![0u16, 1, 0x7fff, 0x8000, 0xffff]
            } else {
                vec![0u16, 1, 13]
            } {
                let outgoing = if kind == 2 { 5 } else { 3 };
                let arg = if kind == 2 { 3 } else { 1 };
                let caller = assemble(
                    &format!(
                        "tsc\nsec\nsbc #{outgoing}\ntcs\nlda #{n}\nsta {arg},s\njsl ${base:06x}\nsta $7000\ntsc\nclc\nadc #{outgoing}\ntcs\nstp\nnop"
                    ),
                    0x40000,
                );
                for irq in [0, 4] {
                    let mut h = if let Some(ref r) = relocated {
                        Harness::new_o65(r, &caller, irq)
                    } else {
                        Harness::new(&image, &caller, irq)
                    };
                    assert_eq!(
                        &h.bus.ram[base as usize..base as usize + expected.len()],
                        expected
                    );
                    let mut observations = 0;
                    for _ in 0..100_000 {
                        if h.cpu.is_stopped() {
                            break;
                        }
                        if h.cpu.is_instruction_boundary() {
                            if let Some(s) = h
                                .cpu
                                .pc()
                                .checked_sub(base)
                                .and_then(|pc| at.get(&(pc as usize)))
                            {
                                check(s, h.cpu.registers(), 0x5ff0 - outgoing - 3, &h.bus, irq);
                                observations += 1;
                            }
                        }
                        h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                    }
                    assert!(h.cpu.is_stopped() && observations > 10);
                    h.guards(irq);
                    assert_eq!(
                        h.bus.value(0x7000, 2),
                        if kind == 2 {
                            u32::from(n.wrapping_mul(2))
                        } else if kind == 1 {
                            u32::from(n) * 2 + 9
                        } else {
                            u32::from(n) * (u32::from(n) + 1) / 2
                        }
                    );
                    if let Ok(dir) = std::env::var("A816_QUALIFICATION_DIR") {
                        let stem = std::path::Path::new(&dir).join(format!(
                            "state-{}-{optimize}-{variant}-{n}-{irq}",
                            if kind == 2 {
                                "parameter-forwarding"
                            } else if kind == 1 {
                                "frame-forwarding"
                            } else {
                                "tracker"
                            }
                        ));
                        std::fs::write(stem.with_extension("bin"), &expected).unwrap();
                        std::fs::write(
                            stem.with_extension("txt"),
                            format!("observations={observations}\n{trace:#?}"),
                        )
                        .unwrap();
                    }
                }
            }
        }
    }
}

#[test]
fn direct_return_and_indirect_transfer_events_have_distinct_stack_phases() {
    use actionc::mir65816::image;
    let source = "CARD direct,indirect,choose=$7100 CARD FUNC POINTER cb(CARD value) CARD FUNC Echo(CARD value) RETURN(value) PROC Main() IF choose=0 THEN direct=Echo($8001) ELSE direct=Echo($9001) FI cb=@Echo indirect=cb(direct) RETURN";
    for optimize in [false, true] {
        let prepared = prepare(source, optimize);
        let (machine, traces) = proof::materialize_with_trace(&prepared.mir).unwrap();
        let image = image::link(&prepared.mir, &machine, &layout()).unwrap();
        assert_eq!(
            image.to_json().unwrap(),
            prepared
                .compile(&layout())
                .unwrap()
                .image
                .to_json()
                .unwrap()
        );
        let main = image
            .routines
            .iter()
            .find(|r| r.address == image.entry)
            .unwrap();
        let code = &machine
            .routines
            .iter()
            .find(|r| r.id.0 == main.id)
            .unwrap()
            .code;
        assert!(
            code.conditional_branches
                .iter()
                .any(|s| s.short && code.return_fixups.iter().any(|(at, _)| s.offset < *at))
        );
        let trace = &traces
            .iter()
            .find(|t| t.routine.0 == main.id)
            .unwrap()
            .snapshots;
        assert!(
            trace
                .iter()
                .any(|s| s.event == proof::Event::CallReturn && s.transfer_pushes == 0)
        );
        assert!(
            trace
                .iter()
                .any(|s| s.event == proof::Event::IndirectTransfer && s.transfer_pushes == 3)
        );
        assert_eq!(trace.iter().map(|s| s.transfer_pushes).max(), Some(6));
        let at: std::collections::BTreeMap<_, _> = trace
            .iter()
            .filter(|s| s.pc < main.size as usize && s.event != proof::Event::IndirectTransfer)
            .map(|s| (s.pc, s))
            .collect();
        for choose in [0u16, 1] {
            for irq in [0, 4] {
                let mut h = Harness::new(&image, &caller(image.entry), irq);
                h.bus.ram[0x7100..0x7102].copy_from_slice(&choose.to_le_bytes());
                for _ in 0..100_000 {
                    if h.cpu.is_stopped() {
                        break;
                    }
                    if h.cpu.is_instruction_boundary() {
                        if let Some(s) = h
                            .cpu
                            .pc()
                            .checked_sub(main.address)
                            .and_then(|pc| at.get(&(pc as usize)))
                        {
                            check(s, h.cpu.registers(), 0x5fec, &h.bus, irq);
                        }
                    }
                    h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                }
                assert!(h.cpu.is_stopped());
                h.guards(irq);
                assert_eq!(
                    h.global(&image, "direct", 2),
                    if choose == 0 { 0x8001 } else { 0x9001 }
                );
                assert_eq!(
                    h.global(&image, "indirect", 2),
                    if choose == 0 { 0x8001 } else { 0x9001 }
                );
            }
        }
    }
}
