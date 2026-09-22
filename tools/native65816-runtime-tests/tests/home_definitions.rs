mod support;
use actionc::mir65816::emit::{
    self,
    proof::{self, EffectAccess, HomeByte},
};
use support::*;

#[test]
fn stored_definition_queries_cover_actual_raw_and_optimized_code() {
    let mut summary = Vec::new();
    let mut total_stores = 0;
    let mut total_uses = 0;
    let mut total_blocked = 0;
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
            let foreign = emit::materialize(&prepared.mir).unwrap();
            let mut stores = 0;
            let mut uses = 0;
            let mut blocked = 0;
            let mut outside_dead = 0;
            let mut undefined_exact = 0;
            let mut undefined_uncertain = 0;
            for (routine, other) in machine.routines.iter().zip(&foreign.routines) {
                let nodes = proof::selected_actions(&routine.code).unwrap();
                let analysis = proof::home_analysis(&routine.code).unwrap();
                let foreign_site = proof::selected_actions(&other.code).unwrap()[0].site;
                assert!(
                    analysis
                        .uses_of_definition(HomeByte::DirectPage(0), foreign_site)
                        .is_err()
                );
                assert!(
                    analysis
                        .uses_of_definition(HomeByte::DirectPage(0), nodes[0].site)
                        .is_err()
                );
                for node in &nodes {
                    if !node.reachable {
                        assert!(
                            analysis
                                .uses_of_definition(HomeByte::DirectPage(0), node.site)
                                .is_err()
                        );
                        continue;
                    }
                    for access in analysis.accesses(node.site).unwrap() {
                        if access.access != EffectAccess::Write {
                            continue;
                        }
                        for &home in &access.homes {
                            stores += 1;
                            assert!(
                                analysis
                                    .definition_dead_outside_window(home, node.site, foreign_site)
                                    .is_err()
                            );
                            assert!(
                                analysis
                                    .uses_of_definition(HomeByte::Stack(-99999), node.site)
                                    .is_err()
                            );
                            let reads = analysis.uses_of_definition(home, node.site).unwrap();
                            uses += reads.len();
                            for usage in reads {
                                let read =
                                    &analysis.accesses(usage.site).unwrap()[usage.access_index];
                                assert_eq!(read.access, EffectAccess::Read);
                                assert!(read.homes.contains(&home));
                            }
                            match analysis
                                .definition_dead_outside_window(home, node.site, node.site)
                            {
                                Ok(true) => outside_dead += 1,
                                Ok(false) => {}
                                Err(_) => blocked += 1,
                            }
                        }
                    }
                }
                for read in analysis.undefined_private_reads().unwrap() {
                    assert!(analysis.homes()[&read.home].private);
                    let access =
                        &analysis.accesses(read.usage.site).unwrap()[read.usage.access_index];
                    assert_eq!(access.access, EffectAccess::Read);
                    assert!(access.homes.contains(&read.home));
                    if read.usage.uncertain {
                        undefined_uncertain += 1;
                    } else {
                        undefined_exact += 1;
                    }
                }
            }
            total_stores += stores;
            total_uses += uses;
            total_blocked += blocked;
            summary.push(serde_json::json!({"case":case,"optimized":optimize,"stored_byte_definitions":stores,"attributed_reads":uses,"blocked_singleton_windows":blocked,"no_reads_outside_singleton":outside_dead,"possibly_undefined_exact_reads":undefined_exact,"possibly_undefined_uncertain_reads":undefined_uncertain}));
        }
    }
    assert!(total_stores > 0 && total_uses > 0 && total_blocked > 0);
    if let Ok(dir) = std::env::var("A816_QUALIFICATION_DIR") {
        std::fs::write(
            std::path::Path::new(&dir).join("home-definitions-summary.json"),
            serde_json::to_string_pretty(&summary).unwrap(),
        )
        .unwrap();
    }
}
