use super::*;

pub(super) fn debug_assert_negation_source_slot(
    expr: &Expr,
    source: StorageSlot,
    target: StorageSlot,
) {
    debug_assert!(
        source.size > 0,
        "unary negation source must have a known non-zero width"
    );
    debug_assert!(
        target.size > 0,
        "unary negation target must have a known non-zero width"
    );
    debug_assert!(
        source.pointee_size.is_none(),
        "unary negation source must be a value slot, not pointer storage"
    );
    debug_assert!(
        source.array.is_none(),
        "unary negation source must be a value slot, not array storage"
    );
    if matches!(
        &expr.kind,
        ExprKind::Unary {
            op: UnaryOp::Deref,
            ..
        } | ExprKind::Index { .. }
            | ExprKind::Call { .. }
            | ExprKind::Field { .. }
    ) {
        debug_assert!(
            source.space == AddressSpace::IndirectIndexedY
                || source.space == AddressSpace::Absolute
                || source.space == AddressSpace::AbsoluteX
                || source.space == AddressSpace::ZeroPage,
            "unary negation lvalue source must resolve before subtraction begins"
        );
    }
}

pub(super) fn debug_assert_assignment_width_shape(
    target: &Expr,
    value: &Expr,
    slot: StorageSlot,
    value_width: Option<u16>,
    stores_value_address: bool,
) {
    debug_assert_lvalue_slot_shape(target, slot);
    debug_assert!(
        slot_accessible_byte_size(slot) > 0,
        "assignment target must have a writable width"
    );
    debug_assert!(
        slot.array.is_none(),
        "assignment target must be value storage, not array backing"
    );
    if slot.pointee_size.is_some() {
        debug_assert_eq!(slot.size, 2, "pointer assignment target must be word-sized");
        debug_assert!(
            stores_value_address || value_width.is_none_or(|width| width <= 2),
            "pointer assignment value must fit in a word-sized address"
        );
    }
    if matches!(
        target.kind,
        ExprKind::Unary {
            op: UnaryOp::Deref,
            ..
        }
    ) {
        debug_assert!(
            slot.pointee_size.is_none(),
            "pointer dereference assignment must write the pointee value, not pointer storage"
        );
    }
    if matches!(value.kind, ExprKind::String(_)) {
        debug_assert!(
            slot_accessible_byte_size(slot) >= 2,
            "string assignment value must be written into pointer-width storage"
        );
    }
}

pub(super) fn debug_assert_runtime_helper_abi_shape(
    helper: RuntimeHelperSlot,
    target: &RuntimeHelperTarget,
    result: StorageSlot,
    store_right_high: bool,
) {
    debug_assert_runtime_helper_target(helper, target);
    debug_assert_storage_slot_shape(result);
    debug_assert!(
        matches!(result.size, 1 | 2),
        "runtime helper result must be byte- or word-sized"
    );
    debug_assert!(
        result.pointee_size.is_none(),
        "runtime helper result must be value storage, not pointer storage"
    );
    debug_assert!(
        result.array.is_none(),
        "runtime helper result must be value storage, not array storage"
    );
    match helper {
        RuntimeHelperSlot::Mul | RuntimeHelperSlot::Div | RuntimeHelperSlot::Mod => {
            debug_assert!(
                store_right_high,
                "{helper:?} ABI requires right operand low/high in AFCUR/AFSIZE"
            );
        }
        RuntimeHelperSlot::Lsh | RuntimeHelperSlot::Rsh => {
            debug_assert!(
                !store_right_high,
                "{helper:?} ABI requires shift count only in AFCUR low byte"
            );
        }
        RuntimeHelperSlot::SArgs => {
            debug_assert!(
                false,
                "SArgs must use the dedicated parameter-frame ABI guard"
            );
        }
    }
}

pub(super) fn debug_assert_sargs_helper_abi(
    target: &RuntimeHelperTarget,
    frame_base: u16,
    arg_bytes: u8,
) {
    debug_assert_runtime_helper_target(RuntimeHelperSlot::SArgs, target);
    debug_assert!(
        arg_bytes >= 3,
        "SArgs ABI is only used for stack argument frames of three or more bytes"
    );
    debug_assert!(
        frame_base >= 0x0100,
        "SArgs ABI destination frame must be addressable absolute storage"
    );
    debug_assert!(
        frame_base.checked_add(u16::from(arg_bytes) - 1).is_some(),
        "SArgs ABI destination frame must not wrap memory"
    );
}

pub(super) fn debug_assert_runtime_helper_target(
    helper: RuntimeHelperSlot,
    target: &RuntimeHelperTarget,
) {
    debug_assert_runtime_helper_target_is_callable(target);
    let RuntimeHelperTarget::Absolute(address) = target else {
        return;
    };
    if let Some(mapped_helper) = known_runtime_helper_target(*address) {
        debug_assert_eq!(
            mapped_helper,
            helper,
            "runtime helper target ${:04X} belongs to {mapped_helper:?}, not {helper:?}",
            address.address()
        );
    }
}

pub(super) fn debug_assert_runtime_helper_target_is_callable(target: &RuntimeHelperTarget) {
    if let RuntimeHelperTarget::Absolute(address) = target {
        debug_assert!(
            address.address() >= 0x0100,
            "runtime helper target must be a callable absolute address, not zero page"
        );
    }
}

pub(super) fn known_runtime_helper_target(address: Absolute) -> Option<RuntimeHelperSlot> {
    match address.address() {
        value
            if value == runtime_helper::LSH_SLOT.address()
                || value == runtime_helper::CARTRIDGE_LSH.address() =>
        {
            Some(RuntimeHelperSlot::Lsh)
        }
        value
            if value == runtime_helper::RSH_SLOT.address()
                || value == runtime_helper::CARTRIDGE_RSH.address() =>
        {
            Some(RuntimeHelperSlot::Rsh)
        }
        value
            if value == runtime_helper::MUL_SLOT.address()
                || value == runtime_helper::CARTRIDGE_MUL.address() =>
        {
            Some(RuntimeHelperSlot::Mul)
        }
        value
            if value == runtime_helper::DIV_SLOT.address()
                || value == runtime_helper::CARTRIDGE_DIV.address() =>
        {
            Some(RuntimeHelperSlot::Div)
        }
        value
            if value == runtime_helper::MOD_SLOT.address()
                || value == runtime_helper::CARTRIDGE_MOD.address() =>
        {
            Some(RuntimeHelperSlot::Mod)
        }
        value
            if value == runtime_helper::SARGS_SLOT.address()
                || value == runtime_helper::CARTRIDGE_SARGS.address() =>
        {
            Some(RuntimeHelperSlot::SArgs)
        }
        _ => None,
    }
}

pub(super) fn debug_assert_expr_target_slot_shape(expr: &Expr, slot: StorageSlot) {
    debug_assert_storage_slot_shape(slot);
    debug_assert!(
        slot.size > 0,
        "expression target slot must have a non-zero width"
    );
    if matches!(expr.kind, ExprKind::String(_)) {
        debug_assert!(
            slot.size >= 2,
            "string expression target must be able to hold an address"
        );
    }
}

pub(super) fn debug_assert_lvalue_slot_shape(expr: &Expr, slot: StorageSlot) {
    debug_assert_storage_slot_shape(slot);
    debug_assert!(
        slot.size > 0,
        "lvalue slot must have a non-zero value width"
    );
    debug_assert!(
        slot.array.is_none(),
        "lvalue expression must resolve to value storage, not array storage"
    );
    if matches!(
        expr.kind,
        ExprKind::Index { .. }
            | ExprKind::Call { .. }
            | ExprKind::Field { .. }
            | ExprKind::Unary {
                op: UnaryOp::Deref,
                ..
            }
    ) {
        debug_assert!(
            slot.pointee_size.is_none(),
            "lvalue expression must resolve to pointee value storage, not pointer storage"
        );
    }
}

pub(super) fn debug_assert_copy_slot_shape(source: StorageSlot, target: StorageSlot) {
    debug_assert_storage_slot_shape(source);
    debug_assert_storage_slot_shape(target);
    debug_assert!(
        source.size > 0 && target.size > 0,
        "slot copy requires non-zero source and target widths"
    );
    debug_assert!(
        target.array != Some(ArrayStorage::Inline)
            && target.array != Some(ArrayStorage::Descriptor),
        "slot copy target must be scalar storage or array-pointer storage"
    );
}

pub(super) fn debug_assert_slot_byte_access(slot: StorageSlot, byte_index: u16, operation: &str) {
    debug_assert_storage_slot_shape(slot);
    let accessible_size = slot_accessible_byte_size(slot);
    debug_assert!(
        byte_index < accessible_size,
        "{operation} byte index {byte_index} must stay inside slot width {}",
        accessible_size
    );
    if slot.space == AddressSpace::IndirectIndexedY {
        debug_assert!(
            slot.byte_address(0) <= 0xFF,
            "indirect-indexed slot must be backed by a zero-page pointer"
        );
        debug_assert_indirect_pointer_pair(slot.zero_page_byte(0), operation);
    }
}

pub(super) fn debug_assert_storage_slot_shape(slot: StorageSlot) {
    debug_assert!(slot.size > 0, "storage slot width must be non-zero");
    if slot.pointee_size.is_some() {
        debug_assert_eq!(slot.size, 2, "pointer storage must be word-sized");
    }
    if slot.space == AddressSpace::ZeroPage {
        debug_assert!(
            slot.address <= 0xFF,
            "zero-page slot must start inside zero page"
        );
        debug_assert!(
            slot.address.saturating_add(slot.size.saturating_sub(1)) <= 0xFF,
            "zero-page slot must not extend past zero page"
        );
    }
    if slot.space == AddressSpace::IndirectIndexedY {
        debug_assert!(
            slot.address <= 0xFF,
            "indirect-indexed slot pointer must live in zero page"
        );
        debug_assert_indirect_pointer_pair(slot.zero_page_byte(0), "storage slot");
    }
}

pub(super) fn debug_assert_scratch_indirect_pointer(pointer: ZeroPage, context: &str) {
    debug_assert!(
        is_runtime_indirect_pointer(pointer),
        "{context} must use one of the Action! runtime pointer registers"
    );
}

pub(super) fn debug_assert_prepared_indirect_slot(
    slot: StorageSlot,
    pointer: ZeroPage,
    context: &str,
) {
    debug_assert_indirect_slot_pointer(slot, pointer, context);
    debug_assert_scratch_indirect_pointer(pointer, context);
}

pub(super) fn debug_assert_indirect_slot_pointer(
    slot: StorageSlot,
    pointer: ZeroPage,
    context: &str,
) {
    debug_assert_eq!(
        slot.space,
        AddressSpace::IndirectIndexedY,
        "{context} must produce an indirect-indexed value slot"
    );
    debug_assert_eq!(
        slot.zero_page_byte(0),
        pointer,
        "{context} indirect slot must use the prepared pointer"
    );
    debug_assert_indirect_pointer_pair(pointer, context);
}

pub(super) fn debug_assert_indirect_slots_do_not_alias(
    source: StorageSlot,
    target: StorageSlot,
    context: &str,
) {
    if source.space == AddressSpace::IndirectIndexedY
        && target.space == AddressSpace::IndirectIndexedY
    {
        debug_assert_ne!(
            source.zero_page_byte(0),
            target.zero_page_byte(0),
            "{context} must use distinct indirect pointers for source and target"
        );
    }
}

pub(super) fn debug_assert_indirect_pointer_pair(pointer: ZeroPage, context: &str) {
    debug_assert!(
        pointer.address() < 0xFF,
        "{context} indirect pointer must have a high byte in zero page"
    );
}

pub(super) fn is_runtime_indirect_pointer(pointer: ZeroPage) -> bool {
    pointer == runtime_zp::ARRAY_ADDR
        || pointer == runtime_zp::ELEMENT_ADDR
        || pointer == runtime_zp::VALUE_TEMP
        || pointer == runtime_zp::ADDR
}

pub(super) fn slot_accessible_byte_size(slot: StorageSlot) -> u16 {
    if matches!(
        slot.array,
        Some(ArrayStorage::Pointer | ArrayStorage::Descriptor)
    ) {
        2
    } else {
        slot.size
    }
}
