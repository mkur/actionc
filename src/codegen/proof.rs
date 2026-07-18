#![allow(dead_code)]

use super::temp::{
    VirtualTempHome, VirtualTempWidth, ZeroPageTempCandidate, ZeroPageTempPool,
    storage_slots_overlap, zero_page_temp_survives_effects,
};
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ExpressionSideEffectFacts {
    pub(super) has_routine_call: bool,
    pub(super) has_unknown_raw: bool,
    pub(super) reads_memory: bool,
    pub(super) reads_pointer: bool,
    pub(super) reads_volatile: bool,
    pub(super) writes_through_pointer: bool,
    pub(super) evaluation_order_sensitive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ValueRangeFact {
    Unknown,
    Byte,
    Exact(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IndexAddressMode {
    AbsoluteY,
    IndirectY,
    ScaledIndirectY,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IndexAddressRejectReason {
    IndexHasSideEffects,
    NonByteIndex,
    ElementNeedsScaling,
    UnsupportedBase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct IndexAddressProof {
    pub(super) base: StorageSlot,
    pub(super) element_size: u16,
    pub(super) index_width: Option<u16>,
    pub(super) index_range: ValueRangeFact,
    pub(super) effects: ExpressionSideEffectFacts,
    pub(super) mode: IndexAddressMode,
    pub(super) reject_reason: Option<IndexAddressRejectReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PointerDereferenceKind {
    Direct,
    Indexed,
    RecordField,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PointerDereferenceMode {
    IndirectY,
    IndirectYWithOffset,
    NeedsAddressArithmetic,
    ScaledIndirectY,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PointerDereferenceRejectReason {
    NotPointer,
    UnknownRecordField,
    FieldOffsetTooWide,
    IndexHasSideEffects,
    NonByteIndex,
    ElementNeedsScaling,
    UnsupportedShape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PointerDereferenceProof {
    pub(super) pointer: StorageSlot,
    pub(super) kind: PointerDereferenceKind,
    pub(super) pointee_size: u16,
    pub(super) signed: bool,
    pub(super) field: Option<RecordField>,
    pub(super) index: Option<IndexAddressProof>,
    pub(super) mode: PointerDereferenceMode,
    pub(super) reject_reason: Option<PointerDereferenceRejectReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ValueByteAvailability {
    Constant(u8),
    Slot { slot: StorageSlot, byte_index: u16 },
    Register(RegisterName),
    PublicReturnSlot { slot: StorageSlot, byte_index: u16 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ValueAvailabilitySource {
    Constant,
    Storage,
    RoutineReturn,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ValueAvailabilityProof {
    pub(super) width: Option<u16>,
    pub(super) source: ValueAvailabilitySource,
    pub(super) bytes: [Option<ValueByteAvailability>; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RoutineVisibilityFacts {
    pub(super) retargetable: bool,
    pub(super) address_taken: bool,
    pub(super) internal_only_candidate: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RoutineBoundaryKind {
    System,
    Retargetable,
    InternalCandidate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RoutineBoundaryProof {
    pub(super) name: String,
    pub(super) kind: RoutineBoundaryKind,
    pub(super) system_address: Option<u16>,
    pub(super) retargetable: bool,
    pub(super) address_taken: bool,
    pub(super) internal_only_candidate: bool,
    pub(super) public_entry_required: bool,
    pub(super) patchable_entry_required: bool,
    pub(super) internal_abi_candidate: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CallBoundaryProof {
    pub(super) temp_home: VirtualTempHome,
    pub(super) callee_effects: RoutineEffects,
    pub(super) survives: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ZeroPageTempLifetimeProof {
    pub(super) temp_home: VirtualTempHome,
    pub(super) calls_crossed: usize,
    pub(super) survives_all_calls: bool,
    pub(super) first_blocking_call: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ZeroPageTempPlacementProof {
    pub(super) width: VirtualTempWidth,
    pub(super) candidate: Option<ZeroPageTempCandidate>,
    pub(super) blocked_by_occupied_slot: bool,
}

impl ExpressionSideEffectFacts {
    pub(super) fn pure() -> Self {
        Self {
            has_routine_call: false,
            has_unknown_raw: false,
            reads_memory: false,
            reads_pointer: false,
            reads_volatile: false,
            writes_through_pointer: false,
            evaluation_order_sensitive: false,
        }
    }

    pub(super) fn routine_call() -> Self {
        Self {
            has_routine_call: true,
            has_unknown_raw: false,
            reads_memory: false,
            reads_pointer: false,
            reads_volatile: false,
            writes_through_pointer: false,
            evaluation_order_sensitive: true,
        }
    }

    pub(super) fn unknown_raw() -> Self {
        Self {
            has_routine_call: false,
            has_unknown_raw: true,
            reads_memory: false,
            reads_pointer: false,
            reads_volatile: false,
            writes_through_pointer: false,
            evaluation_order_sensitive: true,
        }
    }

    pub(super) fn merge(self, other: Self) -> Self {
        Self {
            has_routine_call: self.has_routine_call || other.has_routine_call,
            has_unknown_raw: self.has_unknown_raw || other.has_unknown_raw,
            reads_memory: self.reads_memory || other.reads_memory,
            reads_pointer: self.reads_pointer || other.reads_pointer,
            reads_volatile: self.reads_volatile || other.reads_volatile,
            writes_through_pointer: self.writes_through_pointer || other.writes_through_pointer,
            evaluation_order_sensitive: self.evaluation_order_sensitive
                || other.evaluation_order_sensitive,
        }
    }

    pub(super) fn is_read_only(self) -> bool {
        !self.has_routine_call && !self.has_unknown_raw && !self.writes_through_pointer
    }

    pub(super) fn can_duplicate(self) -> bool {
        self.is_read_only() && !self.reads_volatile && !self.evaluation_order_sensitive
    }

    pub(super) fn can_reorder(self) -> bool {
        self.can_duplicate() && !self.reads_pointer
    }
}

impl ValueRangeFact {
    pub(super) fn is_byte(self) -> bool {
        matches!(self, Self::Byte | Self::Exact(0..=0xFF))
    }
}

impl Generator {
    pub(super) fn expr_side_effect_facts(&self, expr: &Expr) -> ExpressionSideEffectFacts {
        match &expr.kind {
            ExprKind::Missing | ExprKind::Raw => ExpressionSideEffectFacts::unknown_raw(),
            ExprKind::CurrentLocation
            | ExprKind::Number(_)
            | ExprKind::String(_)
            | ExprKind::Char(_) => ExpressionSideEffectFacts::pure(),
            ExprKind::Cast { expr, .. } => self.expr_side_effect_facts(expr),
            ExprKind::Name(name) => {
                let mut facts = ExpressionSideEffectFacts::pure();
                if let Some(slot) = self.lookup_slot(name) {
                    facts.reads_memory = true;
                    facts.reads_volatile = storage_name_is_volatile(name, slot);
                }
                facts
            }
            ExprKind::Unary {
                op: UnaryOp::Deref,
                expr,
            } => {
                let mut facts = self.expr_side_effect_facts(expr);
                facts.reads_memory = true;
                facts.reads_pointer = true;
                facts
            }
            ExprKind::Unary { expr, .. } => self.expr_side_effect_facts(expr),
            ExprKind::Field { base, .. } => {
                let mut facts = self.expr_side_effect_facts(base);
                facts.reads_memory = true;
                if let ExprKind::Name(name) = &base.kind
                    && self
                        .lookup_slot(name)
                        .is_some_and(|slot| slot.pointee_size.is_some())
                {
                    facts.reads_pointer = true;
                }
                facts
            }
            ExprKind::Binary { left, right, .. } => {
                let left_facts = self.expr_side_effect_facts(left);
                let right_facts = self.expr_side_effect_facts(right);
                let mut facts = left_facts.merge(right_facts);
                if left_facts.has_routine_call || right_facts.has_routine_call {
                    facts.evaluation_order_sensitive = true;
                }
                facts
            }
            ExprKind::Index { base, index } => self
                .expr_side_effect_facts(base)
                .merge(self.expr_side_effect_facts(index)),
            ExprKind::Call { callee, args } => {
                let nested = args
                    .iter()
                    .fold(self.expr_side_effect_facts(callee), |facts, arg| {
                        facts.merge(self.expr_side_effect_facts(arg))
                    });
                if self.array_call_slot_size(callee, args).is_some() {
                    let mut facts = nested;
                    facts.reads_memory = true;
                    facts.reads_pointer = array_call_reads_through_pointer(self, callee, args);
                    facts
                } else {
                    nested.merge(ExpressionSideEffectFacts::routine_call())
                }
            }
        }
    }

    pub(super) fn expr_value_range_fact(&self, expr: &Expr) -> ValueRangeFact {
        if let Some(value) = self.constant_u16(expr) {
            return ValueRangeFact::Exact(value);
        }
        if let ExprKind::Binary {
            op: BinaryOp::And,
            left,
            right,
        } = &expr.kind
            && self
                .constant_u16(left)
                .or_else(|| self.constant_u16(right))
                .is_some_and(|mask| mask <= u16::from(u8::MAX))
        {
            return ValueRangeFact::Byte;
        }
        match self.expr_size(expr) {
            Some(1) => ValueRangeFact::Byte,
            _ => ValueRangeFact::Unknown,
        }
    }

    pub(super) fn index_address_proof(&self, expr: &Expr) -> Option<IndexAddressProof> {
        let (base, index, element_size) = match &expr.kind {
            ExprKind::Index { base, index } => {
                let ExprKind::Name(name) = &base.kind else {
                    return None;
                };
                let slot = self.lookup_slot(name)?;
                (slot, index.as_ref(), slot.size)
            }
            ExprKind::Call { callee, args } if args.len() == 1 => {
                let ExprKind::Name(name) = &callee.kind else {
                    return None;
                };
                let slot = self.lookup_slot(name)?;
                let element_size = self.array_call_slot_size(callee, args)?;
                (slot, &args[0], element_size)
            }
            _ => return None,
        };

        let effects = self.expr_side_effect_facts(index);
        let index_width = self.expr_size(index);
        let index_range = self.expr_value_range_fact(index);
        let byte_index = index_width == Some(1) || index_range.is_byte();
        let (mode, reject_reason) = if !effects.is_read_only() || effects.reads_volatile {
            (
                IndexAddressMode::Unsupported,
                Some(IndexAddressRejectReason::IndexHasSideEffects),
            )
        } else if !byte_index {
            (
                IndexAddressMode::Unsupported,
                Some(IndexAddressRejectReason::NonByteIndex),
            )
        } else {
            match (element_size, base.array, base.pointee_size) {
                (1, Some(ArrayStorage::Inline), _) => (IndexAddressMode::AbsoluteY, None),
                (1, Some(ArrayStorage::Pointer | ArrayStorage::Descriptor), _)
                | (1, None, Some(_)) => (IndexAddressMode::IndirectY, None),
                (
                    2,
                    Some(ArrayStorage::Inline | ArrayStorage::Pointer | ArrayStorage::Descriptor),
                    _,
                )
                | (2, None, Some(_)) => (IndexAddressMode::ScaledIndirectY, None),
                (_, Some(_), _) | (_, None, Some(_)) => (
                    IndexAddressMode::Unsupported,
                    Some(IndexAddressRejectReason::ElementNeedsScaling),
                ),
                _ => (
                    IndexAddressMode::Unsupported,
                    Some(IndexAddressRejectReason::UnsupportedBase),
                ),
            }
        };

        Some(IndexAddressProof {
            base,
            element_size,
            index_width,
            index_range,
            effects,
            mode,
            reject_reason,
        })
    }

    pub(super) fn pointer_dereference_proof(&self, expr: &Expr) -> Option<PointerDereferenceProof> {
        match &expr.kind {
            ExprKind::Unary {
                op: UnaryOp::Deref,
                expr,
            } => {
                let ExprKind::Name(name) = &expr.kind else {
                    return None;
                };
                let pointer = self.lookup_slot(name)?;
                let Some(pointee_size) = pointer.pointee_size else {
                    return Some(PointerDereferenceProof::rejected(
                        pointer,
                        PointerDereferenceKind::Direct,
                        0,
                        pointer.signed,
                        PointerDereferenceRejectReason::NotPointer,
                    ));
                };
                Some(PointerDereferenceProof {
                    pointer,
                    kind: PointerDereferenceKind::Direct,
                    pointee_size,
                    signed: pointer.signed,
                    field: None,
                    index: None,
                    mode: PointerDereferenceMode::IndirectY,
                    reject_reason: None,
                })
            }
            ExprKind::Call { callee, args } if args.len() == 1 => {
                let ExprKind::Name(name) = &callee.kind else {
                    return None;
                };
                let pointer = self.lookup_slot(name)?;
                let pointee_size = pointer.pointee_size?;
                let index = self.index_address_proof(expr)?;
                let (mode, reject_reason) = pointer_index_mode_from_index_proof(index);
                Some(PointerDereferenceProof {
                    pointer,
                    kind: PointerDereferenceKind::Indexed,
                    pointee_size,
                    signed: pointer.signed,
                    field: None,
                    index: Some(index),
                    mode,
                    reject_reason,
                })
            }
            ExprKind::Field { base, field } => {
                let ExprKind::Name(name) = &base.kind else {
                    return None;
                };
                let pointer = self.lookup_slot(name)?;
                pointer.pointee_size?;
                let Some(record_id) = pointer.record else {
                    return Some(PointerDereferenceProof::rejected(
                        pointer,
                        PointerDereferenceKind::RecordField,
                        pointer.pointee_size.unwrap_or_default(),
                        pointer.signed,
                        PointerDereferenceRejectReason::UnknownRecordField,
                    ));
                };
                let Some(field) = self.record_layouts.field(record_id, field) else {
                    return Some(PointerDereferenceProof::rejected(
                        pointer,
                        PointerDereferenceKind::RecordField,
                        pointer.pointee_size.unwrap_or_default(),
                        pointer.signed,
                        PointerDereferenceRejectReason::UnknownRecordField,
                    ));
                };
                let (mode, reject_reason) = if field.offset == 0 {
                    (PointerDereferenceMode::IndirectY, None)
                } else if record_field_fits_indirect_y(field) {
                    (PointerDereferenceMode::IndirectYWithOffset, None)
                } else {
                    (
                        PointerDereferenceMode::NeedsAddressArithmetic,
                        Some(PointerDereferenceRejectReason::FieldOffsetTooWide),
                    )
                };
                Some(PointerDereferenceProof {
                    pointer,
                    kind: PointerDereferenceKind::RecordField,
                    pointee_size: field.size,
                    signed: field.signed,
                    field: Some(field),
                    index: None,
                    mode,
                    reject_reason,
                })
            }
            _ => None,
        }
    }

    pub(super) fn value_availability_proof(&self, expr: &Expr) -> ValueAvailabilityProof {
        if let Some(value) = self.constant_u16(expr) {
            return ValueAvailabilityProof::constant(value);
        }
        if let Some(slot) = self.direct_scalar_slot(expr) {
            return ValueAvailabilityProof::slot(slot, self.expr_size(expr));
        }

        match &expr.kind {
            ExprKind::Call { callee, args }
                if self.array_call_slot_size(callee, args).is_none() =>
            {
                if let Some(info) = self.call_routine_info(callee) {
                    return ValueAvailabilityProof::routine_return(info);
                }
            }
            _ => {}
        }

        ValueAvailabilityProof {
            width: self.expr_size(expr),
            source: ValueAvailabilitySource::Unknown,
            bytes: [None, None],
        }
    }

    pub(super) fn routine_visibility_facts(&self, name: &str) -> Option<RoutineVisibilityFacts> {
        let proof = self.routine_boundary_proof(name)?;
        Some(RoutineVisibilityFacts {
            retargetable: proof.retargetable,
            address_taken: proof.address_taken,
            internal_only_candidate: proof.internal_only_candidate,
        })
    }

    pub(super) fn routine_boundary_proof(&self, name: &str) -> Option<RoutineBoundaryProof> {
        let normalized = normalize_name(name);
        let routine = self.routines.get(&normalized)?;
        let retargetable = self.routine_assignment_targets.contains(&normalized);
        let address_taken = retargetable;
        let kind = if routine.system_address.is_some() {
            RoutineBoundaryKind::System
        } else if retargetable || address_taken {
            RoutineBoundaryKind::Retargetable
        } else {
            RoutineBoundaryKind::InternalCandidate
        };
        let internal_only_candidate = kind == RoutineBoundaryKind::InternalCandidate;
        Some(RoutineBoundaryProof {
            name: normalized,
            kind,
            system_address: routine.system_address,
            retargetable,
            address_taken,
            internal_only_candidate,
            public_entry_required: !internal_only_candidate,
            patchable_entry_required: retargetable,
            internal_abi_candidate: internal_only_candidate,
        })
    }
}

impl ValueAvailabilityProof {
    fn constant(value: u16) -> Self {
        let width = if value > 0xFF { 2 } else { 1 };
        let mut bytes = [None, None];
        bytes[0] = Some(ValueByteAvailability::Constant(value as u8));
        if width > 1 {
            bytes[1] = Some(ValueByteAvailability::Constant((value >> 8) as u8));
        }
        Self {
            width: Some(width),
            source: ValueAvailabilitySource::Constant,
            bytes,
        }
    }

    fn slot(slot: StorageSlot, width: Option<u16>) -> Self {
        let mut bytes = [None, None];
        for byte_index in 0..slot.size.min(2) {
            bytes[usize::from(byte_index)] = Some(ValueByteAvailability::Slot { slot, byte_index });
        }
        Self {
            width,
            source: ValueAvailabilitySource::Storage,
            bytes,
        }
    }

    fn routine_return(info: RoutineInfo) -> Self {
        let mut bytes = [None, None];
        let abi = info.internal_abi();
        let Some(slot) = abi.public_result_slot() else {
            return Self {
                width: None,
                source: ValueAvailabilitySource::RoutineReturn,
                bytes,
            };
        };
        for byte_index in 0..slot.size.min(2) {
            bytes[usize::from(byte_index)] = match abi.result_byte(byte_index) {
                Some(InternalResultByte::RegisterA) => {
                    Some(ValueByteAvailability::Register(RegisterName::A))
                }
                Some(InternalResultByte::RegisterX) => {
                    Some(ValueByteAvailability::Register(RegisterName::X))
                }
                Some(InternalResultByte::RegisterY) => {
                    Some(ValueByteAvailability::Register(RegisterName::Y))
                }
                Some(InternalResultByte::PublicSlot(public_byte)) => {
                    Some(ValueByteAvailability::PublicReturnSlot {
                        slot,
                        byte_index: public_byte,
                    })
                }
                None => None,
            };
        }
        Self {
            width: Some(slot.size),
            source: ValueAvailabilitySource::RoutineReturn,
            bytes,
        }
    }
}

fn storage_name_is_volatile(name: &str, slot: StorageSlot) -> bool {
    matches!(normalize_name(name).as_str(), "COLOR" | "DEVICE")
        || matches!(slot.space, AddressSpace::ZeroPage)
            && matches!(slot.address as u8, 0x11 | 0x82..=0x87 | 0xB7 | 0xC2)
}

fn array_call_reads_through_pointer(generator: &Generator, callee: &Expr, args: &[Expr]) -> bool {
    if args.len() != 1 {
        return false;
    }
    let ExprKind::Name(name) = &callee.kind else {
        return false;
    };
    generator.lookup_slot(name).is_some_and(|slot| {
        slot.pointee_size.is_some()
            || matches!(
                slot.array,
                Some(ArrayStorage::Pointer | ArrayStorage::Descriptor)
            )
    })
}

impl PointerDereferenceProof {
    fn rejected(
        pointer: StorageSlot,
        kind: PointerDereferenceKind,
        pointee_size: u16,
        signed: bool,
        reject_reason: PointerDereferenceRejectReason,
    ) -> Self {
        Self {
            pointer,
            kind,
            pointee_size,
            signed,
            field: None,
            index: None,
            mode: PointerDereferenceMode::Unsupported,
            reject_reason: Some(reject_reason),
        }
    }
}

fn pointer_index_mode_from_index_proof(
    index: IndexAddressProof,
) -> (
    PointerDereferenceMode,
    Option<PointerDereferenceRejectReason>,
) {
    match index.reject_reason {
        None if index.mode == IndexAddressMode::IndirectY => {
            (PointerDereferenceMode::IndirectY, None)
        }
        None if index.mode == IndexAddressMode::ScaledIndirectY => {
            (PointerDereferenceMode::ScaledIndirectY, None)
        }
        Some(IndexAddressRejectReason::ElementNeedsScaling) => (
            PointerDereferenceMode::Unsupported,
            Some(PointerDereferenceRejectReason::ElementNeedsScaling),
        ),
        Some(IndexAddressRejectReason::IndexHasSideEffects) => (
            PointerDereferenceMode::Unsupported,
            Some(PointerDereferenceRejectReason::IndexHasSideEffects),
        ),
        Some(IndexAddressRejectReason::NonByteIndex) => (
            PointerDereferenceMode::Unsupported,
            Some(PointerDereferenceRejectReason::NonByteIndex),
        ),
        _ => (
            PointerDereferenceMode::Unsupported,
            Some(PointerDereferenceRejectReason::UnsupportedShape),
        ),
    }
}

pub(super) fn call_boundary_proof(
    temp_home: VirtualTempHome,
    callee_effects: RoutineEffects,
) -> CallBoundaryProof {
    CallBoundaryProof {
        temp_home,
        callee_effects,
        survives: zero_page_temp_survives_effects(temp_home, callee_effects),
    }
}

pub(super) fn zero_page_temp_lifetime_proof(
    temp_home: VirtualTempHome,
    crossed_calls: &[RoutineEffects],
) -> ZeroPageTempLifetimeProof {
    let first_blocking_call = crossed_calls
        .iter()
        .position(|effects| !zero_page_temp_survives_effects(temp_home, *effects));
    ZeroPageTempLifetimeProof {
        temp_home,
        calls_crossed: crossed_calls.len(),
        survives_all_calls: first_blocking_call.is_none(),
        first_blocking_call,
    }
}

pub(super) fn zero_page_temp_placement_proof(
    width: VirtualTempWidth,
    pool: &ZeroPageTempPool,
    occupied_slots: &[StorageSlot],
) -> ZeroPageTempPlacementProof {
    let candidates = pool.candidates(width);
    let candidate = candidates.iter().copied().find(|candidate| {
        !occupied_slots
            .iter()
            .any(|occupied| storage_slots_overlap(*occupied, candidate.slot))
    });
    ZeroPageTempPlacementProof {
        width,
        candidate,
        blocked_by_occupied_slot: candidate.is_none() && !candidates.is_empty(),
    }
}
