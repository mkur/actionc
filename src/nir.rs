mod analysis;
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
mod verifier;

#[cfg(test)]
mod tests;

use crate::semantic::ir::SemProgram;

pub use crate::target::{
    AddressSpaceId, AddressValue, ByteOffset, ByteSize, RoutineActivationModel,
};
pub use analysis::storage::{
    NirProgramStorageAnalysis, NirPromotionBlocker, NirRoutineStorageAnalysis,
    NirStorageBackingClass, NirStorageFacts, analyze_program_storage,
};
pub use facts::{
    BlockId, LocalId, NirStorageId, NirType, NirTypeKind, NirValue, ParamId, RoutineId,
    RuntimeSymbolId, SignatureId, SymbolId, TempId, direct_storage_id, runtime_symbol_id,
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
    let optimized = storage_optimizer::propagate_program(&optimized)?;
    let optimized = promotion::promote_program(&optimized)?;
    let optimized = home_elision::elide_program(&optimized)?;
    optimizer::optimize_program(&optimized)
}
