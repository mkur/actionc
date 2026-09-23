use super::*;

fn builder(routine: &Mir65816Routine) -> Builder<'_> {
    let frame = AllocatedFrame::new(routine).unwrap();
    let mut code = TrackedEmitter65816::for_test(&frame);
    let blocks = routine
        .blocks
        .iter()
        .map(|b| (b.id, code.label()))
        .collect();
    Builder {
        routine,
        frame,
        code,
        blocks,
        next_block: None,
        loop_x: None,
    }
}

/// A repeatable inventory of verified MIR, not a count inferred from opcode bytes.
#[test]
#[ignore = "set A816_COMPARE_SOURCE, A816_COMPARE_MODULES and A816_COMPARE_INVENTORY"]
fn external_comparison_inventory() {
    let source = std::env::var_os("A816_COMPARE_SOURCE").unwrap();
    let mut modules = crate::includes::ModuleLoadOptions::default();
    modules.module_paths =
        std::env::split_paths(&std::env::var_os("A816_COMPARE_MODULES").unwrap()).collect();
    let mut results = Vec::new();
    for optimize in [false, true] {
        let p = crate::compiler::native65816::prepare_file(&source, optimize, &modules).unwrap();
        let mut groups = BTreeMap::<String, usize>::new();
        for r in p
            .mir
            .routines
            .iter()
            .filter(|r| !r.entry.external && !r.blocks.is_empty())
        {
            let b = builder(r);
            let sole = liveness::sole_branch_conditions(r);
            for block in &r.blocks {
                for (index, op) in block.ops.iter().enumerate() {
                    let Mir65816Op::Compare {
                        dest,
                        width,
                        signed,
                        operation,
                        left,
                        right,
                    } = op
                    else {
                        continue;
                    };
                    let adjacent = index + 1 == block.ops.len()
                        && sole.contains(dest)
                        && matches!(
                        &block.terminator,Mir65816Terminator::Branch{condition:Mir65816Value::Temp(id,w),..}
                        if id==dest && *w==ByteSize::ONE);
                    let native_word = b
                        .word_condition(*dest, width.get() as u8, *signed, *operation, left, right)
                        .unwrap()
                        .is_some();
                    let key = format!(
                        "width={}/signed={signed}/{operation:?}/{}/{}/{}",
                        width.get(),
                        if adjacent {
                            "sole-branch"
                        } else {
                            "materialized"
                        },
                        if native_word {
                            "word-selector"
                        } else {
                            "generic"
                        },
                        r.name.split('_').nth(1).unwrap_or(&r.name)
                    );
                    *groups.entry(key).or_default() += 1;
                }
            }
        }
        results.push(serde_json::json!({"optimize":optimize,"groups":groups}));
    }
    std::fs::write(
        std::env::var_os("A816_COMPARE_INVENTORY").unwrap(),
        serde_json::to_vec_pretty(&results).unwrap(),
    )
    .unwrap();
}
