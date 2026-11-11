//! Complete captured-word facts for a bounded five-byte direct-call schedule.
//! Keeping a value in X does not establish permission to delete its capture.
#![allow(dead_code)] // The selection consumer is introduced in the next slice.
use super::call_predicates::ordinary_source;
use super::sites::{MirProgramPoint, MirSite};
use super::use_def::MirDefSite;
use crate::mir6502::ir::*;
use crate::mir6502::materialize::MaterializeLayout;
use crate::mir6502::rewrite::context::{MirProof, PreHomeRewriteContext};

pub(in crate::mir6502) const MAX_SETUP_OPS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mir6502) enum Blocker {
    Site,
    Shape,
    Callee,
    Capture,
    Storage,
    Budget,
    Placement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) struct ArgumentStaging {
    pub call: MirProgramPoint,
    pub producer: MirSite,
    pub captured: MirTempId,
    pub definitions: [MirDefSite; 2],
    pub sources: [(MirMem, MirWidth); 3],
}

pub(in crate::mir6502) fn prove(
    routine: &MirRoutine,
    context: &PreHomeRewriteContext<'_, '_>,
    call: MirProgramPoint,
) -> Result<ArgumentStaging, Blocker> {
    if !context.valid_point(call) {
        return Err(Blocker::Site);
    }
    let MirSite::Op { block, op_index } = call.site else {
        return Err(Blocker::Site);
    };
    let b = routine
        .blocks
        .iter()
        .find(|b| b.id == block)
        .ok_or(Blocker::Site)?;
    let Some(MirOp::Call {
        target,
        abi,
        args,
        effects,
        additional_results,
        ..
    }) = b.ops.get(op_index)
    else {
        return Err(Blocker::Shape);
    };
    if !matches!(target, MirCallTarget::Routine(_))
        || effects.opaque
        || effects.may_call_os
        || !additional_results.is_empty()
        || effects.reads != MirRegisterSet::default()
    {
        return Err(Blocker::Callee);
    }
    let [first, second, tag] = args.as_slice() else {
        return Err(Blocker::Shape);
    };
    let homes = [
        super::super::abi::action_arg_home(0, MirWidth::Word),
        super::super::abi::action_arg_home(2, MirWidth::Word),
        super::super::abi::action_arg_home(4, MirWidth::Byte),
    ];
    if first.width != MirWidth::Word
        || second.width != MirWidth::Word
        || tag.width != MirWidth::Byte
        || [first.home.clone(), second.home.clone(), tag.home.clone()] != homes
        || abi.params != homes
    {
        return Err(Blocker::Shape);
    }
    let MirValue::Def(MirDef::VTemp(captured)) = first.value else {
        return Err(Blocker::Shape);
    };
    let producer_index = op_index.checked_sub(1).ok_or(Blocker::Shape)?;
    let producer = MirSite::Op {
        block,
        op_index: producer_index,
    };
    let Some(MirOp::Binary {
        op: MirBinaryOp::Add | MirBinaryOp::Sub,
        dst: MirDef::VTemp(result),
        left: MirValue::Def(MirDef::VTemp(input)),
        right: MirValue::ConstU8(_) | MirValue::ConstU16(_),
        width: MirWidth::Word,
        ..
    }) = b.ops.get(producer_index)
    else {
        return Err(Blocker::Shape);
    };
    if *result != captured {
        return Err(Blocker::Shape);
    }
    let MirProof::Proven(definitions) = context.exclusive_word_temp_definition(captured, call.site)
    else {
        return Err(Blocker::Capture);
    };
    if definitions[0].site != producer {
        return Err(Blocker::Capture);
    }
    let MirProof::Proven(input_defs) = context.exclusive_word_temp_definition(*input, producer)
    else {
        return Err(Blocker::Capture);
    };
    let MirSite::Op {
        block: input_block,
        op_index: input_index,
    } = input_defs[0].site
    else {
        return Err(Blocker::Capture);
    };
    if input_block != block || input_index >= producer_index {
        return Err(Blocker::Capture);
    }
    if op_index - input_index + 1 > MAX_SETUP_OPS {
        return Err(Blocker::Budget);
    }
    let MirOp::Load {
        src: MirAddr::Direct(input_source),
        width: MirWidth::Word,
        ..
    } = &b.ops[input_index]
    else {
        return Err(Blocker::Storage);
    };
    let MirValue::PointerCell(tag_source) = &tag.value else {
        return Err(Blocker::Storage);
    };
    let second_source = direct_word_source(&second.value).ok_or(Blocker::Storage)?;
    let sources = [
        (input_source.clone(), MirWidth::Word),
        (second_source, MirWidth::Word),
        (tag_source.clone(), MirWidth::Byte),
    ];
    if sources
        .iter()
        .any(|(mem, width)| !ordinary_source(routine, mem, *width))
    {
        return Err(Blocker::Storage);
    }
    Ok(ArgumentStaging {
        call,
        producer,
        captured,
        definitions,
        sources,
    })
}

fn direct_word_source(value: &MirValue) -> Option<MirMem> {
    if let MirValue::PointerCell(mem) = value {
        return Some(mem.clone());
    }
    let MirValue::Word { lo, hi } = value else {
        return None;
    };
    let (MirValue::PointerCell(lo), MirValue::PointerCell(hi)) = (lo.as_ref(), hi.as_ref()) else {
        return None;
    };
    let expected = match lo {
        MirMem::Param { id, offset } => MirMem::Param {
            id: *id,
            offset: offset.checked_add(1)?,
        },
        MirMem::Local { id, offset } => MirMem::Local {
            id: *id,
            offset: offset.checked_add(1)?,
        },
        _ => return None,
    };
    (expected == *hi).then(|| lo.clone())
}

impl ArgumentStaging {
    pub(in crate::mir6502) fn placed(
        &self,
        routine: RoutineId,
        layout: &MaterializeLayout,
    ) -> bool {
        self.sources.iter().all(|(source, width)| {
            layout.mem_address(routine, source).is_some_and(|address| {
                address >= 0x100
                    && address
                        .checked_add(u16::from(*width == MirWidth::Word))
                        .is_some()
            })
        })
    }
}

#[cfg(test)]
#[path = "argument_staging_tests.rs"]
mod tests;
