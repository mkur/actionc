mod support;
use actionc::mir65816::{
    emit::{
        self,
        proof::{self, ConditionFlag, RegisterLane},
    },
    image,
};
use actionc_vm::native65816::{Inputs, Machine, Registers};
use support::*;

#[test]
fn machine_queries_cover_all_raw_and_optimized_selected_sites() {
    let mut summary = Vec::new();
    for case in [
        "identity",
        "add",
        "subtract",
        "constant_chain",
        "maximum",
        "wide_shift",
        "loop_rotation",
        "sum_loop",
        "recursive_sum",
        "direct_calls",
        "byte_sum",
        "record_field",
        "unlink",
        "forward_copy",
    ] {
        for optimize in [false, true] {
            let p = prepare(&fixture(&format!("code_quality/{case}.act")), optimize);
            let machine = emit::materialize(&p.mir).unwrap();
            let other = emit::materialize(&p.mir).unwrap();
            let mut counts = [0usize; 11];
            for (r, s) in machine.routines.iter().zip(&other.routines) {
                let analysis = proof::home_analysis(&r.code).unwrap();
                let foreign = proof::selected_actions(&s.code).unwrap()[0].site;
                assert!(analysis.machine_live_before(foreign).is_err());
                assert!(analysis.machine_live_after(foreign).is_err());
                for node in proof::selected_actions(&r.code).unwrap() {
                    if !node.reachable {
                        assert!(analysis.machine_live_before(node.site).is_err());
                        assert!(analysis.machine_live_after(node.site).is_err());
                        continue;
                    }
                    let live = analysis.machine_live_before(node.site).unwrap();
                    analysis.machine_live_after(node.site).unwrap();
                    counts[0] += 1;
                    for (i, lane) in [
                        RegisterLane::ALow,
                        RegisterLane::AHigh,
                        RegisterLane::XLow,
                        RegisterLane::XHigh,
                        RegisterLane::YLow,
                        RegisterLane::YHigh,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        counts[i + 1] += usize::from(live.register_live(lane));
                    }
                    for (i, flag) in [
                        ConditionFlag::N,
                        ConditionFlag::Z,
                        ConditionFlag::C,
                        ConditionFlag::V,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        counts[i + 7] += usize::from(live.flag_live(flag));
                    }
                }
            }
            summary.push(serde_json::json!({"case":case,"optimized":optimize,"reachable_then_live_al_ah_xl_xh_yl_yh_n_z_c_v":counts}));
        }
    }
    if let Ok(dir) = std::env::var("A816_QUALIFICATION_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join("machine-liveness-summary.json"),
            serde_json::to_string_pretty(&summary).unwrap(),
        )
        .unwrap();
    }
}

fn resume(bus: &Bus, registers: Registers) -> (u32, Vec<(u32, u8)>, Vec<u32>) {
    let mut bus = bus.clone();
    bus.writes.clear();
    bus.reads.clear();
    let mut cpu = Machine::start_at(registers);
    assert!(
        cpu.run_until(&mut bus, 100_000, |_| Inputs::default(), |c| c.is_stopped())
            .unwrap()
    );
    (bus.value(0x7000, 2), bus.writes, bus.reads)
}

#[test]
fn independent_vm_perturbations_observe_dead_and_live_lanes_and_flags() {
    let mut observations = Vec::new();
    for optimize in [false, true] {
        for irq in [0, 4] {
            for equality in [false, true] {
                let source = if equality {
                    "CARD FUNC Work(CARD value) IF value=5 THEN RETURN(11) FI RETURN(22) PROC Main() RETURN"
                } else {
                    "CARD FUNC Work(CARD value) RETURN(value+1) PROC Main() RETURN"
                };
                let p = prepare(source, optimize);
                let m = emit::materialize(&p.mir).unwrap();
                let image = image::link(&p.mir, &m, &layout()).unwrap();
                let work = image
                    .routines
                    .iter()
                    .find(|r| r.name.to_lowercase().contains("work"))
                    .unwrap();
                let code = &m.routines.iter().find(|r| r.id.0 == work.id).unwrap().code;
                let analysis = proof::home_analysis(code).unwrap();
                let nodes = proof::selected_actions(code).unwrap();
                let caller = assemble(
                    &format!(
                        "tsc\nsec\nsbc #3\ntcs\nlda #5\nsta 1,s\njsl ${:06x}\nsta $7000\ntsc\nclc\nadc #3\ntcs\nstp\nnop",
                        work.address
                    ),
                    0x40000,
                );
                let mut h = Harness::new(&image, &caller, irq);
                let mut dead_checked = false;
                let mut live_checked = false;
                for _ in 0..100_000 {
                    if h.cpu.is_stopped() {
                        break;
                    }
                    if h.cpu.is_instruction_boundary() {
                        if let Some(node) = nodes.iter().find(|n| {
                            n.kind == "instruction"
                                && n.source.is_some()
                                && work.address + n.encoded.start as u32 == h.cpu.pc()
                        }) {
                            let live = analysis.machine_live_before(node.site).unwrap();
                            let original = h.cpu.registers();
                            if !equality
                                && !dead_checked
                                && live.registers == proof::EffectRegisters::default()
                                && live.flags == 0
                            {
                                let expected = resume(&h.bus, original);
                                for part in 0..10 {
                                    let mut r = original;
                                    match part {
                                        0 => r.a ^= 1,
                                        1 => r.a ^= 0x100,
                                        2 => r.x ^= 1,
                                        3 => r.x ^= 0x100,
                                        4 => r.y ^= 1,
                                        5 => r.y ^= 0x100,
                                        6 => r.p ^= 0x80,
                                        7 => r.p ^= 2,
                                        8 => r.p ^= 1,
                                        9 => r.p ^= 0x40,
                                        _ => unreachable!(),
                                    }
                                    assert_eq!(
                                        resume(&h.bus, r),
                                        expected,
                                        "dead part {part}, optimize={optimize}, irq={irq}"
                                    );
                                }
                                dead_checked = true;
                            }
                            let opcode = code.bytes[node.encoded.start];
                            if !live_checked
                                && ((!equality && matches!(opcode, 0x69 | 0x63 | 0x65))
                                    || (equality && matches!(opcode, 0xd0 | 0xf0)))
                            {
                                let expected = resume(&h.bus, original);
                                let mut r = original;
                                if equality {
                                    assert!(live.flag_live(ConditionFlag::Z));
                                    r.p ^= 2;
                                } else {
                                    assert!(live.flag_live(ConditionFlag::C));
                                    r.p ^= 1;
                                }
                                assert_ne!(
                                    resume(&h.bus, r).0,
                                    expected.0,
                                    "live branch/carry mutation must change result"
                                );
                                if !equality {
                                    for mask in [1, 0x100] {
                                        let mut r = original;
                                        r.a ^= mask;
                                        assert!(live.register_live(if mask == 1 {
                                            RegisterLane::ALow
                                        } else {
                                            RegisterLane::AHigh
                                        }));
                                        assert_ne!(resume(&h.bus, r).0, expected.0);
                                    }
                                }
                                live_checked = true;
                            }
                        }
                    }
                    h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                }
                assert!(h.cpu.is_stopped() && live_checked && (equality || dead_checked));
                h.guards(irq);
                assert_eq!(h.bus.value(0x7000, 2), if equality { 11 } else { 6 });
                observations.push(serde_json::json!({"optimized":optimize,"i":irq,"equality":equality,"dead_perturbations":if equality{0}else{10},"live_perturbations":if equality{1}else{3},"memory_events_compared":!equality}));
            }
        }
    }
    if let Ok(dir) = std::env::var("A816_QUALIFICATION_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join("machine-liveness-perturbations.json"),
            serde_json::to_string_pretty(&observations).unwrap(),
        )
        .unwrap();
    }
}
