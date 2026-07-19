use std::collections::{BTreeMap, BTreeSet};

use crate::nir::{NirCallEffects, NirCallableSignature, NirCallee};

use super::abi::{action_call_clobbers, mir_memory_effect};
use super::builtin::{MirBuiltinResolution, resolve_builtin_target};
use super::diagnostics::MirDiagnostic;
use super::ir::{
    MirArgHome, MirCallAbi, MirCallArg, MirCallResult, MirCallTarget, MirDef, MirEffects,
    MirFixedZpSlot, MirMemoryEffect, MirReg, MirRegisterSet, MirResultHome, MirValue, MirWidth,
    RoutineId,
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
    public_action_abi_routines: &BTreeSet<&str>,
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
            let home = action_abi_arg_home(*offset, width);
            *offset = offset.saturating_add(width_bytes(width));
            Some(home)
        })
        .collect::<Vec<_>>();
    let mirror_public_homes = matches!(callee, NirCallee::User(name) if public_action_abi_routines.contains(name.as_str()));
    let mut call_args = Vec::new();
    if mirror_public_homes {
        let mut offset = 0u16;
        for (value, width) in args {
            if let Some(home) = public_action_abi_shadow_home(offset, *width) {
                call_args.push(MirCallArg {
                    value: value.clone(),
                    width: *width,
                    home,
                });
            }
            offset = offset.saturating_add(width_bytes(*width));
        }
    }
    call_args.extend(
        args.iter()
            .cloned()
            .zip(primary_homes.iter().cloned())
            .map(|((value, width), home)| MirCallArg { value, width, home }),
    );
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

fn action_abi_arg_home(offset: u16, width: MirWidth) -> MirArgHome {
    match width {
        MirWidth::Byte => action_abi_byte_home(offset),
        MirWidth::Word => {
            let lo = action_abi_byte_home(offset);
            let hi = action_abi_byte_home(offset.saturating_add(1));
            match (&lo, &hi) {
                (MirArgHome::Reg(lo), MirArgHome::Reg(hi)) => {
                    MirArgHome::RegisterPair { lo: *lo, hi: *hi }
                }
                _ => MirArgHome::BytePair {
                    lo: Box::new(lo),
                    hi: Box::new(hi),
                },
            }
        }
    }
}

fn action_abi_byte_home(offset: u16) -> MirArgHome {
    match offset {
        0 => MirArgHome::Reg(MirReg::A),
        1 => MirArgHome::Reg(MirReg::X),
        2 => MirArgHome::Reg(MirReg::Y),
        _ => MirArgHome::FixedZeroPage(MirFixedZpSlot(
            u8::try_from(0x00A0u16.saturating_add(offset)).unwrap_or(u8::MAX),
        )),
    }
}

fn public_action_abi_shadow_home(offset: u16, width: MirWidth) -> Option<MirArgHome> {
    if offset >= 3 {
        return None;
    }
    let byte_home = |byte_offset: u16| {
        MirArgHome::FixedZeroPage(MirFixedZpSlot(
            u8::try_from(0x00A0u16.saturating_add(byte_offset)).unwrap_or(u8::MAX),
        ))
    };
    Some(match width {
        MirWidth::Byte => byte_home(offset),
        MirWidth::Word => MirArgHome::BytePair {
            lo: Box::new(byte_home(offset)),
            hi: Box::new(byte_home(offset.saturating_add(1))),
        },
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

fn width_bytes(width: MirWidth) -> u16 {
    match width {
        MirWidth::Byte => 1,
        MirWidth::Word => 2,
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
