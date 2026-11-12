use actionc::nir::{self, NirGlobalBacking, NirOp, NirPlaceKind, SymbolId};
use std::{collections::BTreeSet, path::Path};

#[test]
fn cursor_mutators_are_audited_by_resolved_storage() {
    for name in ["TN.ACT", "TNDBG.ACT"] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("samples/tn/modern")
            .join(name);
        let loaded = actionc::includes::load_program_with_expanded_source(&path).unwrap();
        let model = actionc::semantic::analyze_with_options(
            &loaded.program,
            actionc::semantic::SemanticOptions::modern(),
        )
        .unwrap();
        let program = nir::lower_program(&actionc::semantic::ir::lower_program(
            &loaded.program,
            &model,
        ));
        nir::verify_program(&program).unwrap();
        fn address(p: &nir::NirProgram, id: SymbolId, depth: usize) -> Option<u64> {
            if depth > p.globals.len() {
                return None;
            }
            match p.globals.iter().find(|g| g.id == id)?.backing {
                NirGlobalBacking::Absolute(a) => Some(a.value),
                NirGlobalBacking::Alias { target, offset } => {
                    address(p, target, depth + 1)?.checked_add(u64::from(offset.get()))
                }
                NirGlobalBacking::Ordinary => None,
            }
        }
        let cursor = program
            .globals
            .iter()
            .find(|g| g.name.eq_ignore_ascii_case("allocp"))
            .unwrap();
        assert_eq!(address(&program, cursor.id, 0), Some(0xE8));
        let mut mutators = BTreeSet::new();
        for routine in &program.routines {
            for op in routine.blocks.iter().flat_map(|b| &b.ops) {
                if let NirOp::Store { place, ty, .. } | NirOp::VolatileStore { place, ty, .. } = op
                {
                    let start = match place.kind {
                        NirPlaceKind::Global { id, .. } => address(&program, id, 0),
                        NirPlaceKind::Absolute(a) => Some(a.value),
                        _ => None,
                    };
                    if start
                        .is_some_and(|a| a < 0xEA && a + u64::from(ty.width.unwrap().get()) > 0xE8)
                    {
                        mutators.insert(routine.name.as_str());
                    }
                }
            }
        }
        assert_eq!(
            mutators,
            ["Alloc", "Free", "Init", "Pull", "Push", "RestorePos"]
                .into_iter()
                .collect()
        );
        let optimized = nir::optimize_program(&program).unwrap();
        for p in [&program, &optimized] {
            let ranges = nir::analyze_unsigned_ranges(p).unwrap();
            for name in ["ArenaCursor", "Alloc"] {
                let r = p.routines.iter().find(|r| r.name == name).unwrap();
                let range = ranges.normal_return_range(r.id).unwrap();
                assert_eq!((range.min(), range.max()), (0x2000, 0x2800), "{name}");
            }
            let generic = p.routines.iter().find(|r| r.name == "Strcat").unwrap();
            assert!(
                ranges
                    .storage_ranges_at_entry(generic.id, generic.blocks[0].id)
                    .is_empty()
            );
        }
    }
}
