//! Observe sealed plans after final rewrites, without changing baseline inventory.
use actionc::mir65816::emit::{MachineProgram, proof};
use serde_json::json;
use std::{fs, path::Path};

pub(super) fn report(machine: &MachineProgram, output: &Path) {
    let mut rows = vec![];
    let mut opaque = vec![];
    for r in &machine.routines {
        let logical = machine
            .prepared
            .routines
            .iter()
            .find(|p| p.id == r.id)
            .unwrap();
        let Some(s) = proof::placement_summary(&r.code).unwrap() else {
            opaque.push(json!({"id":r.id.0,"name":logical.name,"kind":if logical.helper.is_some() {"helper"} else {"forwarding"}}));
            continue;
        };
        rows.push(json!({"id":r.id.0,"name":logical.name,"values":s.values,"materialized":s.materialized,
            "borrowed":s.borrowed,"register_intervals":s.register_intervals,"component_intervals":s.component_intervals,
            "redirected_locals":s.redirected_locals,"deferred_assignments":s.deferred_assignments,
            "mixed_homes":s.mixed_homes,"backed_residences":s.backed_residences,
            "windows":s.windows,"record_windows":s.record_windows,"scalar_windows":s.scalar_windows,
            "address_windows":s.address_windows,"barriers":s.barriers,"boundaries":s.boundaries,
            "edges":s.edges,"transfers":s.transfers,"staging_bytes":s.staging_bytes,"x_mirror":s.x_mirror}));
    }
    fs::write(
        output.with_extension("placement.json"),
        serde_json::to_vec(&json!({"routines":rows,"opaque_routines":opaque})).unwrap(),
    )
    .unwrap();
}
