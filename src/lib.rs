mod analysis;

pub mod asm6502;
pub mod ast;
pub mod build_info;
pub mod cli;
pub mod codegen;
pub mod compiler;
pub mod diagnostic;
pub mod embedded_vfs;
pub mod includes;
pub mod lexer;
pub mod map_query;
pub mod mir6502;
pub mod nir;
pub mod parser;
pub mod resident;
pub mod runner;
pub mod runtime;
pub mod semantic;
pub mod source;

pub mod tac {
    pub use crate::nir::{
        BlockId, LocalId, NirBinaryOp as TacBinaryOp, NirBlock as TacBlock,
        NirBlockParam as TacBlockParam, NirCallEffects as TacCallEffects,
        NirCallResult as TacCallResult, NirCallableSignature as TacCallableSignature,
        NirCallee as TacCallee, NirCompareOp as TacCompareOp, NirDiagnostic as TacDiagnostic,
        NirEdge as TacEdge, NirGlobal as TacGlobal, NirInlineAsm as TacInlineAsm,
        NirInlineAsmRelocation as TacInlineAsmRelocation, NirInlineAsmTarget as TacInlineAsmTarget,
        NirLocal as TacLocal, NirMachineEffects as TacMachineEffects,
        NirMachineItem as TacMachineItem, NirMemoryAccess as TacMemoryAccess,
        NirMemoryEffects as TacMemoryEffects, NirMemoryRegion as TacMemoryRegion,
        NirMemoryRegionKind as TacMemoryRegionKind, NirOp as TacOp, NirOperand as TacOperand,
        NirOperandKind as TacOperandKind, NirParam as TacParam, NirPlace as TacPlace,
        NirPlaceKind as TacPlaceKind, NirProgram as TacProgram, NirRoutine as TacRoutine,
        NirRoutineNote as TacRoutineNote, NirRoutineNoteKind as TacRoutineNoteKind,
        NirStaticData as TacStaticData, NirStorageClass as TacStorageClass,
        NirStorageId as TacStorageId, NirTemp as TacTemp, NirTempDef as TacTempDef,
        NirTerminator as TacTerminator, NirType as TacType, NirTypeKind as TacTypeKind,
        NirUnaryOp as TacUnaryOp, NirValue as TacValue, ParamId, SymbolId, TempId,
        direct_storage_id, format_program, lower_program, optimize_program, verify_program,
    };
}
