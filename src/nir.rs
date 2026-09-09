mod analysis;
mod aggregate_forwarding;
mod classifier;
mod facts;
mod home_elision;
mod ir;
mod lowerer;
mod optimizer;
mod printer;
mod promotion;
mod stats;
mod storage_optimizer;
mod subregion_constants;
mod verifier;

#[cfg(test)]
mod tests;

use crate::semantic::ir::SemProgram;

pub use crate::target::{
    AddressSpaceId, AddressValue, ByteOffset, ByteSize, RoutineActivationModel,
};
pub use analysis::aggregate_regions::{
    NirAggregateAddressUse, NirAggregatePoint, NirAggregateProofFailure,
    NirAggregateRegionAnalysis, NirExactStorageRegion, NirRegionRelation,
    NirRoutineAggregateRegions, analyze_aggregate_regions,
};
pub use analysis::storage::{
    NirProgramStorageAnalysis, NirPromotionBlocker, NirRoutineStorageAnalysis,
    NirStorageBackingClass, NirStorageFacts, NirStorageIdentityDomain, analyze_program_storage,
};
pub use facts::{
    BlockId, LocalId, NirCallableKind, NirIntegerRole, NirIntegerType, NirStorageId, NirType,
    NirTypeKind, NirValue, ParamId, RoutineId, RuntimeSymbolId, SignatureId, SymbolId, TempId,
    direct_storage_id, runtime_symbol_id,
};
pub use ir::{
    ExternalAbiId, NirActivationModel, NirArrayGlobalFact, NirBinaryOp, NirBlock, NirBlockParam,
    NirCallConvention, NirCallEffects, NirCallResult, NirCallableSignature, NirCallee, NirCastKind,
    NirCompareOp, NirDataAddressEncoding, NirDataAddressTarget, NirDataBacking, NirDataFragment,
    NirDataImage, NirEdge, NirForeignCode, NirForeignCodeKind, NirForeignCodePayload,
    NirForeignCodeTarget, NirForeignRelocation, NirGlobal, NirGlobalBacking, NirGlobalInit,
    NirLinkValue, NirLocal, NirLocalBacking, NirLocalPurpose, NirMachineAtom,
    NirMachineByteSelector, NirMachineEffects, NirMachineItem, NirMemoryAccess, NirMemoryEffects,
    NirMemoryRegion, NirMemoryRegionKind, NirObjectLayout, NirOp, NirParam, NirPlace, NirPlaceKind,
    NirProgram, NirRealOp, NirRealSource, NirRoutine, NirRoutineEntry, NirRoutineNote,
    NirRoutineNoteKind, NirRoutinePlacement, NirRuntimeBinding, NirRuntimeTarget, NirStaticData,
    NirStorageBacking, NirStorageClass, NirStorageDuration, NirStorageInit, NirTemp, NirTempDef,
    NirTerminator, NirUnaryOp,
};
pub use stats::{
    NirPlaceStats, NirProgramStats, NirStorageKindStats, NirStorageStats, collect_program_stats,
    format_stats_comparison,
};
pub use verifier::NirDiagnostic;

pub fn lower_program(program: &SemProgram) -> NirProgram {
    let mut lowerer = lowerer::NirLowerer::default();
    lowerer.program(program)
}

pub fn format_program(program: &NirProgram) -> String {
    let mut printer = printer::NirPrinter::default();
    printer.program(program);
    printer.finish()
}

pub fn verify_program(program: &NirProgram) -> Result<(), Vec<NirDiagnostic>> {
    verifier::verify_program(program)
}

pub fn optimize_program(program: &NirProgram) -> Result<NirProgram, Vec<NirDiagnostic>> {
    let optimized = optimizer::optimize_program(program)?;
    // Programs without aggregate captures retain the existing scalar schedule.
    // The dependent fixed point is only needed for the new byte proofs.
    if !optimized.routines.iter().any(|routine| {
        routine
            .locals
            .iter()
            .any(|local| local.purpose == NirLocalPurpose::AggregateCapture)
    }) {
        let optimized = aggregate_forwarding::forward_program(&optimized)?;
        let optimized = storage_optimizer::propagate_program(&optimized)?;
        let optimized = promotion::promote_program(&optimized)?;
        let optimized = home_elision::elide_program(&optimized)?;
        return optimizer::optimize_program(&optimized);
    }
    let optimized = optimize_storage_values(&optimized)?;
    // Scalar promotion is a representation change and runs exactly once.
    let optimized = promotion::promote_program(&optimized)?;
    optimize_storage_values(&optimized)
}

/// Stabilize only dependent memory/value/CFG cleanup. A byte replacement can
/// expose a scalar constant or redundant snapshot, and either can expose another
/// byte proof. Home removal can also discharge an obsolete address-use blocker.
fn optimize_storage_values(program: &NirProgram) -> Result<NirProgram, Vec<NirDiagnostic>> {
    let budget = program
        .routines
        .iter()
        .map(|routine| {
            routine.locals.len()
                + routine
                    .blocks
                    .iter()
                    .map(|block| block.ops.len() + 1)
                    .sum::<usize>()
        })
        .sum::<usize>();
    let mut optimized = program.clone();
    for _ in 0..=budget {
        let next = aggregate_forwarding::forward_program(&optimized)?;
        let next = subregion_constants::propagate_program(&next)?;
        let next = storage_optimizer::propagate_program(&next)?;
        let next = home_elision::elide_program(&next)?;
        let next = optimizer::optimize_program(&next)?;
        if next == optimized {
            return Ok(next);
        }
        optimized = next;
    }
    // Every component preserves verification; budget exhaustion is conservative.
    Ok(optimized)
}
