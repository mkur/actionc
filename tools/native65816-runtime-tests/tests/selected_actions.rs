mod support;
use actionc::mir65816::{
    emit::{
        self,
        proof::{self, SelectedObservation},
    },
    image,
};
use actionc_vm::native65816::Inputs;
use std::collections::{BTreeMap, BTreeSet};
use support::*;

fn ordinal(nodes: &[SelectedObservation], site: proof::SelectedSite) -> usize {
    nodes.iter().position(|n| n.site == site).unwrap()
}
fn frontier(nodes: &[SelectedObservation], from: usize) -> BTreeSet<usize> {
    let mut pending: Vec<_> = nodes[from]
        .successors
        .iter()
        .map(|s| ordinal(nodes, *s))
        .collect();
    let mut seen = BTreeSet::new();
    let mut result = BTreeSet::new();
    while let Some(n) = pending.pop() {
        if !seen.insert(n) {
            continue;
        }
        if matches!(nodes[n].kind, "instruction" | "return-exit" | "fault-exit") {
            result.insert(n);
        } else {
            pending.extend(nodes[n].successors.iter().map(|s| ordinal(nodes, *s)));
        }
    }
    result
}

#[test]
fn production_records_cover_all_code_with_and_without_optional_tracing() {
    let mut summary = Vec::new();
    let mut request_kinds = BTreeSet::new();
    let mut fused = 0;
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
            let prepared = prepare(&fixture(&format!("code_quality/{case}.act")), optimize);
            let plain = emit::materialize(&prepared.mir).unwrap();
            let (traced, _) = proof::materialize_with_trace(&prepared.mir).unwrap();
            assert_eq!(
                image::link(&prepared.mir, &plain, &layout())
                    .unwrap()
                    .to_json()
                    .unwrap(),
                image::link(&prepared.mir, &traced, &layout())
                    .unwrap()
                    .to_json()
                    .unwrap()
            );
            let mut count = 0;
            let mut instructions = 0;
            let mut reachable = 0;
            for (a, b) in plain.routines.iter().zip(&traced.routines) {
                let nodes = proof::selected_actions(&a.code).unwrap();
                let other = proof::selected_actions(&b.code).unwrap();
                assert_eq!(nodes.len(), other.len());
                assert!(proof::instruction_effects(&a.code).is_empty());
                assert!(proof::selected_site(&b.code, nodes[0].site).is_err());
                let mut cursor = 0;
                for (n, m) in nodes.iter().zip(&other) {
                    assert_eq!(
                        (
                            n.kind,
                            n.request,
                            &n.encoded,
                            n.source,
                            n.fused_terminator,
                            n.control,
                            n.reachable
                        ),
                        (
                            m.kind,
                            m.request,
                            &m.encoded,
                            m.source,
                            m.fused_terminator,
                            m.control,
                            m.reachable
                        )
                    );
                    assert_eq!(
                        n.successors
                            .iter()
                            .map(|s| ordinal(&nodes, *s))
                            .collect::<Vec<_>>(),
                        m.successors
                            .iter()
                            .map(|s| ordinal(&other, *s))
                            .collect::<Vec<_>>()
                    );
                    if n.kind == "instruction" {
                        assert_eq!(n.encoded.start, cursor);
                        cursor = n.encoded.end;
                        instructions += 1;
                    } else {
                        assert!(n.encoded.is_empty());
                    }
                    if let Some(request) = n.request {
                        request_kinds.insert(request);
                    }
                    if n.fused_terminator.is_some() {
                        fused += 1;
                    }
                    if n.reachable {
                        reachable += 1;
                    }
                }
                assert_eq!(cursor, a.code.bytes.len());
                count += nodes.len();
            }
            summary.push(serde_json::json!({"case":case,"optimized":optimize,"sites":count,"instructions":instructions,"reachable_sites":reachable,"image_equal":true}));
        }
    }
    for request in [
        "mode",
        "body-anchor",
        "barrier",
        "home",
        "blocks",
        "entry-obligations",
        "consume-word",
        "remember-word",
        "capture-incoming",
        "x-obligations",
        "increment-x",
        "refresh-x",
        "compare-x",
        "fallthrough",
        "jump",
        "dispatch",
    ] {
        assert!(request_kinds.contains(request), "{request}");
    }
    assert!(fused > 0);
    if let Ok(dir) = std::env::var("A816_QUALIFICATION_DIR") {
        std::fs::write(std::path::Path::new(&dir).join("selected-actions-summary.json"),serde_json::to_string_pretty(&serde_json::json!({"builds":summary,"requests":request_kinds,"fused_sources":fused})).unwrap()).unwrap();
    }
}

#[test]
fn indirect_rtl_is_a_summary_edge_to_its_typed_continuation() {
    let source = "CARD direct,indirect CARD FUNC POINTER cb(CARD value) CARD FUNC Echo(CARD value) RETURN(value) PROC Main() direct=Echo($8001) cb=@Echo indirect=cb(direct) RETURN";
    for optimize in [false, true] {
        let p = prepare(source, optimize);
        let m = emit::materialize(&p.mir).unwrap();
        let mut calls = 0;
        for routine in &m.routines {
            let nodes = proof::selected_actions(&routine.code).unwrap();
            for n in &nodes {
                if n.control == Some(proof::EffectControl::Call { target: None }) {
                    calls += 1;
                    assert_eq!(n.successors.len(), 1);
                    let continuation = &nodes[ordinal(&nodes, n.successors[0])];
                    assert_eq!(continuation.kind, "bind-label");
                    assert_eq!(continuation.encoded.start, n.encoded.end);
                    assert_eq!(n.depth_before - n.depth_after, 6);
                    assert_eq!(continuation.depth_after, n.depth_after);
                    assert_eq!(routine.code.bytes[n.encoded.start], 0x6b);
                }
            }
        }
        assert_eq!(calls, 1);
    }
}

#[test]
fn selected_graph_predicts_actual_raw_and_optimized_loop_and_guard_paths() {
    let mut counts = Vec::new();
    for optimize in [false, true] {
        let p = prepare(&fixture("code_quality/sum_loop.act"), optimize);
        let m = emit::materialize(&p.mir).unwrap();
        let image = image::link(&p.mir, &m, &layout()).unwrap();
        let work = image
            .routines
            .iter()
            .find(|r| r.name.to_lowercase().contains("work"))
            .unwrap();
        let code = &m.routines.iter().find(|r| r.id.0 == work.id).unwrap().code;
        let nodes = proof::selected_actions(code).unwrap();
        let at: BTreeMap<_, _> = nodes
            .iter()
            .filter(|n| n.kind == "instruction")
            .map(|n| (work.address + n.encoded.start as u32, n.ordinal))
            .collect();
        let next: BTreeMap<_, _> = nodes
            .iter()
            .filter(|n| n.kind == "instruction" || n.kind == "entry")
            .map(|n| (n.ordinal, frontier(&nodes, n.ordinal)))
            .collect();
        for n in [0u16, 1, 13] {
            for irq in [0, 4] {
                for fail_guard in [false, true] {
                    let caller = assemble(
                        &format!(
                            "tsc\nsec\nsbc #3\ntcs\nlda #{n}\nsta 1,s\njsl ${:06x}\nsta $7000\ntsc\nclc\nadc #3\ntcs\nstp\nnop",
                            work.address
                        ),
                        0x40000,
                    );
                    let mut h = Harness::new(&image, &caller, irq);
                    if fail_guard {
                        h.bus.ram[0x2044..0x2046].copy_from_slice(&0x5fefu16.to_le_bytes());
                    }
                    let mut previous = 0;
                    let mut observations = 0;
                    let mut ended = false;
                    for _ in 0..100_000 {
                        if h.cpu.is_stopped() {
                            break;
                        }
                        if h.cpu.is_instruction_boundary() {
                            if let Some(&current) = at.get(&h.cpu.pc()) {
                                assert!(
                                    next[&previous].contains(&current),
                                    "{optimize}/{n}/{irq}: {} -> {current}",
                                    previous
                                );
                                previous = current;
                                observations += 1;
                            } else if observations > 0
                                && !(work.address..work.address + code.bytes.len() as u32)
                                    .contains(&h.cpu.pc())
                                && !ended
                            {
                                let expected = if fail_guard {
                                    "fault-exit"
                                } else {
                                    "return-exit"
                                };
                                assert!(next[&previous].iter().any(|&n| nodes[n].kind == expected));
                                ended = true;
                            }
                        }
                        h.cpu.tick(&mut h.bus, Inputs::default()).unwrap();
                    }
                    assert!(h.cpu.is_stopped() && ended && observations > 0);
                    if !fail_guard {
                        h.guards(irq);
                        assert_eq!(
                            h.bus.value(0x7000, 2),
                            u32::from(n) * (u32::from(n) + 1) / 2
                        );
                    }
                    counts.push(serde_json::json!({"optimized":optimize,"n":n,"i":irq,"guard_failure":fail_guard,"checked_transitions":observations,"exit_checked":ended}));
                }
            }
        }
    }
    if let Ok(dir) = std::env::var("A816_QUALIFICATION_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join("selected-cfg-execution.json"),
            serde_json::to_string_pretty(&counts).unwrap(),
        )
        .unwrap();
    }
}
