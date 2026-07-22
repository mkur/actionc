use std::collections::BTreeMap;

use crate::nir::{NirCallEffects, NirCallableSignature, NirCallee};

use super::abi::{
    action_arg_home, action_arg_width_bytes, action_call_clobbers, mir_memory_effect,
};
use super::builtin::{MirBuiltinResolution, resolve_builtin_target};
use super::diagnostics::MirDiagnostic;
use super::ir::{
    MirCallAbi, MirCallArg, MirCallResult, MirCallTarget, MirDef, MirEffects, MirMemoryEffect,
    MirRegisterSet, MirResultHome, MirValue, MirWidth, RoutineId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MirCallPlan {
    pub target: MirCallTarget,
    pub abi: MirCallAbi,
    pub args: Vec<MirCallArg>,
    pub result: Option<MirCallResult>,
    pub effects: MirEffects,
}

pub(super) fn plan_call(
    routine: &str,
    block: &str,
    callee: &NirCallee,
    signature: &NirCallableSignature,
    args: &[(MirValue, MirWidth)],
    result: Option<(MirDef, MirWidth)>,
    indirect_target: Option<(MirValue, MirWidth)>,
    effects: &NirCallEffects,
    routine_ids: &BTreeMap<&str, RoutineId>,
    routine_system_addresses: &BTreeMap<&str, u16>,
    diagnostics: &mut Vec<MirDiagnostic>,
) -> Option<MirCallPlan> {
    let target = lower_call_target(
        routine,
        block,
        callee,
        indirect_target,
        routine_ids,
        routine_system_addresses,
        diagnostics,
    )?;
    if signature.variadic.is_none() && args.len() > signature.params.len() {
        diagnostics.push(MirDiagnostic::block(
            routine,
            block,
            "call argument count does not match signature",
        ));
        return None;
    }
    if signature.variadic.is_some() && args.len() < signature.params.len() {
        diagnostics.push(MirDiagnostic::block(
            routine,
            block,
            "call argument count does not match signature",
        ));
        return None;
    }
    let primary_homes = args
        .iter()
        .map(|(_, width)| *width)
        .scan(0u16, |offset, width| {
            let home = action_arg_home(*offset, width);
            *offset = offset.saturating_add(action_arg_width_bytes(width));
            Some(home)
        })
        .collect::<Vec<_>>();
    let call_args = args
        .iter()
        .cloned()
        .zip(primary_homes.iter().cloned())
        .map(|((value, width), home)| MirCallArg { value, width, home })
        .collect::<Vec<_>>();
    let homes = call_args
        .iter()
        .map(|arg| arg.home.clone())
        .collect::<Vec<_>>();
    let result_home = result
        .as_ref()
        .map(|_| MirResultHome::ReturnSlot { offset: 0 });
    let result = result.map(|(dst, width)| MirCallResult {
        dst,
        width,
        home: MirResultHome::ReturnSlot { offset: 0 },
    });
    let effects = if is_external_call_target(&target) {
        opaque_external_call_effects(mir_call_effects(effects))
    } else {
        mir_call_effects(effects)
    };
    let abi = MirCallAbi {
        params: homes,
        result: result_home,
        clobbers: effects.clobbers,
        preserves: effects.preserves,
    };
    Some(MirCallPlan {
        target,
        abi,
        args: call_args,
        result,
        effects,
    })
}

fn lower_call_target(
    routine: &str,
    block: &str,
    callee: &NirCallee,
    indirect_target: Option<(MirValue, MirWidth)>,
    routine_ids: &BTreeMap<&str, RoutineId>,
    routine_system_addresses: &BTreeMap<&str, u16>,
    diagnostics: &mut Vec<MirDiagnostic>,
) -> Option<MirCallTarget> {
    match callee {
        NirCallee::User(name) => {
            if let Some(address) = routine_system_addresses.get(name.as_str()) {
                return Some(MirCallTarget::Runtime {
                    name: name.clone(),
                    address: Some(*address),
                });
            }
            routine_ids
                .get(name.as_str())
                .copied()
                .map(MirCallTarget::Routine)
                .or_else(|| {
                    diagnostics.push(MirDiagnostic::block(
                        routine,
                        block,
                        format!("direct call target `{name}` does not have a routine id"),
                    ));
                    None
                })
        }
        NirCallee::Runtime { name, address } => Some(MirCallTarget::Runtime {
            name: name.clone(),
            address: *address,
        }),
        NirCallee::Builtin(name) => Some(MirCallTarget::Builtin {
            name: name.clone(),
            address: match resolve_builtin_target(name) {
                MirBuiltinResolution::Resolved { address } => Some(address),
                MirBuiltinResolution::Deferred { .. }
                | MirBuiltinResolution::Unsupported { .. }
                | MirBuiltinResolution::Unknown => None,
            },
        }),
        NirCallee::Indirect { .. } => indirect_target
            .map(|(target, width)| MirCallTarget::Indirect { target, width })
            .or_else(|| {
                diagnostics.push(MirDiagnostic::block(
                    routine,
                    block,
                    "indirect call target could not be materialized",
                ));
                None
            }),
    }
}

fn mir_call_effects(effects: &NirCallEffects) -> MirEffects {
    MirEffects {
        memory_reads: mir_memory_effect(&effects.memory.reads),
        memory_writes: mir_memory_effect(&effects.memory.writes),
        clobbers: action_call_clobbers(),
        preserves: MirRegisterSet::default(),
        stack_depth_delta: None,
        may_call_os: effects.may_call_os,
        opaque: effects.opaque,
    }
}

fn is_external_call_target(target: &MirCallTarget) -> bool {
    !matches!(target, MirCallTarget::Routine(_))
}

fn opaque_external_call_effects(effects: MirEffects) -> MirEffects {
    MirEffects {
        memory_reads: MirMemoryEffect::Unknown,
        memory_writes: MirMemoryEffect::Unknown,
        clobbers: MirRegisterSet {
            a: true,
            x: true,
            y: true,
            flags: true,
            sp: effects.clobbers.sp,
        },
        preserves: MirRegisterSet::default(),
        stack_depth_delta: effects.stack_depth_delta,
        may_call_os: true,
        opaque: true,
    }
}
