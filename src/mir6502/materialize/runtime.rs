use super::layout::MaterializeLayout;
use super::values::{split_def, split_value_with_temp_widths};
use crate::mir6502::analysis::use_def::{MirTempLane, MirTempUseDefIndex};
use crate::mir6502::ir::{
    MirAddr, MirArgHome, MirBinaryOp, MirCallAbi, MirDef, MirEffects, MirFixedZpSlot, MirMem,
    MirMemoryEffect, MirMemoryRegion, MirMemoryRegionKind, MirOp, MirProgram, MirReg,
    MirRegisterSet, MirResultHome, MirRuntimeHelper, MirRuntimeHelperDecl, MirRuntimeHelperTarget,
    MirTempId, MirValue, MirWidth,
};
use std::collections::BTreeMap;

pub(super) fn ensure_helper_decl(program: &mut MirProgram, helper: MirRuntimeHelper) {
    if program
        .runtime_helpers
        .iter()
        .any(|decl| decl.helper == helper)
    {
        return;
    }
    program.runtime_helpers.push(deferred_helper_decl(helper));
}

pub(in crate::mir6502) fn deferred_helper_decl(helper: MirRuntimeHelper) -> MirRuntimeHelperDecl {
    MirRuntimeHelperDecl {
        // Helper discovery records a logical requirement.  The selected
        // runtime owns the later physical binding.
        target: MirRuntimeHelperTarget::Deferred,
        effects: helper_effects(&helper),
        helper,
        abi: helper_abi_for(helper),
        additional_results: helper_additional_results(helper),
    }
}

pub(in crate::mir6502) fn helper_abi() -> MirCallAbi {
    MirCallAbi {
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

fn helper_has_byte_result(helper: MirRuntimeHelper) -> bool {
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
    let mut args = vec![MirArgHome::Reg(MirReg::A), MirArgHome::Reg(MirReg::X)];
    match helper {
        MirRuntimeHelper::Fault(_) => return Vec::new(),
        MirRuntimeHelper::Mul32 | MirRuntimeHelper::Div32 | MirRuntimeHelper::Mod32
        | MirRuntimeHelper::UDiv32 | MirRuntimeHelper::UMod32 | MirRuntimeHelper::Lsh32 | MirRuntimeHelper::Rsh32 => {
            return (0x82..=0x85).chain(0xC0..=0xC3)
                .map(|byte| MirArgHome::FixedZeroPage(MirFixedZpSlot(byte))).collect();
        }
        MirRuntimeHelper::MulByte | MirRuntimeHelper::DivU8 | MirRuntimeHelper::ModU8 => {}
        MirRuntimeHelper::Mul
        | MirRuntimeHelper::Div
        | MirRuntimeHelper::Mod
        | MirRuntimeHelper::UDiv
        | MirRuntimeHelper::UMod
        | MirRuntimeHelper::DivMod
        | MirRuntimeHelper::UDivMod => {
            args.extend([
                MirArgHome::FixedZeroPage(MirFixedZpSlot(0x84)),
                MirArgHome::FixedZeroPage(MirFixedZpSlot(0x85)),
            ]);
        }
        MirRuntimeHelper::Lsh
        | MirRuntimeHelper::Rsh
        | MirRuntimeHelper::DivU16U8
        | MirRuntimeHelper::ModU16U8 => {
            args.push(MirArgHome::FixedZeroPage(MirFixedZpSlot(0x84)));
        }
        MirRuntimeHelper::SArgs => {
            args.push(MirArgHome::Reg(MirReg::Y));
        }
    }
    args
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

pub(in crate::mir6502) fn helper_for_binary(
    op: MirBinaryOp,
    width: MirWidth,
) -> Option<MirRuntimeHelper> {
    match (op, width) {
        (MirBinaryOp::Mul, _) => Some(MirRuntimeHelper::Mul),
        (MirBinaryOp::Div, _) => Some(MirRuntimeHelper::Div),
        (MirBinaryOp::Mod, _) => Some(MirRuntimeHelper::Mod),
        (MirBinaryOp::UDiv, _) => Some(MirRuntimeHelper::UDiv),
        (MirBinaryOp::UMod, _) => Some(MirRuntimeHelper::UMod),
        (MirBinaryOp::Lsh | MirBinaryOp::Rsh, MirWidth::Word) => match op {
            MirBinaryOp::Lsh => Some(MirRuntimeHelper::Lsh),
            MirBinaryOp::Rsh => Some(MirRuntimeHelper::Rsh),
            _ => None,
        },
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::mir6502) struct MirRuntimeBinarySelection {
    pub helper: MirRuntimeHelper,
    pub input_widths: [MirWidth; 2],
    pub result_width: MirWidth,
}

pub(in crate::mir6502) fn helper_for_typed_binary(
    op: MirBinaryOp,
    width: MirWidth,
    left: &MirValue,
    right: &MirValue,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    allow_widening_byte_multiply: bool,
    allow_narrow_division: bool,
    linked_helpers: &[MirRuntimeHelper],
) -> Option<MirRuntimeBinarySelection> {
    if allow_widening_byte_multiply
        && op == MirBinaryOp::Mul
        && width == MirWidth::Word
        && value_is_known_byte(left, temp_widths)
        && value_is_known_byte(right, temp_widths)
    {
        return Some(MirRuntimeBinarySelection {
            helper: MirRuntimeHelper::MulByte,
            input_widths: [MirWidth::Byte; 2],
            result_width: MirWidth::Word,
        });
    }
    let helper = helper_for_binary(op, width)?;
    let general = MirRuntimeBinarySelection {
        helper,
        input_widths: [width; 2],
        result_width: width,
    };
    if allow_narrow_division
        && matches!(op, MirBinaryOp::UDiv | MirBinaryOp::UMod)
        && value_is_known_byte(right, temp_widths)
    {
        let byte_left = value_is_known_byte(left, temp_widths);
        let candidate = MirRuntimeBinarySelection {
            helper: match (op, byte_left) {
                (MirBinaryOp::UDiv, true) => MirRuntimeHelper::DivU8,
                (MirBinaryOp::UMod, true) => MirRuntimeHelper::ModU8,
                (MirBinaryOp::UDiv, false) => MirRuntimeHelper::DivU16U8,
                _ => MirRuntimeHelper::ModU16U8,
            },
            input_widths: [
                if byte_left {
                    MirWidth::Byte
                } else {
                    MirWidth::Word
                },
                MirWidth::Byte,
            ],
            result_width: width,
        };
        if division_selection_cost(candidate, linked_helpers)
            < division_selection_cost(general, linked_helpers)
        {
            return Some(candidate);
        }
    }
    Some(general)
}

/// A bounded speed/size policy: eight worst-case cycles buy one code byte.
/// Charge the actual helper body only when it has not already been linked;
/// staging and result adaptation are charged at every call site. The cycle
/// bounds include the fixed 8/16-iteration kernels and their return paths.
fn division_selection_cost(
    selection: MirRuntimeBinarySelection,
    linked: &[MirRuntimeHelper],
) -> usize {
    let (body, cycles) = match selection.helper {
        MirRuntimeHelper::DivU8 | MirRuntimeHelper::ModU8 => (
            crate::integer6502::narrow_division_body(
                false,
                selection.helper == MirRuntimeHelper::ModU8,
            )
            .len()
                + 1,
            500,
        ),
        MirRuntimeHelper::DivU16U8 | MirRuntimeHelper::ModU16U8 => (
            crate::integer6502::narrow_division_body(
                true,
                selection.helper == MirRuntimeHelper::ModU16U8,
            )
            .len()
                + 1,
            1000,
        ),
        MirRuntimeHelper::UDiv | MirRuntimeHelper::UMod => (
            crate::integer6502::division_body(false, selection.helper == MirRuntimeHelper::UMod)
                .len()
                + 1,
            1400,
        ),
        MirRuntimeHelper::Div | MirRuntimeHelper::Mod => (
            crate::integer6502::division_body(true, selection.helper == MirRuntimeHelper::Mod)
                .len()
                + 1,
            1700,
        ),
        MirRuntimeHelper::DivMod | MirRuntimeHelper::UDivMod => (
            crate::integer6502::divmod_body(selection.helper == MirRuntimeHelper::DivMod).len() + 1,
            if selection.helper == MirRuntimeHelper::DivMod {
                1800
            } else {
                1400
            },
        ),
        _ => unreachable!("only unsigned division candidates are costed here"),
    };
    let lanes: usize = selection
        .input_widths
        .iter()
        .map(|width| if *width == MirWidth::Byte { 1 } else { 2 })
        .sum();
    let adaptation = usize::from(
        helper_has_byte_result(selection.helper) && selection.result_width == MirWidth::Word,
    ) * 2;
    usize::from(!linked.contains(&selection.helper)) * body
        + 3
        + lanes * 5
        + adaptation
        + (cycles + lanes * 7 + 6) / 8
}

/// Fuse only adjacent operations on already captured SSA values. This first
/// implementation intentionally does not cross loads, stores, calls or CFG
/// edges, and never equates a pointer-cell reread with a captured value.
pub(super) fn fuse_adjacent_divmod(
    routine: &mut crate::mir6502::ir::MirRoutine,
    config: &crate::mir6502::Mir6502Config,
    layout: &MaterializeLayout,
    helpers: &mut Vec<MirRuntimeHelper>,
) -> usize {
    if !config.select_runtime_helpers || !config.enable_peepholes {
        return 0;
    }
    let widths = super::temp_widths::collect_routine_temp_widths(routine);
    let use_def = MirTempUseDefIndex::from_routine(routine);
    let captured = |value: &MirValue| single_captured_value(value, &use_def);
    let mut selected = 0;
    for block in &mut routine.blocks {
        let ops = std::mem::take(&mut block.ops);
        let mut index = 0;
        while index < ops.len() {
            let pair = match (ops.get(index), ops.get(index + 1)) {
                (
                    Some(MirOp::Binary {
                        op: a,
                        dst: ad,
                        left: al,
                        right: ar,
                        width: aw,
                        carry_in: ai,
                        carry_out: ac,
                    }),
                    Some(MirOp::Binary {
                        op: b,
                        dst: bd,
                        left: bl,
                        right: br,
                        width: bw,
                        carry_in: bi,
                        carry_out: bc,
                    }),
                ) if aw == bw
                    && al == bl
                    && ar == br
                    && captured(al)
                    && captured(ar)
                    && ad != bd
                    && captured(&MirValue::Def(ad.clone()))
                    && captured(&MirValue::Def(bd.clone()))
                    && ai.is_none()
                    && bi.is_none()
                    && *ac == crate::mir6502::ir::MirCarryOut::Ignore
                    && *bc == crate::mir6502::ir::MirCarryOut::Ignore
                    && matches!(ad, MirDef::VTemp(_))
                    && matches!(bd, MirDef::VTemp(_))
                    && matches!(
                        (a, b),
                        (MirBinaryOp::Div, MirBinaryOp::Mod)
                            | (MirBinaryOp::Mod, MirBinaryOp::Div)
                            | (MirBinaryOp::UDiv, MirBinaryOp::UMod)
                            | (MirBinaryOp::UMod, MirBinaryOp::UDiv)
                    ) =>
                {
                    Some((*a, *b, ad.clone(), bd.clone(), al.clone(), ar.clone(), *aw))
                }
                _ => None,
            };
            if let Some((a, b, ad, bd, left, right, width)) = pair {
                let helper = if matches!(a, MirBinaryOp::Div | MirBinaryOp::Mod) {
                    MirRuntimeHelper::DivMod
                } else {
                    MirRuntimeHelper::UDivMod
                };
                let pair_selection = MirRuntimeBinarySelection {
                    helper,
                    input_widths: [width; 2],
                    result_width: width,
                };
                let first =
                    helper_for_typed_binary(a, width, &left, &right, &widths, false, true, helpers)
                        .unwrap();
                let second =
                    helper_for_typed_binary(b, width, &left, &right, &widths, false, true, helpers)
                        .unwrap();
                if division_selection_cost(pair_selection, helpers) + 8
                    < division_selection_cost(first, helpers)
                        + division_selection_cost(second, helpers)
                {
                    let (q, r) = if matches!(a, MirBinaryOp::Div | MirBinaryOp::UDiv) {
                        (ad, bd)
                    } else {
                        (bd, ad)
                    };
                    materialize_runtime_helper_binary(
                        helper,
                        None,
                        left,
                        right,
                        [width; 2],
                        width,
                        layout,
                        &widths,
                        &mut block.ops,
                    );
                    materialize_runtime_helper_result(q, width, &mut block.ops);
                    block.ops.push(MirOp::Load {
                        dst: r,
                        src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0x86))),
                        width,
                    });
                    helpers.push(helper);
                    selected += 1;
                    index += 2;
                    continue;
                }
            }
            block.ops.push(ops[index].clone());
            index += 1;
        }
    }
    selected
}

/// Expose an immediately preceding zero-extension as explicit word lanes.
/// This preserves the captured byte (never substitutes a memory reread), keeps
/// the original definition for other users, and makes existing byte-range and
/// helper rewrite proofs applicable without a second range-analysis system.
pub(super) fn expose_zero_extended_binary_inputs(routine: &mut crate::mir6502::ir::MirRoutine) {
    let use_def = MirTempUseDefIndex::from_routine(routine);
    for block in &mut routine.blocks {
        let mut extended = BTreeMap::new();
        for op in &mut block.ops {
            match op {
                MirOp::Extend {
                    dst: MirDef::VTemp(id),
                    src,
                    from_width: MirWidth::Byte,
                    to_width: MirWidth::Word,
                    signed: false,
                } if single_captured_value(src, &use_def)
                    && single_captured_value(&MirValue::Def(MirDef::VTemp(*id)), &use_def) =>
                {
                    extended.insert(
                        *id,
                        MirValue::Word {
                            lo: Box::new(src.clone()),
                            hi: Box::new(MirValue::ConstU8(0)),
                        },
                    );
                }
                MirOp::Binary {
                    op: MirBinaryOp::UDiv | MirBinaryOp::UMod,
                    left,
                    right,
                    width: MirWidth::Word,
                    ..
                } => {
                    for value in [left, right] {
                        if let MirValue::Def(MirDef::VTemp(id)) = value {
                            if let Some(replacement) = extended.get(id) {
                                *value = replacement.clone();
                            }
                        }
                    }
                    extended.clear();
                }
                _ => extended.clear(),
            }
        }
    }
}

fn single_captured_value(value: &MirValue, index: &MirTempUseDefIndex) -> bool {
    match value {
        MirValue::ConstU8(_) | MirValue::ConstU16(_) => true,
        MirValue::Def(MirDef::VTemp(id)) => {
            let mut definitions = index.definitions_of_temp(*id);
            definitions
                .next()
                .is_some_and(|first| definitions.all(|other| other.site == first.site))
        }
        MirValue::Def(MirDef::VTempByte { id, byte }) => index
            .unique_definition(MirTempLane {
                temp: *id,
                byte: *byte,
            })
            .is_some(),
        MirValue::Word { lo, hi } => {
            single_captured_value(lo, index) && single_captured_value(hi, index)
        }
        _ => false,
    }
}

fn value_is_known_byte(value: &MirValue, temp_widths: &BTreeMap<MirTempId, MirWidth>) -> bool {
    match value {
        MirValue::ConstU8(_) => true,
        MirValue::ConstU16(value) => u8::try_from(*value).is_ok(),
        MirValue::Def(MirDef::VTemp(id)) => temp_widths.get(id) == Some(&MirWidth::Byte),
        MirValue::Def(MirDef::VTempByte { .. }) => true,
        MirValue::Def(MirDef::Reg(_))
        | MirValue::StaticAddr(_)
        | MirValue::GlobalAddr(_)
        | MirValue::RoutineAddr(_)
        | MirValue::RoutineAddrByte { .. }
        | MirValue::StorageAddrByte { .. }
        | MirValue::PointerCell(_) => false,
        MirValue::Word { hi, .. } => {
            matches!(hi.as_ref(), MirValue::ConstU8(0) | MirValue::ConstU16(0))
        }
    }
}

/// Semantic validation shared with the rewrite driver. Cost determines which
/// eligible helper to choose; it cannot make an ineligible signature valid.
pub(in crate::mir6502) fn helper_implements_binary(
    helper: MirRuntimeHelper,
    op: MirBinaryOp,
    width: MirWidth,
    left: &MirValue,
    right: &MirValue,
    widths: &BTreeMap<MirTempId, MirWidth>,
) -> bool {
    if helper_for_binary(op, width) == Some(helper) {
        return true;
    }
    match helper {
        MirRuntimeHelper::MulByte => {
            op == MirBinaryOp::Mul
                && width == MirWidth::Word
                && value_is_known_byte(left, widths)
                && value_is_known_byte(right, widths)
        }
        MirRuntimeHelper::DivU8 | MirRuntimeHelper::ModU8 => {
            op == if helper == MirRuntimeHelper::DivU8 {
                MirBinaryOp::UDiv
            } else {
                MirBinaryOp::UMod
            } && value_is_known_byte(left, widths)
                && value_is_known_byte(right, widths)
        }
        MirRuntimeHelper::DivU16U8 | MirRuntimeHelper::ModU16U8 => {
            op == if helper == MirRuntimeHelper::DivU16U8 {
                MirBinaryOp::UDiv
            } else {
                MirBinaryOp::UMod
            } && width == MirWidth::Word
                && value_is_known_byte(right, widths)
        }
        _ => false,
    }
}

pub(super) fn materialize_runtime_helper_binary(
    helper: MirRuntimeHelper,
    dst: Option<MirDef>,
    left: MirValue,
    right: MirValue,
    input_widths: [MirWidth; 2],
    result_width: MirWidth,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
    out: &mut Vec<MirOp>,
) {
    let (left_lo, left_hi) = split_value_for_width(left, input_widths[0], layout, temp_widths);
    let (right_lo, right_hi) = split_value_for_width(right, input_widths[1], layout, temp_widths);

    match helper {
        MirRuntimeHelper::MulByte | MirRuntimeHelper::DivU8 | MirRuntimeHelper::ModU8 => {}
        MirRuntimeHelper::Mul
        | MirRuntimeHelper::Div
        | MirRuntimeHelper::Mod
        | MirRuntimeHelper::UDiv
        | MirRuntimeHelper::UMod
        | MirRuntimeHelper::DivMod
        | MirRuntimeHelper::UDivMod => {
            materialize_helper_arg_to_mem(
                right_lo.clone(),
                MirMem::FixedZeroPage(MirFixedZpSlot(0x84)),
                out,
            );
            materialize_helper_arg_to_mem(
                right_hi,
                MirMem::FixedZeroPage(MirFixedZpSlot(0x85)),
                out,
            );
        }
        MirRuntimeHelper::Lsh
        | MirRuntimeHelper::Rsh
        | MirRuntimeHelper::DivU16U8
        | MirRuntimeHelper::ModU16U8 => {
            materialize_helper_arg_to_mem(
                right_lo.clone(),
                MirMem::FixedZeroPage(MirFixedZpSlot(0x84)),
                out,
            );
        }
        MirRuntimeHelper::SArgs => {}
        MirRuntimeHelper::Fault(_) => unreachable!("fault has no binary operands"),
        MirRuntimeHelper::Mul32 | MirRuntimeHelper::Div32 | MirRuntimeHelper::Mod32
        | MirRuntimeHelper::UDiv32 | MirRuntimeHelper::UMod32 | MirRuntimeHelper::Lsh32 | MirRuntimeHelper::Rsh32 => {
            unreachable!("wide helper operands are legalized as word lanes")
        }
    }

    materialize_helper_arg_to_reg(left_lo, MirReg::A, out);
    if matches!(
        helper,
        MirRuntimeHelper::MulByte | MirRuntimeHelper::DivU8 | MirRuntimeHelper::ModU8
    ) {
        materialize_helper_arg_to_reg(right_lo, MirReg::X, out);
    } else {
        materialize_helper_arg_to_reg(left_hi, MirReg::X, out);
    }
    let effects = helper_effects(&helper);
    let args = helper_args(&helper);
    out.push(MirOp::RuntimeHelper {
        helper,
        args,
        result: None,
        additional_results: helper_additional_results(helper),
        effects,
    });
    if helper_has_byte_result(helper) && result_width == MirWidth::Word {
        // The physical result is A only. Do not depend on an undocumented X
        // value when adapting a byte remainder to a word source result.
        out.push(MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::X),
            value: 0,
            width: MirWidth::Byte,
        });
    }
    if let Some(dst) = dst {
        materialize_runtime_helper_result(dst, result_width, out);
    }
}

fn materialize_helper_arg_to_reg(value: MirValue, reg: MirReg, out: &mut Vec<MirOp>) {
    match value {
        MirValue::PointerCell(mem) => out.push(MirOp::Load {
            dst: MirDef::Reg(reg),
            src: MirAddr::Direct(mem),
            width: MirWidth::Byte,
        }),
        value => out.push(MirOp::Move {
            dst: MirDef::Reg(reg),
            src: value,
            width: MirWidth::Byte,
        }),
    }
}

fn materialize_helper_arg_to_mem(value: MirValue, dst: MirMem, out: &mut Vec<MirOp>) {
    let src = match value {
        MirValue::PointerCell(mem) => {
            out.push(MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(mem),
                width: MirWidth::Byte,
            });
            MirValue::Def(MirDef::Reg(MirReg::A))
        }
        value => value,
    };
    out.push(MirOp::Store {
        dst: MirAddr::Direct(dst),
        src,
        width: MirWidth::Byte,
    });
}

fn split_value_for_width(
    value: MirValue,
    width: MirWidth,
    layout: &MaterializeLayout,
    temp_widths: &BTreeMap<MirTempId, MirWidth>,
) -> (MirValue, MirValue) {
    match width {
        MirWidth::Byte => (
            match value {
                MirValue::Word { lo, .. } => *lo,
                MirValue::ConstU16(bits) => MirValue::ConstU8(bits as u8),
                value => value,
            },
            MirValue::ConstU8(0),
        ),
        MirWidth::Word => split_value_with_temp_widths(value, layout, temp_widths),
    }
}

pub(super) fn runtime_helper_result_width(
    helper: &MirRuntimeHelper,
    width: MirWidth,
    dst: &MirDef,
) -> MirWidth {
    match (helper, width) {
        (MirRuntimeHelper::MulByte, _) => MirWidth::Word,
        (MirRuntimeHelper::Mul, MirWidth::Byte) if split_def(dst.clone()).is_some() => {
            MirWidth::Word
        }
        _ => width,
    }
}

fn materialize_runtime_helper_result(dst: MirDef, width: MirWidth, out: &mut Vec<MirOp>) {
    match width {
        MirWidth::Byte => out.push(MirOp::Move {
            dst,
            src: MirValue::Def(MirDef::Reg(MirReg::A)),
            width: MirWidth::Byte,
        }),
        MirWidth::Word => {
            if let Some((lo_dst, hi_dst)) = split_def(dst.clone()) {
                out.push(MirOp::Move {
                    dst: lo_dst,
                    src: MirValue::Def(MirDef::Reg(MirReg::A)),
                    width: MirWidth::Byte,
                });
                out.push(MirOp::Move {
                    dst: hi_dst,
                    src: MirValue::Def(MirDef::Reg(MirReg::X)),
                    width: MirWidth::Byte,
                });
            } else {
                out.push(MirOp::Move {
                    dst,
                    src: MirValue::Word {
                        lo: Box::new(MirValue::Def(MirDef::Reg(MirReg::A))),
                        hi: Box::new(MirValue::Def(MirDef::Reg(MirReg::X))),
                    },
                    width: MirWidth::Word,
                });
            }
        }
    }
}
