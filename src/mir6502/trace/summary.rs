//! A diagnostic index, not an optimizer provenance or deadness analysis.
use super::{Mir6502Trace, TraceEvent};
use serde_json::{Value, json};

impl Mir6502Trace {
    /// Borrow records without duplicating the bounded journal (for artifact writers).
    pub fn with_events<T>(&self, f: impl FnOnce(&[TraceEvent]) -> T) -> T {
        f(&self.inner.state.borrow().events)
    }

    /// Final accesses and physical placement, with references into the attempt journal.
    /// History references retain attempt IDs: equal home IDs in different attempts
    /// do not imply equal values or selection. Remaps are the only exact home links.
    pub fn home_summary(&self) -> Value {
        self.with_events(|events| {
            let final_snapshot = events.iter().rev().find(|e|e.kind == "snapshot" && e.data["stage"] == "pre-emission");
            let Some(final_snapshot) = final_snapshot else { return json!({"status":"no-final-snapshot"}); };
            let mut routines = Vec::new();
            for accesses in final_snapshot.data["accesses"].as_array().into_iter().flatten() {
                let routine = &accesses["routine"];
                let mut homes = Vec::new();
                for access in accesses["homes"].as_array().into_iter().flatten() {
                    let home = &access["home"];
                    let placements = events.iter().filter(|e|e.kind == "final-home" && &e.data["routine"] == routine && &e.data["home"] == home).map(|e|json!({"event":e.id,"address":e.data["address"],"bytes":e.data["bytes"]})).collect::<Vec<_>>();
                    let empty_reads = access["counts"]["read_points"].as_array().into_iter().flatten().filter_map(|p| {
                        events.iter().find(|e|e.kind == "emitted-range" && &e.data["routine"] == routine && e.data["block"]==p["block"] && e.data["op"]==p["op"] && e.data["offset_start"]==e.data["offset_end"]).map(|e|e.id)
                    }).collect::<Vec<_>>();
                    let history = events.iter().filter(|e| &e.data["routine"] == routine && (
                        (matches!(e.kind,"home-assignment"|"home-fate"|"compat-spill-stop"|"compat-spill-outcome"|"home-definition-reader") && &e.data["home"]==home)
                        || (e.kind == "home-assignment" && &e.data["planned_home"] == home)
                        || (e.kind == "rewrite-blocked" && &e.data["details"]["home"]==home)
                    )).map(|e|json!({"event":e.id,"attempt":e.attempt,"kind":e.kind})).collect::<Vec<_>>();
                    homes.push(json!({"home":home,"final_accesses":access["counts"],"placements":placements,"zero_byte_read_events":empty_reads,"history_references":history}));
                }
                let remaps = events.iter().filter(|e| &e.data["routine"] == routine && matches!(e.kind,"home-remap"|"home-plan"|"home-prune"|"abi-home" )).map(|e|json!({"event":e.id,"attempt":e.attempt,"kind":e.kind})).collect::<Vec<_>>();
                routines.push(json!({"routine":routine,"homes":homes,"demand_and_remap_references":remaps,"non_explicit_observations":accesses["non_explicit_observations"]}));
            }
            json!({"final_snapshot":final_snapshot.id,"routines":routines,
                "interpretation":"Home identity is not value identity. History references include speculative attempts; follow selection records and remaps. Consumer changes without an operation map are snapshot comparisons, not persistent operation identity. Zero explicit reads or zero emitted bytes do not prove deadness."})
        })
    }
}
