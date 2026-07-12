use super::proof::IndexAddressMode;
use super::proof::{PointerDereferenceKind, PointerDereferenceMode};
use super::*;

impl Generator {
    pub(super) fn emit_load_array_pointer_value_byte(
        &mut self,
        expr: &Expr,
        byte_index: u16,
    ) -> bool {
        let ExprKind::Name(name) = &expr.kind else {
            return false;
        };
        let Some(slot) = self.lookup_slot(name) else {
            return false;
        };
        if slot.array.is_none() {
            return false;
        }
        self.emit_load_array_pointer_value_slot_byte(slot, byte_index);
        true
    }

    pub(super) fn emit_load_array_pointer_value_slot_byte(
        &mut self,
        slot: StorageSlot,
        byte_index: u16,
    ) {
        match slot.array {
            Some(ArrayStorage::Inline) if byte_index < 2 => {
                self.emit_lda_immediate(Immediate::new(slot.address), byte_index);
            }
            Some(ArrayStorage::Inline) => {
                self.emit_lda_imm(0);
            }
            Some(ArrayStorage::Pointer | ArrayStorage::Descriptor) if byte_index < 2 => {
                self.emit_lda_absolute(Absolute::new(slot.address.wrapping_add(byte_index)));
            }
            Some(ArrayStorage::Pointer | ArrayStorage::Descriptor) => {
                self.emit_lda_imm(0);
            }
            None => unreachable!(),
        }
    }

    pub(super) fn emit_load_array_descriptor_pointer_byte(
        &mut self,
        expr: &Expr,
        byte_index: u16,
    ) -> bool {
        let ExprKind::Name(name) = &expr.kind else {
            return false;
        };
        let Some(slot) = self.lookup_slot(name) else {
            return false;
        };
        match slot.array {
            Some(ArrayStorage::Pointer | ArrayStorage::Descriptor) if byte_index < 2 => {
                self.emit_lda_absolute(Absolute::new(slot.address.wrapping_add(byte_index)));
                true
            }
            Some(ArrayStorage::Pointer | ArrayStorage::Descriptor) => {
                self.emit_lda_imm(0);
                true
            }
            _ => false,
        }
    }

    pub(super) fn emit_pointer_slot_to_addr(&mut self, slot: StorageSlot, addr: ZeroPage) -> bool {
        if slot.array.is_some() {
            self.emit_load_array_pointer_value_slot_byte(slot, 0);
            self.emit_sta_zero_page(addr);
            self.emit_load_array_pointer_value_slot_byte(slot, 1);
            self.emit_sta_zero_page(addr.offset(1));
            return true;
        }
        if slot.size < 2 {
            return false;
        }
        if self.profile.enables_modern_optimizations()
            && let (Some(low), Some(high)) = (
                self.processor.memory_value(slot, 0),
                self.processor.memory_value(slot, 1),
            )
            && self.processor.zero_page_matches_known_byte(addr, low)
            && self
                .processor
                .zero_page_matches_known_byte(addr.offset(1), high)
        {
            return true;
        }
        self.emit_slot_byte_to_zero_page(slot, 0, addr);
        self.emit_slot_byte_to_zero_page(slot, 1, addr.offset(1));
        true
    }

    pub(super) fn emit_load_effective_address_byte(
        &mut self,
        expr: &Expr,
        byte_index: u16,
    ) -> bool {
        let Some(address) = self.byte_index_effective_address(expr, runtime_zp::ARRAY_ADDR) else {
            return false;
        };
        if byte_index >= address.element_size {
            self.emit_lda_imm(0);
            return true;
        }
        if !self.emit_effective_address_pointer_and_y(address, byte_index) {
            return false;
        }
        self.emit_lda_indirect_indexed_y(IndirectIndexedY::new(address.pointer));
        self.record_modern_optimization(
            CodegenOptimizationKind::EffectiveAddressLowered,
            4,
            Some(expr.span),
            "lowered byte-indexed effective address directly to (zp),Y",
        );
        true
    }

    pub(super) fn byte_index_effective_address(
        &self,
        expr: &Expr,
        pointer: ZeroPage,
    ) -> Option<EffectiveAddress> {
        if !self.profile.enables_modern_optimizations() {
            return None;
        }
        let proof = self.index_address_proof(expr)?;
        if !matches!(
            proof.mode,
            IndexAddressMode::IndirectY | IndexAddressMode::NeedsScaling
        ) || !matches!(proof.element_size, 1 | 2)
        {
            return None;
        }
        if proof.base.pointee_size.is_some() {
            let pointer_proof = self.pointer_dereference_proof(expr)?;
            if pointer_proof.kind != PointerDereferenceKind::Indexed
                || !matches!(
                    pointer_proof.mode,
                    PointerDereferenceMode::IndirectY | PointerDereferenceMode::NeedsIndexScaling
                )
            {
                return None;
            }
        }
        let index_expr = indexed_expr_index(expr)?;
        let index = self.direct_scalar_slot(index_expr)?;
        if index.size != 1 {
            return None;
        }
        Some(EffectiveAddress {
            base: proof.base,
            index,
            pointer,
            element_size: proof.element_size,
        })
    }

    pub(super) fn emit_effective_address_pointer_and_y(
        &mut self,
        address: EffectiveAddress,
        byte_index: u16,
    ) -> bool {
        match address.element_size {
            1 => {
                if byte_index != 0 || !self.emit_effective_address_base_to_pointer(address) {
                    return false;
                }
                self.emit_ldy_slot_byte(address.index, 0);
                true
            }
            2 => {
                if byte_index > 1 {
                    return false;
                }
                self.emit_lda_slot_byte_value_only(address.index, 0);
                self.emit_asl_a();
                self.emit_tay();
                if !self.emit_effective_address_base_to_pointer(address) {
                    return false;
                }
                self.emit_lda_zero_page_value_only(address.pointer.offset(1));
                self.emit_adc_imm(0);
                self.emit_sta_zero_page(address.pointer.offset(1));
                if byte_index == 1 {
                    self.emit_iny();
                }
                true
            }
            _ => false,
        }
    }

    fn emit_effective_address_base_to_pointer(&mut self, address: EffectiveAddress) -> bool {
        if address.base.pointee_size.is_some() {
            return self.emit_pointer_slot_to_addr(address.base, address.pointer);
        }
        self.emit_array_base_to_pointer(address.base, address.pointer)
            .is_some()
    }

    pub(super) fn emit_effective_address_to_slot(
        &mut self,
        expr: &Expr,
        slot: StorageSlot,
    ) -> bool {
        if matches!(
            slot.space,
            AddressSpace::AbsoluteX | AddressSpace::IndirectIndexedY
        ) {
            return false;
        }
        let Some(address) = self.byte_index_effective_address(expr, runtime_zp::ARRAY_ADDR) else {
            return false;
        };
        match address.element_size {
            1 => {
                if !self.emit_effective_address_pointer_and_y(address, 0) {
                    return false;
                }
                self.emit_lda_indirect_indexed_y(IndirectIndexedY::new(address.pointer));
                self.emit_sta_slot_byte(slot, 0);
                for byte_index in 1..slot.size {
                    self.emit_lda_imm(0);
                    self.emit_sta_slot_byte(slot, byte_index);
                }
                self.record_modern_optimization(
                    CodegenOptimizationKind::EffectiveAddressLowered,
                    4,
                    Some(expr.span),
                    "lowered byte-indexed effective address directly to (zp),Y",
                );
                true
            }
            2 => {
                if slot.size < 2 {
                    return false;
                }
                if !self.emit_effective_address_pointer_and_y(address, 0) {
                    return false;
                }
                self.emit_lda_indirect_indexed_y(IndirectIndexedY::new(address.pointer));
                self.emit_sta_slot_byte(slot, 0);
                self.emit_iny();
                self.emit_lda_indirect_indexed_y(IndirectIndexedY::new(address.pointer));
                self.emit_sta_slot_byte(slot, 1);
                self.record_modern_optimization(
                    CodegenOptimizationKind::EffectiveAddressLowered,
                    8,
                    Some(expr.span),
                    "prepared word effective address once for indexed load",
                );
                true
            }
            _ => false,
        }
    }

    pub(super) fn emit_inline_byte_array_scalar_index_to_slot(
        &mut self,
        expr: &Expr,
        slot: StorageSlot,
    ) -> bool {
        if matches!(
            slot.space,
            AddressSpace::AbsoluteX | AddressSpace::IndirectIndexedY
        ) {
            return false;
        }
        let (base, index_expr) = match &expr.kind {
            ExprKind::Index { base, index } => (base.as_ref(), index.as_ref()),
            ExprKind::Call { callee, args } if args.len() == 1 => (callee.as_ref(), &args[0]),
            _ => return false,
        };
        let ExprKind::Name(name) = &base.kind else {
            return false;
        };
        if self.constant_u16(index_expr).is_some()
            || matches!(index_expr.kind, ExprKind::Call { .. })
        {
            return false;
        }
        let Some(array) = self.lookup_slot(name) else {
            return false;
        };
        if array.array != Some(ArrayStorage::Inline)
            || array.size != 1
            || !(self.expr_size(index_expr) == Some(1)
                || self.expr_value_range_fact(index_expr).is_byte())
        {
            return false;
        }
        if let Some(index) = self.direct_scalar_slot(index_expr) {
            if !self.profile.enables_modern_optimizations() || index.size != 1 {
                return false;
            }
            self.emit_ldy_slot_byte(index, 0);
            self.emit_lda_absolute_y(Absolute::new(array.address));
        } else {
            let index = StorageSlot::zero_page(runtime_zp::ELEMENT_ADDR.address(), 1);
            if !self.emit_expr_to_slot(index_expr, index) {
                return false;
            }
            self.emit_ldx_zero_page(runtime_zp::ELEMENT_ADDR);
            self.emitter
                .emit_lda_absolute_x(AbsoluteX::new(array.address));
        }
        self.emit_sta_slot_byte(slot, 0);
        for byte_index in 1..slot.size {
            self.emit_lda_imm(0);
            self.emit_sta_slot_byte(slot, byte_index);
        }
        self.record_modern_optimization(
            CodegenOptimizationKind::EffectiveAddressLowered,
            0,
            Some(expr.span),
            "lowered inline byte array index through proof-guided absolute,Y",
        );
        true
    }

    pub(super) fn emit_pointer_slot_plus_offset_to_addr(
        &mut self,
        slot: StorageSlot,
        offset: u16,
        addr: ZeroPage,
    ) -> bool {
        if slot.size < 2 {
            return false;
        }
        self.emit_clc();
        self.emit_lda_slot_byte(slot, 0);
        self.emit_adc_immediate(Immediate::new(offset), 0);
        self.emit_sta_zero_page(addr);
        self.emit_lda_slot_byte(slot, 1);
        self.emit_adc_immediate(Immediate::new(offset), 1);
        self.emit_sta_zero_page(addr.offset(1));
        true
    }

    pub(super) fn emit_dynamic_array_address(
        &mut self,
        array: StorageSlot,
        index: &Expr,
    ) -> Option<ZeroPage> {
        if !self.segment_storage {
            return self.emit_legacy_dynamic_array_address(array, index);
        }

        if self.emit_array_base_plus_scaled_byte_index_to_addr(array, index) {
            return Some(runtime_zp::ARRAY_ADDR);
        }

        if self.emit_array_base_plus_call_byte_index_to_addr(array, index) {
            return Some(runtime_zp::ARRAY_ADDR);
        }

        if let Some(pointer) = self.emit_compatible_word_array_expr_index_address(array, index) {
            return Some(pointer);
        }

        if let Some(pointer) = self.emit_complex_byte_index_array_address(array, index) {
            return Some(pointer);
        }

        if !self.emit_index_expr_to_temp(index, runtime_zp::ADDR) {
            return None;
        }

        self.emit_array_base_to_addr(array)?;

        self.emit_lda_zero_page(runtime_zp::ADDR);
        if array.size == 2 {
            self.emit_asl_a();
        } else if array.size != 1 {
            return None;
        }
        self.emit_clc();
        self.emit_adc_zero_page(runtime_zp::ARRAY_ADDR);
        self.emit_sta_zero_page(runtime_zp::ARRAY_ADDR);

        self.emit_lda_zero_page(runtime_zp::ADDR.offset(1));
        if array.size == 2 {
            self.emit_rol_a();
        }
        self.emit_adc_zero_page(runtime_zp::ARRAY_ADDR.offset(1));
        self.emit_sta_zero_page(runtime_zp::ARRAY_ADDR.offset(1));
        Some(runtime_zp::ARRAY_ADDR)
    }

    pub(super) fn emit_compatible_word_array_expr_index_address(
        &mut self,
        array: StorageSlot,
        index: &Expr,
    ) -> Option<ZeroPage> {
        array.array?;
        if array.size != 2
            || self.expr_size(index)? != 2
            || self.direct_scalar_slot(index).is_some()
        {
            return None;
        }
        if !self.emit_index_expr_to_temp(index, runtime_zp::ARRAY_ADDR) {
            return None;
        }
        self.emit_word_array_address_from_word_index_temp(
            array,
            runtime_zp::ARRAY_ADDR,
            runtime_zp::ELEMENT_ADDR,
        )?;
        Some(runtime_zp::ELEMENT_ADDR)
    }

    fn emit_legacy_dynamic_array_address(
        &mut self,
        array: StorageSlot,
        index: &Expr,
    ) -> Option<ZeroPage> {
        let AddressSpace::Absolute = array.space else {
            return None;
        };

        if !self.emit_index_expr_to_temp(index, runtime_zp::ARRAY_ADDR) {
            return None;
        }

        self.emit_lda_zero_page(runtime_zp::ARRAY_ADDR);
        if array.size == 2 {
            self.emit_asl_a();
        } else if array.size != 1 {
            return None;
        }
        self.emit_clc();
        self.emit_adc_immediate(Immediate::new(array.address), 0);
        self.emit_sta_zero_page(runtime_zp::ADDR);

        self.emit_lda_zero_page(runtime_zp::ARRAY_ADDR.offset(1));
        if array.size == 2 {
            self.emit_rol_a();
        }
        self.emit_adc_immediate(Immediate::new(array.address), 1);
        self.emit_sta_zero_page(runtime_zp::ADDR.offset(1));
        Some(runtime_zp::ADDR)
    }

    fn emit_complex_byte_index_array_address(
        &mut self,
        array: StorageSlot,
        index: &Expr,
    ) -> Option<ZeroPage> {
        self.emit_complex_byte_index_array_address_to_pointer(
            array,
            index,
            runtime_zp::ELEMENT_ADDR,
            runtime_zp::ARRAY_ADDR,
        )?;
        Some(runtime_zp::ELEMENT_ADDR)
    }

    pub(super) fn emit_complex_byte_index_array_address_to_pointer(
        &mut self,
        array: StorageSlot,
        index: &Expr,
        pointer: ZeroPage,
        temp: ZeroPage,
    ) -> Option<()> {
        array.array?;
        if !matches!(array.size, 1 | 2) || self.expr_size(index)? != 1 {
            return None;
        }
        if pointer == temp {
            return None;
        }

        if array.size == 2
            && let ExprKind::Binary {
                op: op @ (BinaryOp::Lsh | BinaryOp::Rsh),
                left,
                right,
            } = &index.kind
            && let Some(count) = self.constant_u16(right)
            && self.expr_size(left).is_some_and(|size| count < size * 8)
        {
            if !self.emit_index_low_expr_to_temp(left, temp) {
                return None;
            }
            self.emit_lda_zero_page(temp);
            for _ in 0..count {
                match op {
                    BinaryOp::Lsh => self.emit_asl_a(),
                    BinaryOp::Rsh => self.emit_lsr_a(),
                    _ => unreachable!(),
                }
            }
            self.emit_sta_zero_page(temp);
            self.emit_word_array_address_from_byte_index_temp(array, temp, pointer)?;
            return Some(());
        }

        if !self.emit_index_low_expr_to_temp(index, temp) {
            return None;
        }

        if array.size == 1 {
            self.emit_clc();
            self.emit_array_base_low_for_add(array)?;
            self.emit_adc_zero_page(temp);
            self.emit_sta_zero_page(pointer);
            self.emit_array_base_high_for_add(array)?;
            self.emit_adc_imm(0);
            self.emit_sta_zero_page(pointer.offset(1));
            return Some(());
        }

        self.emit_word_array_address_from_byte_index_temp(array, temp, pointer)?;
        Some(())
    }

    fn emit_word_array_address_from_byte_index_temp(
        &mut self,
        array: StorageSlot,
        temp: ZeroPage,
        pointer: ZeroPage,
    ) -> Option<()> {
        self.emit_lda_zero_page_value_only(temp);
        self.emit_asl_a();
        self.emitter.emit_php();
        self.emit_clc();
        self.emit_adc_array_base_low(array)?;
        self.emit_sta_zero_page(pointer);
        self.emit_lda_imm(0);
        self.emit_rol_a();
        self.emit_plp();
        self.emit_adc_array_base_high(array)?;
        self.emit_sta_zero_page(pointer.offset(1));
        Some(())
    }

    fn emit_word_array_address_from_word_index_temp(
        &mut self,
        array: StorageSlot,
        temp: ZeroPage,
        pointer: ZeroPage,
    ) -> Option<()> {
        self.emit_lda_zero_page_value_only(temp);
        self.emit_asl_a();
        self.emitter.emit_php();
        self.emit_clc();
        self.emit_adc_array_base_low(array)?;
        self.emit_sta_zero_page(pointer);
        self.emit_lda_zero_page_value_only(temp.offset(1));
        self.emit_rol_a();
        self.emit_plp();
        self.emit_adc_array_base_high(array)?;
        self.emit_sta_zero_page(pointer.offset(1));
        Some(())
    }

    fn emit_index_low_expr_to_temp(&mut self, index: &Expr, temp: ZeroPage) -> bool {
        if !self.emit_index_low_expr_to_acc(index) {
            return false;
        }
        self.emit_sta_zero_page(temp);
        true
    }

    pub(super) fn emit_index_low_expr_to_acc(&mut self, index: &Expr) -> bool {
        match &index.kind {
            ExprKind::Binary {
                op:
                    op @ (BinaryOp::Add | BinaryOp::Sub | BinaryOp::And | BinaryOp::Or | BinaryOp::Xor),
                left,
                right,
            } => self.emit_binary_expr_byte(*op, left, right, 0, true),
            ExprKind::Binary {
                op: op @ (BinaryOp::Lsh | BinaryOp::Rsh),
                left,
                right,
            } if self
                .constant_u16(right)
                .zip(self.expr_size(left))
                .is_some_and(|(count, size)| count < size * 8) =>
            {
                let Some(count) = self.constant_u16(right) else {
                    return false;
                };
                if !self.emit_index_low_expr_to_acc(left) {
                    return false;
                }
                for _ in 0..count {
                    match op {
                        BinaryOp::Lsh => self.emit_asl_a(),
                        BinaryOp::Rsh => self.emit_lsr_a(),
                        _ => unreachable!(),
                    }
                }
                true
            }
            _ => self.emit_load_simple_byte(index, 0),
        }
    }

    pub(super) fn emit_index_expr_to_temp(&mut self, index: &Expr, temp: ZeroPage) -> bool {
        let slot = StorageSlot::zero_page(temp.address(), 2);
        self.emit_expr_to_slot(index, slot)
    }

    pub(super) fn emit_array_base_to_addr(&mut self, array: StorageSlot) -> Option<()> {
        self.emit_array_base_to_pointer(array, runtime_zp::ARRAY_ADDR)
    }

    pub(super) fn emit_array_base_to_pointer(
        &mut self,
        array: StorageSlot,
        pointer: ZeroPage,
    ) -> Option<()> {
        let address = Immediate::new(array.address);
        match array.array? {
            ArrayStorage::Inline => {
                self.emit_lda_immediate(address, 0);
                self.emit_sta_zero_page(pointer);
                self.emit_lda_immediate(address, 1);
                self.emit_sta_zero_page(pointer.offset(1));
            }
            ArrayStorage::Pointer | ArrayStorage::Descriptor => {
                self.emit_lda_absolute(Absolute::new(array.address));
                self.emit_sta_zero_page(pointer);
                self.emitter
                    .emit_lda_absolute(Absolute::new(array.address.wrapping_add(1)));
                self.emit_sta_zero_page(pointer.offset(1));
            }
        }
        Some(())
    }

    pub(super) fn emit_array_base_plus_constant_to_pointer(
        &mut self,
        array: StorageSlot,
        offset: u16,
        pointer: ZeroPage,
    ) -> Option<()> {
        let immediate = Immediate::new(offset);
        self.emit_clc();
        self.emit_array_base_low_for_add(array)?;
        self.emit_adc_immediate(immediate, 0);
        self.emit_sta_zero_page(pointer);
        self.emit_array_base_high_for_add(array)?;
        self.emit_adc_immediate(immediate, 1);
        self.emit_sta_zero_page(pointer.offset(1));
        Some(())
    }

    pub(super) fn emit_array_base_plus_constant_to_addr(
        &mut self,
        array: StorageSlot,
        offset: u16,
    ) -> Option<()> {
        if self.segment_storage
            && array.size == 2
            && offset > 0
            && offset.is_multiple_of(2)
            && matches!(
                array.array?,
                ArrayStorage::Pointer | ArrayStorage::Descriptor
            )
        {
            let index = Immediate::new(offset / 2);
            self.emit_lda_immediate(index, 0);
            self.emit_asl_a();
            self.emitter.emit_php();
            self.emit_clc();
            self.emit_adc_array_base_low(array)?;
            self.emit_sta_zero_page(runtime_zp::ARRAY_ADDR);
            self.emit_lda_immediate(index, 1);
            self.emit_rol_a();
            self.emit_plp();
            self.emit_adc_array_base_high(array)?;
            self.emitter
                .emit_sta_zero_page(runtime_zp::ARRAY_ADDR.offset(1));
            return Some(());
        }

        self.emit_array_base_plus_constant_to_pointer(array, offset, runtime_zp::ARRAY_ADDR)
    }

    pub(super) fn emit_array_base_plus_scaled_byte_index_to_addr(
        &mut self,
        array: StorageSlot,
        index: &Expr,
    ) -> bool {
        self.emit_array_base_plus_scaled_byte_index_to_pointer(array, index, runtime_zp::ARRAY_ADDR)
    }

    pub(super) fn emit_array_base_plus_scaled_byte_index_to_pointer(
        &mut self,
        array: StorageSlot,
        index: &Expr,
        pointer: ZeroPage,
    ) -> bool {
        if array.size != 1 && array.size != 2 {
            return false;
        }
        let Some(index_slot) = self.direct_scalar_slot(index) else {
            return false;
        };
        if index_slot.size != 1 {
            if array.size == 1 && index_slot.size == 2 {
                self.emit_clc();
                if self.emit_array_base_low_for_add(array).is_none() {
                    return false;
                }
                self.emit_adc_slot_byte(index_slot, 0);
                self.emit_sta_zero_page(pointer);
                if self.emit_array_base_high_for_add(array).is_none() {
                    return false;
                }
                self.emit_adc_slot_byte(index_slot, 1);
                self.emit_sta_zero_page(pointer.offset(1));
                return true;
            }
            if array.size == 2 && index_slot.size == 2 {
                self.emit_lda_slot_byte_value_only(index_slot, 0);
                self.emit_asl_a();
                self.emitter.emit_php();
                self.emit_clc();
                if self.emit_adc_array_base_low(array).is_none() {
                    return false;
                }
                self.emit_sta_zero_page(pointer);
                self.emit_lda_slot_byte_value_only(index_slot, 1);
                self.emit_rol_a();
                self.emit_plp();
                if self.emit_adc_array_base_high(array).is_none() {
                    return false;
                }
                self.emit_sta_zero_page(pointer.offset(1));
                return true;
            }
            return false;
        }

        if array.size == 1 {
            self.emit_clc();
            if self.emit_array_base_low_for_add(array).is_none() {
                return false;
            }
            self.emit_adc_slot_byte(index_slot, 0);
            self.emit_sta_zero_page(pointer);
            if self.emit_array_base_high_for_add(array).is_none() {
                return false;
            }
            self.emit_adc_imm(0);
            self.emit_sta_zero_page(pointer.offset(1));
            return true;
        }

        self.emit_lda_slot_byte_value_only(index_slot, 0);
        self.emit_asl_a();
        self.emitter.emit_php();
        self.emit_clc();
        if self.emit_adc_array_base_low(array).is_none() {
            return false;
        }
        self.emit_sta_zero_page(pointer);
        self.emit_lda_imm(0);
        self.emit_rol_a();
        self.emit_plp();
        if self.emit_adc_array_base_high(array).is_none() {
            return false;
        }
        self.emit_sta_zero_page(pointer.offset(1));
        true
    }

    fn emit_array_base_plus_call_byte_index_to_addr(
        &mut self,
        array: StorageSlot,
        index: &Expr,
    ) -> bool {
        if array.size != 1 && array.size != 2 {
            return false;
        }
        let ExprKind::Call { callee, args } = &index.kind else {
            return false;
        };
        if self.array_call_slot_size(callee, args).is_some() {
            return false;
        }
        let Some(return_slot) = self.call_return_slot(callee) else {
            return false;
        };
        if return_slot.size != 1 || !self.emit_call(callee, args, index.span) {
            return false;
        }

        if array.size == 1 {
            self.emit_clc();
            self.emit_lda_slot_byte(return_slot, 0);
            if self.emit_adc_array_base_low(array).is_none() {
                return false;
            }
            self.emit_sta_zero_page(runtime_zp::ARRAY_ADDR);
            if self.emit_array_base_high_for_add(array).is_none() {
                return false;
            }
            self.emit_adc_imm(0);
            self.emitter
                .emit_sta_zero_page(runtime_zp::ARRAY_ADDR.offset(1));
            return true;
        }

        self.emit_lda_slot_byte(return_slot, 0);
        self.emit_asl_a();
        self.emitter.emit_php();
        self.emit_clc();
        if self.emit_adc_array_base_low(array).is_none() {
            return false;
        }
        self.emit_sta_zero_page(runtime_zp::ARRAY_ADDR);
        self.emit_lda_imm(0);
        self.emit_rol_a();
        self.emit_plp();
        if self.emit_adc_array_base_high(array).is_none() {
            return false;
        }
        self.emitter
            .emit_sta_zero_page(runtime_zp::ARRAY_ADDR.offset(1));
        true
    }

    fn emit_array_base_low_for_add(&mut self, array: StorageSlot) -> Option<()> {
        match array.array? {
            ArrayStorage::Inline => {
                self.emit_lda_immediate(Immediate::new(array.address), 0);
            }
            ArrayStorage::Pointer | ArrayStorage::Descriptor => {
                self.emit_lda_absolute(Absolute::new(array.address));
            }
        }
        Some(())
    }

    fn emit_array_base_high_for_add(&mut self, array: StorageSlot) -> Option<()> {
        match array.array? {
            ArrayStorage::Inline => {
                self.emit_lda_immediate(Immediate::new(array.address), 1);
            }
            ArrayStorage::Pointer | ArrayStorage::Descriptor => {
                self.emit_lda_absolute(Absolute::new(array.address.wrapping_add(1)));
            }
        }
        Some(())
    }

    fn emit_adc_array_base_low(&mut self, array: StorageSlot) -> Option<()> {
        match array.array? {
            ArrayStorage::Inline => {
                self.emit_adc_immediate(Immediate::new(array.address), 0);
            }
            ArrayStorage::Pointer | ArrayStorage::Descriptor => {
                self.emit_adc_absolute(Absolute::new(array.address));
            }
        }
        Some(())
    }

    fn emit_adc_array_base_high(&mut self, array: StorageSlot) -> Option<()> {
        match array.array? {
            ArrayStorage::Inline => {
                self.emit_adc_immediate(Immediate::new(array.address), 1);
            }
            ArrayStorage::Pointer | ArrayStorage::Descriptor => {
                self.emit_adc_absolute(Absolute::new(array.address.wrapping_add(1)));
            }
        }
        Some(())
    }

    pub(super) fn emit_add_constant_to_array_addr(&mut self, offset: u16) {
        self.emit_add_constant_to_addr(runtime_zp::ARRAY_ADDR, offset);
    }

    pub(super) fn emit_add_constant_to_addr(&mut self, addr: ZeroPage, offset: u16) {
        if offset == 0 {
            return;
        }
        let immediate = Immediate::new(offset);
        self.emit_lda_zero_page(addr);
        self.emit_clc();
        self.emit_adc_immediate(immediate, 0);
        self.emit_sta_zero_page(addr);
        self.emit_lda_zero_page(addr.offset(1));
        self.emit_adc_immediate(immediate, 1);
        self.emit_sta_zero_page(addr.offset(1));
    }

    pub(super) fn emit_array_pointer_value_to_slot(
        &mut self,
        expr: &Expr,
        slot: StorageSlot,
    ) -> bool {
        if slot.size < 2 {
            return false;
        }
        if !self.emit_load_array_pointer_value_byte(expr, 1) {
            return false;
        }
        self.emit_sta_slot_byte(slot, 1);
        if !self.emit_load_array_pointer_value_byte(expr, 0) {
            return false;
        }
        self.emit_sta_slot_byte(slot, 0);
        true
    }

    pub(super) fn index_slot_with_pointer(
        &mut self,
        base: &Expr,
        index: &Expr,
        pointer: ZeroPage,
    ) -> Option<StorageSlot> {
        let ExprKind::Name(name) = &base.kind else {
            return None;
        };
        let slot = self.lookup_slot(name)?;
        if slot.pointee_size.is_some() {
            return self.pointer_index_slot_with_addr(slot, index, pointer);
        }
        slot.array?;
        if self.constant_u16(index).is_none() {
            if !self.emit_array_base_plus_scaled_byte_index_to_pointer(slot, index, pointer) {
                let temp = if pointer == runtime_zp::ADDR {
                    runtime_zp::VALUE_TEMP
                } else {
                    runtime_zp::ADDR
                };
                self.emit_complex_byte_index_array_address_to_pointer(slot, index, pointer, temp)?;
            }
            let indexed = StorageSlot::indirect_indexed_y(pointer, slot.size).signed(slot.signed);
            debug_assert_prepared_indirect_slot(indexed, pointer, "dynamic array index");
            return Some(indexed);
        }
        let index = self.constant_u16(index)?;
        let offset = index.saturating_mul(slot.size);
        match slot.array? {
            ArrayStorage::Inline => Some(StorageSlot {
                array: None,
                ..slot.offset_bytes(offset)
            }),
            ArrayStorage::Pointer | ArrayStorage::Descriptor => {
                if offset > 0 {
                    self.emit_array_base_plus_constant_to_pointer(slot, offset, pointer)?;
                } else {
                    self.emit_array_base_to_pointer(slot, pointer)?;
                }
                let indexed =
                    StorageSlot::indirect_indexed_y(pointer, slot.size).signed(slot.signed);
                debug_assert_prepared_indirect_slot(indexed, pointer, "constant array index");
                Some(indexed)
            }
        }
    }

    pub(super) fn constant_descriptor_index_slot_with_pointer(
        &mut self,
        expr: &Expr,
        pointer: ZeroPage,
    ) -> Option<StorageSlot> {
        let (base, index) = match &expr.kind {
            ExprKind::Index { base, index } => (base.as_ref(), index.as_ref()),
            ExprKind::Call { callee, args } if args.len() == 1 => (callee.as_ref(), &args[0]),
            _ => return None,
        };
        let ExprKind::Name(name) = &base.kind else {
            return None;
        };
        let array = self.lookup_slot(name)?;
        if !matches!(
            array.array?,
            ArrayStorage::Pointer | ArrayStorage::Descriptor
        ) || array.pointee_size.is_some()
        {
            return None;
        }
        let index = self.constant_u16(index)?;
        let offset = index.saturating_mul(array.size);
        if offset > 0 {
            self.emit_array_base_plus_constant_to_pointer(array, offset, pointer)?;
        } else {
            self.emit_array_base_to_pointer(array, pointer)?;
        }
        let indexed = StorageSlot::indirect_indexed_y(pointer, array.size).signed(array.signed);
        debug_assert_prepared_indirect_slot(indexed, pointer, "constant descriptor index");
        Some(indexed)
    }
}

pub(super) fn indexed_expr_index(expr: &Expr) -> Option<&Expr> {
    match &expr.kind {
        ExprKind::Index { index, .. } => Some(index),
        ExprKind::Call { args, .. } if args.len() == 1 => Some(&args[0]),
        _ => None,
    }
}
