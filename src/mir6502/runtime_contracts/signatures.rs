//! Private helper signatures and directional effects, shared by all consumers.
use super::super::ir::*;

pub(in crate::mir6502) fn helper_abi() -> MirCallAbi {
    MirCallAbi {
        library: None,
        additional_results: Vec::new(),
        params: vec![
            MirArgHome::StackFrame { base: 0, offset: 0 },
            MirArgHome::StackFrame { base: 0, offset: 2 },
        ],
        result: Some(MirResultHome::ReturnSlot { offset: 0 }),
        clobbers: MirRegisterSet {
            a: true,
            x: true,
            y: true,
            flags: true,
            sp: false,
        },
        preserves: MirRegisterSet::default(),
    }
}

/// Concrete private signatures, independent of the public Action! ABI.
/// MulByte has two byte inputs and a full word output; word arithmetic has
/// a register-pair input and a separately homed zero-page word input.
pub(in crate::mir6502) fn helper_abi_for(helper: MirRuntimeHelper) -> MirCallAbi {
    let mut abi = helper_abi();
    if matches!(helper, MirRuntimeHelper::Fault(_)) {
        abi.params.clear();
        abi.result = None;
        return abi;
    }
    if helper.is_wide() {
        abi.params = [0x82, 0x84, 0xC0, 0xC2].into_iter().map(|low| MirArgHome::BytePair {
            lo: Box::new(MirArgHome::FixedZeroPage(MirFixedZpSlot(low))),
            hi: Box::new(MirArgHome::FixedZeroPage(MirFixedZpSlot(low + 1))),
        }).collect();
        abi.result = Some(MirResultHome::FixedZeroPage(MirFixedZpSlot(0xC4)));
        return abi;
    }
    if helper == MirRuntimeHelper::SArgs {
        return abi;
    }
    abi.params = if matches!(
        helper,
        MirRuntimeHelper::MulByte | MirRuntimeHelper::DivU8 | MirRuntimeHelper::ModU8
    ) {
        vec![MirArgHome::Reg(MirReg::A), MirArgHome::Reg(MirReg::X)]
    } else {
        vec![
            MirArgHome::RegisterPair {
                lo: MirReg::A,
                hi: MirReg::X,
            },
            if matches!(
                helper,
                MirRuntimeHelper::Lsh
                    | MirRuntimeHelper::Rsh
                    | MirRuntimeHelper::DivU16U8
                    | MirRuntimeHelper::ModU16U8
            ) {
                MirArgHome::FixedZeroPage(MirFixedZpSlot(0x84))
            } else {
                MirArgHome::BytePair {
                    lo: Box::new(MirArgHome::FixedZeroPage(MirFixedZpSlot(0x84))),
                    hi: Box::new(MirArgHome::FixedZeroPage(MirFixedZpSlot(0x85))),
                }
            },
        ]
    };
    abi.result = Some(if helper_has_byte_result(helper) {
        MirResultHome::Reg(MirReg::A)
    } else {
        MirResultHome::RegisterPair {
            lo: MirReg::A,
            hi: MirReg::X,
        }
    });
    abi
}

pub(in crate::mir6502) fn helper_has_byte_result(helper: MirRuntimeHelper) -> bool {
    matches!(
        helper,
        MirRuntimeHelper::DivU8 | MirRuntimeHelper::ModU8 | MirRuntimeHelper::ModU16U8
    )
}

pub(in crate::mir6502) fn helper_additional_results(
    helper: MirRuntimeHelper,
) -> Vec<crate::mir6502::ir::MirHelperResult> {
    if helper.is_wide() {
        return vec![crate::mir6502::ir::MirHelperResult {
            home: MirResultHome::FixedZeroPage(MirFixedZpSlot(0xC6)), width: MirWidth::Word,
        }];
    }
    if matches!(helper, MirRuntimeHelper::DivMod | MirRuntimeHelper::UDivMod) {
        vec![crate::mir6502::ir::MirHelperResult {
            home: MirResultHome::FixedZeroPage(MirFixedZpSlot(0x86)),
            width: MirWidth::Word,
        }]
    } else {
        Vec::new()
    }
}

pub(in crate::mir6502) fn helper_args(helper: &MirRuntimeHelper) -> Vec<MirArgHome> {
    // SArgs' descriptor extends these three register lanes at each capture.
    if *helper == MirRuntimeHelper::SArgs {
        return [MirReg::A, MirReg::X, MirReg::Y].into_iter().map(MirArgHome::Reg).collect();
    }
    fn bytes(home: MirArgHome, result: &mut Vec<MirArgHome>) {
        match home {
            MirArgHome::RegisterPair { lo, hi } => result.extend([MirArgHome::Reg(lo), MirArgHome::Reg(hi)]),
            MirArgHome::BytePair { lo, hi } => { bytes(*lo, result); bytes(*hi, result); },
            byte => result.push(byte),
        }
    }
    let mut result = Vec::new();
    for home in helper_abi_for(*helper).params { bytes(home, &mut result); }
    result
}

pub(in crate::mir6502) fn helper_effects(helper: &MirRuntimeHelper) -> MirEffects {
    let mut effects = helper_return_effects(helper);
    if super::super::runtime::requires_error(*helper) {
        effects.memory_reads = MirMemoryEffect::Unknown;
        effects.memory_writes = MirMemoryEffect::Unknown;
        effects.may_call_os = true;
    }
    effects
}

/// Private scratch used by returning computations, distinct from the Error
/// handler's observable effects on a non-returning path.
pub(in crate::mir6502) fn helper_return_effects(helper: &MirRuntimeHelper) -> MirEffects {
    let (memory_reads, memory_writes) = match helper {
        MirRuntimeHelper::Fault(_) => (MirMemoryEffect::Unknown, MirMemoryEffect::Unknown),
        MirRuntimeHelper::Mul32 | MirRuntimeHelper::Div32 | MirRuntimeHelper::Mod32
        | MirRuntimeHelper::UDiv32 | MirRuntimeHelper::UMod32 | MirRuntimeHelper::Lsh32 | MirRuntimeHelper::Rsh32 => (
            zero_page_effect(&[(0x82, 4), (0xC0, 4)]),
            zero_page_effect(&[(0x82, 6), (0xC0, 8)]),
        ),
        MirRuntimeHelper::Lsh | MirRuntimeHelper::Rsh => (
            zero_page_effect(&[(0x84, 1)]),
            zero_page_effect(&[(0x85, 1)]),
        ),
        MirRuntimeHelper::MulByte => (
            zero_page_effect(&[(0x82, 6)]),
            zero_page_effect(&[(0x82, 6)]),
        ),
        MirRuntimeHelper::Mul => (
            zero_page_effect(&[(0x82, 6), (0xc0, 3), (0xc6, 2), (0xd3, 1)]),
            zero_page_effect(&[(0x82, 6), (0xc0, 3), (0xc6, 2), (0xd3, 1)]),
        ),
        MirRuntimeHelper::Div | MirRuntimeHelper::Mod | MirRuntimeHelper::DivMod => (
            zero_page_effect(&[(0x84, 2)]),
            zero_page_effect(&[(0x82, 6), (0xc2, 2)]),
        ),
        MirRuntimeHelper::UDiv | MirRuntimeHelper::UMod | MirRuntimeHelper::UDivMod => (
            zero_page_effect(&[(0x84, 2)]),
            zero_page_effect(&[(0x82, 6)]),
        ),
        MirRuntimeHelper::DivU8 | MirRuntimeHelper::ModU8 => (
            MirMemoryEffect::None,
            zero_page_effect(&[(0x82, 1), (0x84, 1), (0x86, 1)]),
        ),
        MirRuntimeHelper::DivU16U8 | MirRuntimeHelper::ModU16U8 => (
            zero_page_effect(&[(0x84, 1)]),
            zero_page_effect(&[(0x82, 2), (0x86, 1)]),
        ),
        // SArgs also reads the caller's inline descriptor and stack argument
        // area and writes through a descriptor-selected destination. MIR's
        // current effect union cannot retain exact regions alongside an
        // unknown region, so preserve the required conservative boundary.
        MirRuntimeHelper::SArgs => (MirMemoryEffect::Unknown, MirMemoryEffect::Unknown),
    };
    MirEffects {
        memory_reads,
        memory_writes,
        reads: MirRegisterSet::default(),
        clobbers: helper_abi().clobbers,
        preserves: MirRegisterSet::default(),
        stack_depth_delta: Some(0),
        may_call_os: false,
        opaque: false,
    }
}

fn zero_page_effect(ranges: &[(u16, u16)]) -> MirMemoryEffect {
    MirMemoryEffect::Regions(
        ranges
            .iter()
            .map(|(offset, size)| MirMemoryRegion {
                kind: MirMemoryRegionKind::ZeroPage,
                offset: *offset,
                size: *size,
            })
            .collect(),
    )
}

/// Physical primary result, even when the caller does not consume it. Width
/// belongs to the signature; a fixed-ZP address alone is not a byte result.
pub(in crate::mir6502) fn primary_result(helper: MirRuntimeHelper) -> Option<MirHelperResult> {
    if matches!(helper, MirRuntimeHelper::SArgs | MirRuntimeHelper::Fault(_)) { return None; }
    Some(MirHelperResult {
        home: helper_abi_for(helper).result?,
        width: if helper_has_byte_result(helper) { MirWidth::Byte } else { MirWidth::Word },
    })
}

pub(in crate::mir6502) fn result_lanes(helper: MirRuntimeHelper) -> Vec<MirParamLane> {
    primary_result(helper).into_iter().chain(helper_additional_results(helper)).flat_map(|result| {
        let home = match result.home {
            MirResultHome::Reg(reg) => MirArgHome::Reg(reg),
            MirResultHome::RegisterPair { lo, hi } => MirArgHome::RegisterPair { lo, hi },
            MirResultHome::FixedZeroPage(slot) => MirArgHome::FixedZeroPage(slot),
            _ => unreachable!("private helper result must have concrete lanes"),
        };
        crate::mir6502::abi::physical_lanes(&home, result.width).expect("valid private result width")
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::analysis::effects::{classify_op, MirHomeByte};

    #[test]
    fn helper_result_widths_cover_every_wide_lane_and_verifier_rejects_inconsistent_signatures() {
        let mut p = super::super::tests::program("PROC Main() RETURN", crate::runtime::Runtime::ActionCart);
        for helper in [MirRuntimeHelper::Mul32, MirRuntimeHelper::UDiv32, MirRuntimeHelper::Lsh32, MirRuntimeHelper::Rsh32] {
            let result = primary_result(helper).unwrap();
            assert_eq!(result.width, MirWidth::Word);
            assert_eq!(result_lanes(helper), (0xc4..=0xc7).map(|b| MirParamLane::ZeroPage(MirFixedZpSlot(b))).collect::<Vec<_>>());
            let op = MirOp::RuntimeHelper { helper, args: helper_args(&helper), result: Some(result.home),
                additional_results: helper_additional_results(helper), effects: helper_effects(&helper) };
            let effects = classify_op(&op);
            for byte in 0xc4..=0xc7 {
                assert!(effects.homes.definite_writes.contains(&MirHomeByte::FixedZeroPage(MirFixedZpSlot(byte))));
            }
            let mut declaration = crate::mir6502::materialize::deferred_helper_decl(helper);
            declaration.target = MirRuntimeHelperTarget::KnownAbsolute(0x2000);
            p.runtime_helpers = vec![declaration];
            p.routines[0].blocks[0].ops = vec![op];
            crate::mir6502::verify_program(&p, MirPhase::PreEmission).unwrap();
            for variant in 0..5 {
                let mut bad = p.clone();
                let MirOp::RuntimeHelper { args, result, additional_results, effects, .. } = &mut bad.routines[0].blocks[0].ops[0] else { unreachable!() };
                match variant {
                    0 => additional_results[0].width = MirWidth::Byte,
                    1 => *result = Some(MirResultHome::Reg(MirReg::Y)),
                    2 => { args.pop(); },
                    3 => effects.clobbers = MirRegisterSet::default(),
                    _ => bad.runtime_helpers[0].abi.result = None,
                }
                assert!(crate::mir6502::verify_program(&bad, MirPhase::PreEmission).is_err(), "{helper:?} {variant}");
            }
        }
        assert_eq!(primary_result(MirRuntimeHelper::ModU16U8).unwrap().width, MirWidth::Byte);
        assert!(primary_result(MirRuntimeHelper::SArgs).is_none());
        assert_eq!(result_lanes(MirRuntimeHelper::UDivMod).len(), 4);
    }
}
