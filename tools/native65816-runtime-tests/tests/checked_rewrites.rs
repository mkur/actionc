mod support;
use actionc::mir65816::emit::{self, proof};
use std::collections::BTreeMap;
use support::*;

#[test]
fn every_planned_load_agrees_with_the_reference_and_preserves_output() {
    let mut inventory = Vec::new();
    let mut classified = 0;
    let mut accepted = 0;
    let mut blockers = BTreeMap::new();
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
            let (reference, old) = proof::materialize_reference(&p.mir, true).unwrap();
            let (checked, new) = proof::materialize_with_trace(&p.mir).unwrap();
            let plain = emit::materialize(&p.mir).unwrap();
            let mut observations = Vec::new();
            for (((a, b), c), (old, new)) in reference
                .routines
                .iter()
                .zip(&checked.routines)
                .zip(&plain.routines)
                .zip(old.iter().zip(&new))
            {
                proof::compare_replay_output(&a.code, &b.code).unwrap();
                assert_eq!(a.code.bytes, c.code.bytes);
                assert_eq!(old.snapshots, new.snapshots);
                assert_eq!(
                    proof::rewrite_observations(&b.code),
                    proof::rewrite_observations(&c.code)
                );
                for o in proof::rewrite_observations(&b.code) {
                    classified += 1;
                    accepted += usize::from(o.accepted);
                    if !o.accepted {
                        *blockers.entry(o.reason.clone()).or_insert(0usize) += 1;
                    }
                    observations.push(serde_json::json!({"routine":b.id.0,"request":o.request_ordinal,"accepted":o.accepted,"reason":o.reason,"original_load":o.original_load,"read_bytes_removed":o.private_read_bytes_removed}));
                }
            }
            inventory.push(serde_json::json!({"case":case,"optimized":optimize,"output_and_trace_equal":true,"candidates":observations}));
        }
    }
    assert!(accepted > 0 && classified > accepted);
    if let Ok(dir) = std::env::var("A816_QUALIFICATION_DIR") {
        std::fs::write(std::path::Path::new(&dir).join("checked-rewrites-summary.json"),serde_json::to_string_pretty(&serde_json::json!({"builds":inventory,"classified":classified,"accepted":accepted,"blocked":classified-accepted,"blockers":blockers})).unwrap()).unwrap();
    }
}

#[test]
fn forwarding_alias_width_call_and_preemption_fixtures_keep_exact_decisions() {
    for name in [
        "accumulator_forwarding.act",
        "frame_forwarding.act",
        "parameter_forwarding.act",
        "preemption.act",
        "pointer_preemption.act",
    ] {
        for optimize in [false, true] {
            let p = prepare(&fixture(name), optimize);
            let (a, old) = proof::materialize_reference(&p.mir, true).unwrap();
            let (b, new) = proof::materialize_with_trace(&p.mir).unwrap();
            for ((a, b), (old, new)) in a.routines.iter().zip(&b.routines).zip(old.iter().zip(&new))
            {
                proof::compare_replay_output(&a.code, &b.code).unwrap();
                assert_eq!(old.snapshots, new.snapshots);
            }
        }
    }
}
