use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

const INVENTORY: &str = "surveys/tn/mir6502-rewrite-migration-inventory.tsv";

const AUDITED_PRODUCER_REMOVERS: &[(&str, &str)] = &[
    (
        "src/mir6502/materialize/compare_branch.rs",
        "fold_compare_operand_producers_before_branches",
    ),
    (
        "src/mir6502/materialize/compare_branch.rs",
        "narrow_byte_bitwise_zero_compares",
    ),
    (
        "src/mir6502/materialize/compare_branch.rs",
        "try_fuse_byte_binary_compare_consumer",
    ),
    (
        "src/mir6502/materialize/compare_branch.rs",
        "try_fuse_byte_compare_consumer",
    ),
    (
        "src/mir6502/materialize/calls.rs",
        "fold_call_arg_producers",
    ),
    (
        "src/mir6502/materialize/calls.rs",
        "try_materialize_call_arg_expr_producers",
    ),
    (
        "src/mir6502/materialize/calls.rs",
        "forward_return_slot_call_result_args",
    ),
    (
        "src/mir6502/materialize/calls.rs",
        "try_forward_param_word_store_consumer",
    ),
    ("src/mir6502/rewrite/pilots.rs", "call_result_store_plan"),
    (
        "src/mir6502/rewrite/pilots.rs",
        "loaded_arg_call_result_store_plan",
    ),
    ("src/mir6502/rewrite/pilots.rs", "store_expr_consumer_plan"),
    ("src/mir6502/rewrite/pilots.rs", "cast_store_consumer_plan"),
    (
        "src/mir6502/rewrite/pilots.rs",
        "direct_copy_store_consumer_plan",
    ),
    ("src/mir6502/rewrite/pilots.rs", "word_store_consumer_plan"),
    (
        "src/mir6502/rewrite/pilots.rs",
        "byte_mul_add_sub_word_store_consumer_plan",
    ),
    (
        "src/mir6502/rewrite/pilots.rs",
        "byte_mul_word_store_consumer_plan",
    ),
    ("src/mir6502/rewrite/pilots.rs", "byte_store_consumer_plan"),
    (
        "src/mir6502/rewrite/pilots.rs",
        "direct_pointer_temp_rematerialization_plan",
    ),
    ("src/mir6502/rewrite/pilots.rs", "pointer_temp_deref_plan"),
    (
        "src/mir6502/materialize/indexes.rs",
        "collect_delayed_byte_index_plan",
    ),
    (
        "src/mir6502/materialize/indexes.rs",
        "try_fuse_indexed_byte_copy",
    ),
    (
        "src/mir6502/materialize/indexes.rs",
        "try_fuse_indexed_word_copy",
    ),
    (
        "src/mir6502/materialize/indexes.rs",
        "try_fuse_dynamic_inline_byte_index",
    ),
    (
        "src/mir6502/materialize/indexes.rs",
        "try_prepare_dynamic_byte_index",
    ),
    (
        "src/mir6502/materialize/indexes.rs",
        "try_prepare_dynamic_word_index",
    ),
    (
        "src/mir6502/rewrite/pilots.rs",
        "address_store_consumer_plan",
    ),
    ("src/mir6502/rewrite/pilots.rs", "discover_unused_lea_addrs"),
    (
        "src/mir6502/materialize/peepholes.rs",
        "next_style_word_store_forward_at",
    ),
    (
        "src/mir6502/materialize/peepholes.rs",
        "staged_byte_word_update_at",
    ),
    (
        "src/mir6502/materialize/peepholes.rs",
        "forwarded_staged_byte_word_update_at",
    ),
    (
        "src/mir6502/materialize/peepholes.rs",
        "indirect_byte_compound_at",
    ),
    (
        "src/mir6502/materialize/peepholes.rs",
        "indirect_byte_direct_compound_at",
    ),
    (
        "src/mir6502/materialize/peepholes.rs",
        "indirect_byte_forwarded_direct_compound_at",
    ),
    (
        "src/mir6502/materialize/peepholes.rs",
        "indirect_byte_const_compound_at",
    ),
    (
        "src/mir6502/materialize/peepholes.rs",
        "indirect_byte_delayed_const_compound_at",
    ),
    (
        "src/mir6502/materialize/peepholes.rs",
        "indirect_byte_const_store_at",
    ),
    (
        "src/mir6502/materialize/peepholes.rs",
        "indirect_y_const_store_at",
    ),
    (
        "src/mir6502/materialize/peepholes.rs",
        "indirect_byte_direct_store_at",
    ),
    (
        "src/mir6502/materialize/peepholes.rs",
        "word_array_store_value_staging_at",
    ),
    (
        "src/mir6502/materialize/indexes.rs",
        "fold_indexed_base_pointer_staging",
    ),
    ("src/mir6502/materialize/calls.rs", "forward_param_reload"),
];

#[test]
fn rewrite_inventory_covers_liveness_audit_entry_points() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let text = fs::read_to_string(root.join(INVENTORY)).expect("read rewrite inventory");
    let mut rows = BTreeMap::new();
    let mut ids = BTreeSet::new();

    for (line_index, line) in text.lines().enumerate() {
        if line_index == 0 {
            assert_eq!(
                line,
                "id\tphase\tbatch\tdomain\tsource\tentry_point\tbaseline_proof"
            );
            continue;
        }
        if line.is_empty() {
            continue;
        }
        let columns = line.split('\t').collect::<Vec<_>>();
        assert_eq!(columns.len(), 7, "invalid inventory row {}", line_index + 1);
        assert!(
            ids.insert(columns[0]),
            "duplicate inventory id {}",
            columns[0]
        );
        assert!(
            rows.insert((columns[4], columns[5]), columns[6]).is_none(),
            "duplicate inventory entry {}::{}",
            columns[4],
            columns[5]
        );
    }

    for &(source, entry_point) in AUDITED_PRODUCER_REMOVERS {
        assert!(
            rows.contains_key(&(source, entry_point)),
            "audit entry point missing from {INVENTORY}: {source}::{entry_point}"
        );
    }
}

#[test]
fn rewrite_inventory_entry_points_still_exist() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let text = fs::read_to_string(root.join(INVENTORY)).expect("read rewrite inventory");
    let mut sources = BTreeMap::<&str, String>::new();

    for line in text.lines().skip(1).filter(|line| !line.is_empty()) {
        let columns = line.split('\t').collect::<Vec<_>>();
        let source = columns[4];
        let entry_point = columns[5];
        let source_text = sources.entry(source).or_insert_with(|| {
            fs::read_to_string(root.join(source))
                .unwrap_or_else(|error| panic!("read inventory source {source}: {error}"))
        });
        assert!(
            source_text.contains(&format!("fn {entry_point}")),
            "stale inventory entry {source}::{entry_point}"
        );
    }
}
