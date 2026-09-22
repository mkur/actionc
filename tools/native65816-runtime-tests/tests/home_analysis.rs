mod support;
use actionc::mir65816::emit::{
    self,
    proof::{self, EffectAccess, HomeByte, HomeOwner},
};
use support::*;

#[test]
fn canonical_homes_and_liveness_cover_raw_and_optimized_selection() {
    let mut summary = Vec::new();
    let mut reused = 0;
    let mut indirect = 0;
    let mut calls = 0;
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
            let machine = emit::materialize(&prepared.mir).unwrap();
            let other = emit::materialize(&prepared.mir).unwrap();
            let mut reachable = 0;
            let mut uncertain = 0;
            let mut bytes = 0;
            for (routine, foreign) in machine.routines.iter().zip(&other.routines) {
                let nodes = proof::selected_actions(&routine.code).unwrap();
                let analysis = proof::home_analysis(&routine.code).unwrap();
                let foreign_site = proof::selected_actions(&foreign.code).unwrap()[0].site;
                assert!(analysis.home_live_before(foreign_site).is_err());
                assert!(analysis.accesses(foreign_site).is_err());
                bytes += analysis.homes().len();
                reused += analysis
                    .homes()
                    .values()
                    .filter(|h| {
                        h.owners
                            .iter()
                            .filter(|o| matches!(o, HomeOwner::Temporary(_)))
                            .count()
                            > 1
                    })
                    .count();
                for (home, info) in analysis.homes() {
                    if matches!(home, HomeByte::DirectPage(64..=255)) {
                        assert!(!info.private && info.entry_defined);
                    }
                    if info.owners.contains(&HomeOwner::ReturnAddress) {
                        assert!(!info.private && info.entry_defined);
                    }
                }
                for node in nodes {
                    if !node.reachable {
                        assert!(analysis.home_live_before(node.site).is_err());
                        assert!(analysis.home_live_after(node.site).is_err());
                        continue;
                    }
                    reachable += 1;
                    analysis.home_live_before(node.site).unwrap();
                    analysis.home_live_after(node.site).unwrap();
                    let accesses = analysis.accesses(node.site).unwrap();
                    for a in accesses {
                        assert!(a.homes.iter().all(|h| analysis.homes().contains_key(h)));
                        if a.uncertain {
                            uncertain += 1;
                            assert_ne!(a.access, EffectAccess::Write);
                            assert_eq!(a.homes.len(), analysis.homes().len());
                        }
                    }
                    if node.kind == "instruction" {
                        // Independent machine encodings: indirect long load/store
                        // reads all three pointer bytes before accessing its target.
                        let encoded = &routine.code.bytes[node.encoded.clone()];
                        if matches!(encoded[0], 0xa7 | 0xb7 | 0x87 | 0x97) {
                            indirect += 1;
                            let pointer = u16::from(encoded[1]);
                            assert_eq!(accesses[0].access, EffectAccess::Read);
                            assert_eq!(
                                accesses[0].homes,
                                (pointer..pointer + 3).map(HomeByte::DirectPage).collect()
                            );
                            assert!(accesses[1].uncertain);
                        }
                    }
                    if matches!(node.control, Some(proof::EffectControl::Call { .. })) {
                        calls += 1;
                        let clobber = accesses
                            .iter()
                            .position(|a| a.access == EffectAccess::MayWrite)
                            .unwrap();
                        assert!(
                            accesses[..clobber]
                                .iter()
                                .any(|a| a.access == EffectAccess::Read)
                        );
                    }
                }
            }
            summary.push(serde_json::json!({"case":case,"optimized":optimize,"reachable_sites":reachable,"home_bytes":bytes,"uncertain_accesses":uncertain}));
        }
    }
    assert!(reused > 0 && indirect > 0 && calls > 0);
    if let Ok(dir) = std::env::var("A816_QUALIFICATION_DIR") {
        std::fs::write(std::path::Path::new(&dir).join("home-analysis-summary.json"),serde_json::to_string_pretty(&serde_json::json!({"builds":summary,"reused_bytes":reused,"indirect_accesses":indirect,"calls":calls})).unwrap()).unwrap();
    }
}
