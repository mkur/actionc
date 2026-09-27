mod support;
use actionc::mir65816::{
    emit::{self, proof},
    image,
};
use std::collections::BTreeSet;
use support::*;

#[test]
fn replay_matches_direct_code_traces_and_repeated_finalization() {
    let mut inventory = Vec::new();
    let mut requests = BTreeSet::new();
    let mut decisions = [0usize; 2];
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
            let plain = emit::materialize(&p.mir).unwrap();
            for trace in [false, true] {
                let (direct, old_traces) = proof::materialize_reference(&p.mir, trace).unwrap();
                let (replayed, new_traces) = proof::materialize_replayed(&p.mir, trace).unwrap();
                let direct_image = image::link(&p.mir, &direct, &layout())
                    .unwrap()
                    .to_json()
                    .unwrap();
                assert_eq!(
                    direct_image,
                    image::link(&p.mir, &replayed, &layout())
                        .unwrap()
                        .to_json()
                        .unwrap()
                );
                assert_eq!(
                    direct_image,
                    image::link(&p.mir, &plain, &layout())
                        .unwrap()
                        .to_json()
                        .unwrap()
                );
                assert_eq!(direct.routines.len(), replayed.routines.len());
                assert_eq!(old_traces.len(), new_traces.len());
                let mut sites = 0;
                let mut snapshots = 0;
                for ((a, b), (old, new)) in direct
                    .routines
                    .iter()
                    .zip(&replayed.routines)
                    .zip(old_traces.iter().zip(&new_traces))
                {
                    proof::compare_replay_output(&a.code, &b.code).unwrap();
                    assert_eq!(old.routine, new.routine);
                    assert_eq!(old.snapshots, new.snapshots);
                    snapshots += new.snapshots.len();
                    let (again, observed) = proof::replay_code(&b.code, trace).unwrap();
                    proof::compare_replay_output(&b.code, &again).unwrap();
                    assert_eq!(observed, new.snapshots);
                    let nodes = proof::selected_actions(&b.code).unwrap();
                    sites += nodes.len();
                    for n in nodes {
                        let next = proof::selected_site(&again, n.site).unwrap();
                        assert_eq!(next.encoded, n.encoded);
                        assert_eq!(next.successors, n.successors);
                        assert_eq!(next.decision, n.decision);
                        if let Some(q) = n.request {
                            requests.insert(q);
                        }
                        if let Some(d) = n.decision {
                            decisions[usize::from(d)] += 1;
                        }
                    }
                }
                inventory.push(serde_json::json!({"case":case,"optimized":optimize,"trace":trace,"sites":sites,"snapshots":snapshots,"direct_replay_repeat_equal":true,"image_equal":true}));
            }
        }
    }
    assert_eq!(requests.len(), 21);
    assert!(requests.contains("prepare-return-join"));
    assert!(decisions[0] > 0 && decisions[1] > 0);
    if let Ok(dir) = std::env::var("A816_QUALIFICATION_DIR") {
        std::fs::write(std::path::Path::new(&dir).join("replay-summary.json"),serde_json::to_string_pretty(&serde_json::json!({"builds":inventory,"requests":requests,"consume_decisions_false_true":decisions})).unwrap()).unwrap();
    }
}

#[test]
fn indirect_calls_keep_return_fixups_and_untrusted_bytes_cannot_bypass_replay() {
    let source = "CARD direct,indirect CARD FUNC POINTER cb(CARD value) CARD FUNC Echo(CARD value) RETURN(value) PROC Main() direct=Echo($8001) cb=@Echo indirect=cb(direct) RETURN";
    for optimize in [false, true] {
        let p = prepare(source, optimize);
        let (a, old) = proof::materialize_reference(&p.mir, true).unwrap();
        let (b, new) = proof::materialize_replayed(&p.mir, true).unwrap();
        let mut per = 0;
        for ((a, b), (old, new)) in a.routines.iter().zip(&b.routines).zip(old.iter().zip(&new)) {
            proof::compare_replay_output(&a.code, &b.code).unwrap();
            assert_eq!(old.snapshots, new.snapshots);
            per += b.code.return_fixups.len();
            let mut changed = b.code.clone();
            changed.bytes[0] ^= 1;
            assert!(proof::replay_code(&changed, true).is_err());
        }
        assert_eq!(per, 1);
    }
}
