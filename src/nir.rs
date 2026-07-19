mod analysis;
mod classifier;
mod facts;
mod ir;
mod lowerer;
mod optimizer;
mod printer;
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
    BlockId, LocalId, NirStorageId, NirType, NirTypeKind, NirValue, ParamId, SymbolId, TempId,
    direct_storage_id,
};
pub use ir::{
    NirBinaryOp, NirBlock, NirCallEffects, NirCallResult, NirCallableSignature, NirCallee,
    NirCompareOp, NirDataBacking, NirGlobal, NirGlobalBacking, NirGlobalInit, NirLocal,
    NirLocalBacking, NirMachineAtom, NirMachineByteSelector, NirMachineEffects, NirMachineItem,
    NirMemoryAccess, NirMemoryEffects, NirMemoryRegion, NirMemoryRegionKind, NirOp, NirOperand,
    NirOperandKind, NirParam, NirPlace, NirPlaceKind, NirProgram, NirRoutine, NirRoutineNote,
    NirRoutineNoteKind, NirStaticData, NirStorageBacking, NirStorageClass, NirStorageInit, NirTemp,
    NirTempDef, NirTerminator, NirUnaryOp,
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
    optimizer::optimize_program(&optimized)
}
