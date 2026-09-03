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

pub use analysis::storage::{
    NirProgramStorageAnalysis, NirPromotionBlocker, NirRoutineStorageAnalysis,
    NirStorageBackingClass, NirStorageFacts, analyze_program_storage,
};
pub use facts::{
    BlockId, LocalId, NirStorageId, NirType, NirTypeKind, NirValue, ParamId, SignatureId,
    SymbolId, TempId, direct_storage_id,
};
pub use ir::{
    NirArrayGlobalFact, NirBinaryOp, NirBlock, NirBlockParam, NirCallEffects, NirCallResult,
    NirCastKind,
    NirCallableSignature, NirCallee, NirCompareOp, NirDataAddressEncoding, NirDataAddressTarget,
    NirDataBacking, NirDataFragment, NirDataImage, NirEdge, NirGlobal, NirGlobalBacking,
    NirGlobalInit, NirInlineAsm, NirInlineAsmRelocation, NirInlineAsmTarget, NirLocal,
    NirLocalBacking, NirLocalPurpose, NirMachineAtom, NirMachineByteSelector, NirMachineEffects,
    NirMachineItem, NirMemoryAccess, NirMemoryEffects, NirMemoryRegion, NirMemoryRegionKind, NirOp,
    NirParam, NirPlace, NirPlaceKind, NirProgram, NirRealOp, NirRealSource, NirRoutine,
    NirRoutineNote, NirRoutineNoteKind, NirRuntimeHelperTarget, NirStaticData, NirStorageBacking,
    NirLinkValue, NirStorageClass, NirStorageInit, NirTemp, NirTempDef, NirTerminator, NirUnaryOp,
};
pub use stats::{
    NirPlaceStats, NirProgramStats, NirStorageKindStats, NirStorageStats, collect_program_stats,
    format_stats_comparison,
};
pub use verifier::NirDiagnostic;
pub use crate::target::{AddressSpaceId, AddressValue, ByteOffset, ByteSize};

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
