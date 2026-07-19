mod analysis;
mod classifier;
mod facts;
mod ir;
mod lowerer;
mod optimizer;
mod printer;
mod stats;
mod verifier;

#[cfg(test)]
mod tests;

use crate::semantic::ir::SemProgram;

pub use facts::{BlockId, LocalId, NirType, NirTypeKind, NirValue, ParamId, SymbolId, TempId};
pub use ir::{
    NirBinaryOp, NirBlock, NirCallEffects, NirCallResult, NirCallableSignature, NirCallee,
    NirCompareOp, NirDataBacking, NirGlobal, NirGlobalBacking, NirGlobalInit, NirLocal,
    NirLocalBacking, NirMachineAtom, NirMachineByteSelector, NirMachineEffects, NirMachineItem,
    NirMemoryAccess, NirMemoryEffects, NirOp, NirOperand, NirOperandKind, NirParam, NirPlace,
    NirPlaceKind, NirProgram, NirRoutine, NirRoutineNote, NirRoutineNoteKind, NirStaticData,
    NirStorageBacking, NirStorageInit, NirTemp, NirTempDef, NirTerminator, NirUnaryOp,
};
pub use stats::{NirPlaceStats, NirProgramStats, collect_program_stats, format_stats_comparison};
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
    optimizer::optimize_program(program)
}
