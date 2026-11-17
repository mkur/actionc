//! Entry transfers and successful-return writes. These are deliberately not
//! full observation summaries for deletion or motion.
use super::*;
use crate::mir6502::analysis::{
    effects::{MirFlagSet, MirHomeByte},
    home_liveness::MirHomeTransfer,
    machine_liveness::MirMachineLiveSet,
};

pub(in crate::mir6502) struct InvocationSummary {
    pub inputs: Vec<MirArgHome>,
    pub definitions: Vec<MirMem>,
    pub register_definitions: MirRegisterSet,
    pub writes: Vec<MirMem>,
    pub buffers: Vec<super::buffers::BufferAccess>,
}

impl InvocationSummary {
    pub fn home_transfer(&self) -> MirHomeTransfer {
        MirHomeTransfer {
            reads: self.inputs.iter().filter_map(|h| match h {
                MirArgHome::FixedZeroPage(s) => Some(MirHomeByte::FixedZeroPage(*s)),
                _ => None,
            }).collect(),
            writes: self.definitions.iter().filter_map(|m| match m {
                MirMem::FixedZeroPage(s) => Some(MirHomeByte::FixedZeroPage(*s)),
                _ => None,
            }).collect(),
        }
    }

    pub fn machine_transfer(&self) -> (MirMachineLiveSet, MirMachineLiveSet) {
        let mut registers = MirRegisterSet::default();
        for home in &self.inputs {
            match home {
                MirArgHome::Reg(MirReg::A) => registers.a = true,
                MirArgHome::Reg(MirReg::X) => registers.x = true,
                MirArgHome::Reg(MirReg::Y) => registers.y = true,
                _ => {},
            }
        }
        (MirMachineLiveSet { registers, flags: MirFlagSet::default() },
            MirMachineLiveSet { registers: self.register_definitions, flags: MirFlagSet::default() })
    }
}

pub(in crate::mir6502) fn invocation(program: &MirProgram, routine: &MirRoutine, op: &MirOp) -> Option<InvocationSummary> {
    if let Some(capture) = sargs_capture(program, routine, op) {
        return Some(InvocationSummary {
            inputs: capture.inputs,
            writes: capture.definitions.iter().cloned().chain(capture.scratch).collect(),
            definitions: capture.definitions,
            register_definitions: MirRegisterSet::default(),
            buffers: Vec::new(),
        });
    }
    helper_invocation(program, op).or_else(|| super::library_invocation(program, op))
}

pub(in crate::mir6502) fn helper_invocation(program: &MirProgram, op: &MirOp) -> Option<InvocationSummary> {
    let MirOp::RuntimeHelper { helper, args, result, additional_results, effects } = op else { return None; };
    if !matches!(helper, MirRuntimeHelper::MulByte | MirRuntimeHelper::Mul | MirRuntimeHelper::Lsh | MirRuntimeHelper::Rsh
        | MirRuntimeHelper::Mul32 | MirRuntimeHelper::Lsh32 | MirRuntimeHelper::Rsh32
        | MirRuntimeHelper::Div | MirRuntimeHelper::Mod | MirRuntimeHelper::UDiv | MirRuntimeHelper::UMod
        | MirRuntimeHelper::DivMod | MirRuntimeHelper::UDivMod
        | MirRuntimeHelper::DivU8 | MirRuntimeHelper::ModU8 | MirRuntimeHelper::DivU16U8 | MirRuntimeHelper::ModU16U8
        | MirRuntimeHelper::Div32 | MirRuntimeHelper::Mod32 | MirRuntimeHelper::UDiv32 | MirRuntimeHelper::UMod32) { return None; }
    selected_helper(program, *helper)?;
    if *args != signatures::helper_args(helper) || *effects != signatures::helper_effects(helper)
        || *additional_results != signatures::helper_additional_results(*helper)
        || result.as_ref().is_some_and(|h| signatures::primary_result(*helper).as_ref().map(|r| &r.home) != Some(h)) {
        return None;
    }
    let mut definitions = Vec::new();
    let mut register_definitions = MirRegisterSet::default();
    for lane in signatures::result_lanes(*helper) {
        match lane {
            MirParamLane::Reg(MirReg::A) => register_definitions.a = true,
            MirParamLane::Reg(MirReg::X) => register_definitions.x = true,
            MirParamLane::Reg(MirReg::Y) => register_definitions.y = true,
            MirParamLane::ZeroPage(slot) => definitions.push(MirMem::FixedZeroPage(slot)),
        }
    }
    let writes = match signatures::helper_return_effects(helper).memory_writes {
        MirMemoryEffect::None => Vec::new(),
        MirMemoryEffect::Regions(regions) => {
            let mut bytes = Vec::new();
            for region in regions {
                if region.kind != MirMemoryRegionKind::ZeroPage { return None; }
                for offset in region.offset..region.offset.checked_add(region.size)? {
                    bytes.push(MirMem::FixedZeroPage(MirFixedZpSlot(u8::try_from(offset).ok()?)));
                }
            }
            bytes
        },
        _ => return None,
    };
    Some(InvocationSummary { inputs: args.clone(), definitions, register_definitions, writes, buffers: Vec::new() })
}
