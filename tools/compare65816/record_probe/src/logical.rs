//! Checked facts for every routine of the frozen workload. This extra observer
//! runs after compilation and is excluded from paired CLI overhead measurements.
use actionc::mir65816::{analysis::*, *};
use serde_json::json;
use std::{fs, path::Path, time::Instant};

pub fn report(program: &Mir65816Program, out: &Path) {
    let mut routines = vec![];
    let mut opaque = vec![];
    for r in &program.routines {
        if r.helper.is_some() || r.entry.external {
            opaque.push(json!({"id":r.id.0,"name":r.name,"helper":r.helper.is_some(),"external":r.entry.external}));
            continue;
        }
        let start = Instant::now();
        let a = RoutineAnalysis::new(r).unwrap();
        let seconds = start.elapsed().as_secs_f64();
        let mut reachable = 0;
        let mut cyclic = 0;
        let mut edges = 0;
        let mut edge_args = 0;
        let mut max_live = 0;
        for b in &r.blocks {
            let block = a.block(b.id).unwrap();
            match a.dominates(block, block) {
                Err(QueryError::Unreachable) => continue,
                Ok(true) => (),
                other => panic!("bad self-dominance: {other:?}"),
            }
            reachable += 1;
            cyclic += usize::from(a.is_cyclic(block).unwrap());
            for index in 0..=b.ops.len() {
                max_live = max_live.max(a.live_at(a.point(block, index).unwrap()).unwrap().len());
            }
            for ordinal in 0..2 {
                let Ok(e) = a.edge(block, ordinal) else {
                    continue;
                };
                let m = a.edge_mapping(e).unwrap();
                let target = r.blocks.iter().find(|b| b.id == m.target.id()).unwrap();
                assert_eq!(m.bindings.len(), target.params.len());
                for ((temp, _), (expected, _)) in m.bindings.iter().zip(&target.params) {
                    assert_eq!(temp.id(), *expected);
                }
                edges += 1;
                edge_args += m.bindings.len();
            }
        }
        let mut identities = std::collections::BTreeSet::new();
        let mut uses = 0;
        let mut checked_uses = 0;
        for (id, _) in &r.temps {
            let t = a.temp(*id).unwrap();
            let f = a.value(t).unwrap();
            identities.insert(f.identity.id());
            uses += f.uses.len();
            for u in &f.uses {
                let p = a
                    .point(a.block(u.point.block).unwrap(), u.point.index)
                    .unwrap();
                match a.available(p, t) {
                    Ok(true) => checked_uses += 1,
                    Err(QueryError::Unreachable) => (),
                    other => panic!("bad availability: {other:?}"),
                }
            }
        }
        let storages: Vec<_> = r
            .frame
            .objects
            .iter()
            .map(|o| StorageId::Frame(o.id))
            .chain(
                r.frame
                    .parameters
                    .iter()
                    .filter(|p| p.frame_object.is_none())
                    .map(|p| StorageId::Input(p.param)),
            )
            .collect();
        let mut storage_rows = vec![];
        for id in storages {
            let s = a.storage(id).unwrap();
            let f = a.storage_facts(s).unwrap();
            let entry = a.point(a.block(r.blocks[0].id).unwrap(), 0).unwrap();
            let contents = a.storage_at(entry, s, 0, f.size.get()).unwrap();
            storage_rows.push(json!({"id":format!("{id:?}"),"bytes":f.size.get(),"ownership":format!("{:?}",f.ownership),
                "initialized_entry_bytes":contents.bytes.iter().filter(|b|b.definitely_initialized).count(),
                "known_entry_versions":contents.bytes.iter().filter(|b|b.version.is_some()).count()}));
        }
        let captures = r
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .filter(|op| matches!(op, Mir65816Op::Load { .. }))
            .count();
        routines.push(json!({"id":r.id.0,"name":r.name,"analysis_seconds":seconds,"values":r.temps.len(),"identities":identities.len(),
            "uses":uses,"checked_reachable_uses":checked_uses,"captures":captures,"blocks":r.blocks.len(),"reachable_blocks":reachable,
            "cyclic_blocks":cyclic,"edges":edges,"edge_arguments":edge_args,"max_live_values":max_live,
            "solver_evaluations":a.solver_evaluations(),"storage":storage_rows}));
    }
    fs::write(
        out.with_extension("analysis.json"),
        serde_json::to_vec(&json!({"routines":routines,"opaque_routines":opaque})).unwrap(),
    )
    .unwrap();
}
