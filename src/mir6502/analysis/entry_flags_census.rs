use super::*;
use crate::mir6502::{
    self,
    analysis::{
        argument_x_forwarding as x, known_callees::MirKnownCalleeSummaries,
        posthome::PostHomeAnalysisSnapshot, sites::*,
    },
    rewrite::context::PostHomeRewriteContext,
};
#[test]
#[ignore = "opt-in typed flag blocker census; writes ignored build artifacts"]
fn callee_entry_flags_census() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut rows = Vec::new();
    for name in ["TN", "TNDBG"] {
        let path = root.join(format!("samples/tn/modern/{name}.ACT"));
        let loaded = crate::includes::load_compilation(
            &path,
            &crate::includes::ModuleLoadOptions::default(),
        )
        .unwrap();
        let model = crate::semantic::analyze_compilation_with_options(
            &loaded,
            crate::semantic::SemanticOptions::modern(),
        )
        .unwrap();
        let semir = crate::linker::select_semir(
            &crate::semantic::ir::lower_compilation(&loaded, &model),
            crate::linker::SemLinkPolicy::EntryReachable,
        )
        .unwrap();
        let nir = crate::nir::optimize_program(&crate::nir::lower_program(&semir)).unwrap();
        let p = mir6502::materialize_program_with_origin(
            mir6502::lower_program(&nir).unwrap(),
            &mir6502::Mir6502Config { enable_argument_x_bitwise_forwarding: false, enable_precise_callee_entry_flags: false, ..mir6502::Mir6502Config::optimized() },
            0x2C00,
        )
        .unwrap();
        let mut emitter = crate::codegen::tracked_emitter::TrackedEmitter::with_origin(0x2C00);
        let memory = mir6502::emit::emit_program_with_physical_references(&p, 0x2C00, &mut emitter, true, &p).unwrap().physical_memory;
        let summaries = MirKnownCalleeSummaries::analyze_for_rewriting(&p);
        for r in &p.routines {
            let snap = PostHomeAnalysisSnapshot::new_with_known_callees(
                r,
                MirRoutineGeneration::initial(),
                &summaries,
            )
            .unwrap();
            let c = PostHomeRewriteContext::new(&snap);
            for b in &r.blocks {
                for i in 0..b.ops.len() {
                    let point = c.point(MirSite::Op {
                        block: b.id,
                        op_index: i,
                    });
                    if c.argument_x_forwarding(point, &memory) != Err(x::Blocker::CallFlags) {
                        continue;
                    }
                    let shape = x::shape(r, point.site).unwrap();
                    let MirSite::Op { op_index, .. } = shape.call else {
                        unreachable!()
                    };
                    let MirOp::Call {
                        target: MirCallTarget::Routine(id),
                        ..
                    } = b.ops[op_index]
                    else {
                        unreachable!()
                    };
                    let target = p.routines.iter().find(|r| r.id == id).unwrap();
                    let flags = MirKnownCalleeEntryFlags::analyze(target);
                    rows.push(serde_json::json!({"program":name,"caller":r.name,"caller_id":r.id.0,"block":b.id.0,"store":i,"target":target.name,"target_id":id.0,"summary":format!("{flags:?}"),"unused_nz":flags.unobserved(MirFlagSet{n:true,z:true,..MirFlagSet::default()}),"scalar_signature":target.scalar_signature.is_some(),"machine_blocks":target.blocks.iter().flat_map(|b|&b.ops).filter(|o|matches!(o,MirOp::MachineBlock{..})).count(),"effects_opaque":target.effects.opaque,"effects_reads_flags":target.effects.reads.flags,"effects_preserves_flags":target.effects.preserves.flags}));
                }
            }
        }
    }
    assert_eq!(rows.len(), 10);
    std::fs::create_dir_all(root.join("build/callee-entry-flags")).unwrap();
    std::fs::write(
        root.join("build/callee-entry-flags/census.json"),
        serde_json::to_string_pretty(&rows).unwrap(),
    )
    .unwrap();
    eprintln!("{}", serde_json::to_string_pretty(&rows).unwrap());
}
