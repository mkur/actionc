use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

const INVENTORY: &str = "surveys/tn/mir6502-rewrite-migration-inventory.tsv";

/// Scheduled entry points that may delete or retarget definitions. Keeping the
/// list explicit makes adding a new production family a reviewable audit
/// decision rather than silently accepting a raw block-local helper.
const ANALYZED_PRODUCER_REMOVERS: &[(&str, &str)] = &[
    (
        "src/mir6502/rewrite/pilots.rs",
        "discover_compare_producers",
    ),
    (
        "src/mir6502/rewrite/pilots.rs",
        "discover_compare_narrowing",
    ),
    (
        "src/mir6502/rewrite/pilots.rs",
        "discover_byte_binary_compare_consumers",
    ),
    (
        "src/mir6502/rewrite/pilots.rs",
        "discover_call_arg_producers",
    ),
    (
        "src/mir6502/rewrite/pilots.rs",
        "discover_return_slot_call_arg_forwards",
    ),
    ("src/mir6502/rewrite/pilots.rs", "discover_call_arg_exprs"),
    (
        "src/mir6502/rewrite/pilots.rs",
        "discover_call_result_store_consumers",
    ),
    ("src/mir6502/rewrite/pilots.rs", "discover_store_consumers"),
    ("src/mir6502/rewrite/pilots.rs", "discover_pointer_rewrites"),
    ("src/mir6502/rewrite/pilots.rs", "discover_index_rewrites"),
    ("src/mir6502/rewrite/pilots.rs", "discover_unused_lea_addrs"),
    (
        "src/mir6502/materialize/calls.rs",
        "discover_param_home_consumers",
    ),
    (
        "src/mir6502/materialize/calls.rs",
        "discover_param_home_reloads",
    ),
    (
        "src/mir6502/materialize/peepholes.rs",
        "discover_staged_word_forwards",
    ),
    (
        "src/mir6502/materialize/peepholes.rs",
        "discover_rhs_and_adjacent_reloads",
    ),
    (
        "src/mir6502/materialize/peepholes.rs",
        "discover_dead_private_scratch_stores",
    ),
    (
        "src/mir6502/materialize/peepholes.rs",
        "discover_indirect_constant_stores",
    ),
    (
        "src/mir6502/materialize/peepholes.rs",
        "discover_indirect_stores_and_compounds",
    ),
    (
        "src/mir6502/materialize/peepholes.rs",
        "discover_word_array_value_staging",
    ),
    (
        "src/mir6502/materialize/peepholes.rs",
        "discover_dead_register_writes",
    ),
    (
        "src/mir6502/materialize/indexes.rs",
        "discover_indexed_base_pointer_staging",
    ),
    (
        "src/mir6502/materialize/spills.rs",
        "discover_spill_forwards",
    ),
    (
        "src/mir6502/materialize/dead_spills.rs",
        "remove_dead_spill_stores",
    ),
    (
        "src/mir6502/materialize/store_consumers.rs",
        "discover_direct_inc_dec_updates",
    ),
    (
        "src/mir6502/materialize/ssa_lite.rs",
        "discover_ssa_lite_byte_rewrites",
    ),
];

/// These pre-home fixed-point cleanups predate the transactional driver. They
/// consume explicit routine liveness and are retained as reference passes, not
/// local definition-eliding matcher APIs.
const TYPED_CONTEXT_EXEMPTIONS: &[(&str, &str)] = &[
    (
        "src/mir6502/materialize/dead_spills.rs",
        "remove_dead_spill_stores",
    ),
    (
        "src/mir6502/materialize/temps.rs",
        "cleanup_pre_materialization_temp_artifacts_with_liveness",
    ),
    (
        "src/mir6502/materialize/word_values.rs",
        "forward_unique_word_load_address_consumers",
    ),
    (
        "src/mir6502/materialize/ssa_lite.rs",
        "fold_mir_copy_prop_const_uses_with_terminator_and_live_out",
    ),
];

#[derive(Debug)]
struct InventoryRow<'a> {
    id: &'a str,
    batch: &'a str,
    source: &'a str,
    entry_point: &'a str,
}

fn inventory_rows(text: &str) -> Vec<InventoryRow<'_>> {
    assert_eq!(
        text.lines().next(),
        Some("id\tphase\tbatch\tdomain\tsource\tentry_point\tbaseline_proof")
    );
    text.lines()
        .skip(1)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let columns = line.split('\t').collect::<Vec<_>>();
            assert_eq!(columns.len(), 7, "invalid inventory row: {line}");
            InventoryRow {
                id: columns[0],
                batch: columns[2],
                source: columns[4],
                entry_point: columns[5],
            }
        })
        .collect()
}

#[test]
fn rewrite_inventory_has_no_pending_liveness_migrations() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let text = fs::read_to_string(root.join(INVENTORY)).expect("read rewrite inventory");
    let rows = inventory_rows(&text);
    let mut ids = BTreeSet::new();
    let entries = rows
        .iter()
        .map(|row| (row.source, row.entry_point))
        .collect::<BTreeSet<_>>();

    for row in &rows {
        assert!(ids.insert(row.id), "duplicate inventory id {}", row.id);
        assert_eq!(
            row.batch, "migrated",
            "pending liveness migration {}: {}::{}",
            row.id, row.source, row.entry_point
        );
    }
    for &entry in ANALYZED_PRODUCER_REMOVERS {
        assert!(
            entries.contains(&entry),
            "audit entry missing: {}::{}",
            entry.0,
            entry.1
        );
    }
}

#[test]
fn definition_eliding_inventory_entry_points_require_analysis_context() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let text = fs::read_to_string(root.join(INVENTORY)).expect("read rewrite inventory");
    let rows = inventory_rows(&text);
    let exemptions = TYPED_CONTEXT_EXEMPTIONS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut sources = BTreeMap::<&str, String>::new();

    for row in rows {
        let source_text = sources.entry(row.source).or_insert_with(|| {
            fs::read_to_string(root.join(row.source))
                .unwrap_or_else(|error| panic!("read inventory source {}: {error}", row.source))
        });
        let marker = format!("fn {}", row.entry_point);
        let start = source_text
            .find(&marker)
            .unwrap_or_else(|| panic!("stale inventory entry {}::{}", row.source, row.entry_point));
        if exemptions.contains(&(row.source, row.entry_point)) {
            continue;
        }
        let signature = source_text[start..]
            .split_once('{')
            .map(|(signature, _)| signature)
            .expect("function signature has a body");
        assert!(
            signature.contains("PreHomeRewriteContext")
                || signature.contains("PostHomeRewriteContext"),
            "definition-eliding matcher lacks typed context: {}::{}",
            row.source,
            row.entry_point
        );
    }
}
