use super::*;

impl Generator {
    pub(super) fn emit_raw_lda_slot_byte(&mut self, slot: StorageSlot, byte_index: u16) {
        debug_assert_slot_byte_access(slot, byte_index, "raw load");
        match slot.space {
            AddressSpace::Absolute => self
                .emitter
                .emit_lda_absolute(slot.absolute_byte(byte_index)),
            AddressSpace::AbsoluteX => self
                .emitter
                .emit_lda_absolute_x(AbsoluteX::new(slot.byte_address(byte_index))),
            AddressSpace::ZeroPage => self
                .emitter
                .emit_lda_zero_page(slot.zero_page_byte(byte_index)),
            AddressSpace::IndirectIndexedY => {
                self.ensure_y_imm(slot.y_index(byte_index));
                self.emitter
                    .emit_lda_indirect_indexed_y(IndirectIndexedY::new(slot.zero_page_byte(0)));
            }
        }
        self.processor
            .set_a_fact(self.slot_byte_value_fact(slot, byte_index));
    }

    pub(super) fn emit_lda_slot_byte_value_only(&mut self, slot: StorageSlot, byte_index: u16) {
        debug_assert_slot_byte_access(slot, byte_index, "value-only load");
        let value = self
            .processor
            .memory_value(slot, byte_index)
            .unwrap_or_else(|| self.slot_byte_value_fact(slot, byte_index));
        if self.profile.enables_modern_optimizations() {
            if self.processor.accumulator_value_matches(value) {
                self.record_modern_optimization(
                    CodegenOptimizationKind::RegisterReloadRemoved,
                    slot_load_instruction_len(slot),
                    None,
                    "suppressed accumulator reload where flags are not observed",
                );
                return;
            }
            if self.processor.x_value_matches(value) {
                self.record_modern_optimization(
                    CodegenOptimizationKind::RegisterReloadRemoved,
                    slot_load_instruction_len(slot).saturating_sub(1),
                    None,
                    "reused X via TXA for value-only accumulator load",
                );
                self.emit_txa();
                return;
            }
            if self.processor.y_value_matches(value) {
                self.record_modern_optimization(
                    CodegenOptimizationKind::RegisterReloadRemoved,
                    slot_load_instruction_len(slot).saturating_sub(1),
                    None,
                    "reused Y via TYA for value-only accumulator load",
                );
                self.emit_tya();
                return;
            }
        }
        self.emit_lda_slot_byte(slot, byte_index);
    }

    pub(super) fn emit_lda_zero_page_value_only(&mut self, zero_page: ZeroPage) {
        self.emit_lda_slot_byte_value_only(StorageSlot::zero_page(zero_page.address(), 1), 0);
    }

    pub(super) fn known_zero_page_scalar_immediate(&self, expr: &Expr) -> Option<u8> {
        let slot = self.direct_scalar_slot(expr)?;
        (slot.space == AddressSpace::ZeroPage && slot.size == 1)
            .then(|| self.processor.zp_value(slot.zero_page_byte(0)).immediate())?
    }

    pub(super) fn emit_increment_slot(&mut self, slot: StorageSlot, amount: u16) {
        if self.segment_storage && amount == 1 && self.emit_word_inc_peephole(slot) {
            return;
        }
        if self.segment_storage && amount == 1 && self.emit_inc_slot_peephole(slot) {
            return;
        }

        let immediate = Immediate::new(amount);
        self.emit_lda_slot_byte(slot, 0);
        self.emit_clc();
        self.emit_adc_immediate(immediate, 0);
        self.emit_sta_slot_byte(slot, 0);

        if slot.size > 1 {
            self.emit_lda_slot_byte(slot, 1);
            self.emit_adc_immediate(immediate, 1);
            self.emit_sta_slot_byte(slot, 1);
        }
    }

    pub(super) fn emit_word_inc_peephole(&mut self, slot: StorageSlot) -> bool {
        if slot.size != 2 || slot.array.is_some() {
            return false;
        }
        let done_label = self.next_label("inc:done");
        match slot.space {
            AddressSpace::Absolute => self.emit_inc_absolute(slot.absolute_byte(0)),
            AddressSpace::ZeroPage => self.emit_inc_zero_page(slot.zero_page_byte(0)),
            AddressSpace::AbsoluteX | AddressSpace::IndirectIndexedY => return false,
        }
        self.emitter
            .emit_branch_label(opcode::BNE_REL, &done_label, Span::new(0, 0));
        match slot.space {
            AddressSpace::Absolute => self.emit_inc_absolute(slot.absolute_byte(1)),
            AddressSpace::ZeroPage => self.emit_inc_zero_page(slot.zero_page_byte(1)),
            AddressSpace::AbsoluteX | AddressSpace::IndirectIndexedY => unreachable!(),
        }
        if let Some(y) = self.processor.y_immediate() {
            self.label_store_y_hints.insert(done_label.clone(), y);
        }
        self.bind_codegen_label(done_label, Span::new(0, 0));
        self.straight_line_store_y = self.processor.y_immediate();
        true
    }

    pub(super) fn emit_decrement_slot(&mut self, slot: StorageSlot, amount: u16) {
        let immediate = Immediate::new(amount);
        self.emit_lda_slot_byte(slot, 0);
        self.emit_sec();
        self.emit_sbc_immediate(immediate, 0);
        self.emit_sta_slot_byte(slot, 0);

        if slot.size > 1 {
            self.emit_lda_slot_byte(slot, 1);
            self.emit_sbc_immediate(immediate, 1);
            self.emit_sta_slot_byte(slot, 1);
        }
    }

    pub(super) fn emit_inc_slot_peephole(&mut self, slot: StorageSlot) -> bool {
        if slot.space == AddressSpace::Absolute && slot.size == 1 && slot.array.is_none() {
            self.emit_inc_absolute(Absolute::new(slot.address));
            true
        } else if slot.space == AddressSpace::ZeroPage && slot.size == 1 && slot.array.is_none() {
            self.emit_inc_zero_page(ZeroPage::new(slot.address as u8));
            true
        } else if slot.space == AddressSpace::AbsoluteX && slot.size == 1 && slot.array.is_none() {
            self.emit_inc_absolute_x(AbsoluteX::new(slot.address), slot);
            true
        } else if self.profile.enables_modern_optimizations()
            && slot.space == AddressSpace::IndirectIndexedY
            && slot.size == 1
            && slot.array.is_none()
        {
            self.emit_clc();
            self.emit_lda_slot_byte(slot, 0);
            self.emit_adc_imm(1);
            self.emit_sta_slot_byte(slot, 0);
            true
        } else {
            false
        }
    }

    pub(super) fn emit_dec_slot_peephole(&mut self, slot: StorageSlot) -> bool {
        if slot.size != 1 || slot.array.is_some() {
            return false;
        }
        match slot.space {
            AddressSpace::Absolute if self.profile.enables_modern_optimizations() => {
                self.emit_dec_absolute(Absolute::new(slot.address));
                true
            }
            AddressSpace::ZeroPage if self.profile.enables_modern_optimizations() => {
                self.emit_dec_zero_page(ZeroPage::new(slot.address as u8));
                true
            }
            AddressSpace::AbsoluteX => {
                self.emit_dec_absolute_x(AbsoluteX::new(slot.address), slot);
                true
            }
            AddressSpace::IndirectIndexedY if self.profile.enables_modern_optimizations() => {
                self.emit_sec();
                self.emit_lda_slot_byte(slot, 0);
                self.emit_sbc_imm(1);
                self.emit_sta_slot_byte(slot, 0);
                true
            }
            AddressSpace::Absolute | AddressSpace::ZeroPage | AddressSpace::IndirectIndexedY => {
                false
            }
        }
    }

    pub(super) fn emit_lda_slot_byte(&mut self, slot: StorageSlot, byte_index: u16) {
        debug_assert_slot_byte_access(slot, byte_index, "load");
        let memory_value = self.processor.memory_value(slot, byte_index);
        let value = memory_value.unwrap_or_else(|| self.slot_byte_value_fact(slot, byte_index));
        if let Some(memory_value) = memory_value
            && self.profile.enables_modern_optimizations()
            && self.processor.accumulator_matches_load_result(memory_value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                slot_load_instruction_len(slot),
                None,
                "suppressed accumulator reload from known memory alias",
            );
            return;
        }
        if self.profile.enables_modern_optimizations() {
            if self.processor.x_value_matches(value) {
                self.record_modern_optimization(
                    CodegenOptimizationKind::RegisterReloadRemoved,
                    slot_load_instruction_len(slot).saturating_sub(1),
                    None,
                    "reused X via TXA instead of reloading accumulator",
                );
                self.emit_txa();
                return;
            }
            if self.processor.y_value_matches(value) {
                self.record_modern_optimization(
                    CodegenOptimizationKind::RegisterReloadRemoved,
                    slot_load_instruction_len(slot).saturating_sub(1),
                    None,
                    "reused Y via TYA instead of reloading accumulator",
                );
                self.emit_tya();
                return;
            }
        }
        match slot.space {
            AddressSpace::Absolute => self.emit_lda_absolute(slot.absolute_byte(byte_index)),
            AddressSpace::AbsoluteX => {
                self.emit_lda_absolute_x(AbsoluteX::new(slot.byte_address(byte_index)))
            }
            AddressSpace::ZeroPage => self.emit_lda_zero_page(slot.zero_page_byte(byte_index)),
            AddressSpace::IndirectIndexedY => {
                self.ensure_y_imm(slot.y_index(byte_index));
                self.emit_lda_indirect_indexed_y(IndirectIndexedY::new(slot.zero_page_byte(0)));
            }
        }
        self.processor.set_a_fact(value);
    }

    pub(super) fn emit_adc_slot_byte(&mut self, slot: StorageSlot, byte_index: u16) {
        debug_assert_slot_byte_access(slot, byte_index, "add");
        match slot.space {
            AddressSpace::Absolute => self.emit_adc_absolute(slot.absolute_byte(byte_index)),
            AddressSpace::AbsoluteX => self.emit_adc_absolute(slot.absolute_byte(byte_index)),
            AddressSpace::ZeroPage => self.emit_adc_zero_page(slot.zero_page_byte(byte_index)),
            AddressSpace::IndirectIndexedY => {
                self.ensure_y_imm(slot.y_index(byte_index));
                self.emit_adc_indirect_indexed_y(IndirectIndexedY::new(slot.zero_page_byte(0)));
            }
        }
    }

    pub(super) fn emit_sbc_slot_byte(&mut self, slot: StorageSlot, byte_index: u16) {
        debug_assert_slot_byte_access(slot, byte_index, "subtract");
        match slot.space {
            AddressSpace::Absolute => self
                .emitter
                .emit_sbc_absolute(slot.absolute_byte(byte_index)),
            AddressSpace::AbsoluteX => self
                .emitter
                .emit_sbc_absolute(slot.absolute_byte(byte_index)),
            AddressSpace::ZeroPage => self
                .emitter
                .emit_sbc_zero_page(slot.zero_page_byte(byte_index)),
            AddressSpace::IndirectIndexedY => {
                self.ensure_y_imm(slot.y_index(byte_index));
                self.emitter
                    .emit_sbc_indirect_indexed_y(IndirectIndexedY::new(slot.zero_page_byte(0)));
            }
        }
        self.processor
            .set_a_subtract_result(self.slot_byte_value_fact(slot, byte_index));
    }

    pub(super) fn ensure_y_imm(&mut self, value: u8) {
        if self.segment_storage && self.processor.y_immediate() == Some(value) {
            return;
        }
        if self.segment_storage && self.processor.y_immediate() == Some(value.wrapping_sub(1)) {
            self.emit_iny();
            return;
        }
        if self.segment_storage && self.processor.y_immediate() == Some(value.wrapping_add(1)) {
            self.emit_dey();
            return;
        }
        self.emit_ldy_imm(value);
    }

    pub(super) fn emit_ldx_slot_byte(&mut self, slot: StorageSlot, byte_index: u16) {
        debug_assert_slot_byte_access(slot, byte_index, "load x");
        let value = self.slot_byte_value_fact(slot, byte_index);
        if self.profile.enables_modern_optimizations()
            && self.processor.x_matches_load_result(value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                slot_load_instruction_len(slot),
                None,
                "suppressed redundant X reload",
            );
            return;
        }
        if self.profile.enables_modern_optimizations()
            && self.processor.accumulator_value_matches(value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                slot_load_instruction_len(slot).saturating_sub(1),
                None,
                "reused accumulator via TAX instead of reloading X",
            );
            self.emit_tax();
            return;
        }
        match slot.space {
            AddressSpace::Absolute => self
                .emitter
                .emit_ldx_absolute(slot.absolute_byte(byte_index)),
            AddressSpace::ZeroPage => self.emit_ldx_zero_page(slot.zero_page_byte(byte_index)),
            AddressSpace::AbsoluteX | AddressSpace::IndirectIndexedY => {
                self.emitter
                    .emit_ldx_absolute(Absolute::new(slot.byte_address(byte_index)));
            }
        }
        self.processor.set_x_value_fact(value);
    }

    pub(super) fn emit_ldy_slot_byte(&mut self, slot: StorageSlot, byte_index: u16) {
        debug_assert_slot_byte_access(slot, byte_index, "load y");
        let value = self.slot_byte_value_fact(slot, byte_index);
        if self.profile.enables_modern_optimizations()
            && self.processor.y_matches_load_result(value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                slot_load_instruction_len(slot),
                None,
                "suppressed redundant Y reload",
            );
            return;
        }
        if self.profile.enables_modern_optimizations()
            && self.processor.accumulator_value_matches(value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                slot_load_instruction_len(slot).saturating_sub(1),
                None,
                "reused accumulator via TAY instead of reloading Y",
            );
            self.emit_tay();
            return;
        }
        match slot.space {
            AddressSpace::Absolute => self.emit_ldy_absolute(slot.absolute_byte(byte_index)),
            AddressSpace::ZeroPage => self.emit_ldy_zero_page(slot.zero_page_byte(byte_index)),
            AddressSpace::AbsoluteX | AddressSpace::IndirectIndexedY => {
                self.emit_ldy_absolute(Absolute::new(slot.byte_address(byte_index)));
            }
        }
        self.processor.set_y_value_fact(value);
    }

    pub(super) fn emit_raw_ldy_slot_byte(&mut self, slot: StorageSlot, byte_index: u16) {
        debug_assert_slot_byte_access(slot, byte_index, "raw load y");
        let value = self.slot_byte_value_fact(slot, byte_index);
        match slot.space {
            AddressSpace::Absolute => self
                .emitter
                .emit_ldy_absolute(slot.absolute_byte(byte_index)),
            AddressSpace::ZeroPage => self
                .emitter
                .emit_ldy_zero_page(slot.zero_page_byte(byte_index)),
            AddressSpace::AbsoluteX | AddressSpace::IndirectIndexedY => {
                self.emitter
                    .emit_ldy_absolute(Absolute::new(slot.byte_address(byte_index)));
            }
        }
        self.processor.set_y_value_fact(value);
    }

    pub(super) fn emit_cmp_slot_byte(&mut self, slot: StorageSlot, byte_index: u16) {
        debug_assert_slot_byte_access(slot, byte_index, "compare");
        match slot.space {
            AddressSpace::Absolute => {
                self.emitter
                    .emit_cmp_absolute(slot.absolute_byte(byte_index));
                self.emit_cmp_slot_fact(ValueFact::SlotByte { slot, byte_index });
            }
            AddressSpace::AbsoluteX => {
                self.emit_lda_slot_byte(slot, byte_index);
                self.emit_cmp_imm(0);
            }
            AddressSpace::ZeroPage => {
                self.emitter
                    .emit_cmp_zero_page(slot.zero_page_byte(byte_index));
                self.emit_cmp_slot_fact(ValueFact::SlotByte { slot, byte_index });
            }
            AddressSpace::IndirectIndexedY => {
                self.ensure_y_imm(slot.y_index(byte_index));
                self.emitter
                    .emit_cmp_indirect_indexed_y(IndirectIndexedY::new(slot.zero_page_byte(0)));
                self.emit_cmp_slot_fact(ValueFact::SlotByte { slot, byte_index });
            }
        }
    }

    pub(super) fn slot_byte_value_fact(&self, slot: StorageSlot, byte_index: u16) -> ValueFact {
        if let Some(value) = self.processor.memory_value(slot, byte_index) {
            return value;
        }
        if slot.space == AddressSpace::ZeroPage {
            let zero_page = slot.zero_page_byte(byte_index);
            return match self.processor.zp_value(zero_page) {
                RegisterValue::Immediate(value) => ValueFact::Immediate(value),
                RegisterValue::Fact(fact) => fact,
                RegisterValue::Unknown => ValueFact::SlotByte { slot, byte_index },
            };
        }
        ValueFact::SlotByte { slot, byte_index }
    }

    pub(super) fn emit_and_slot_byte(&mut self, slot: StorageSlot, byte_index: u16) {
        debug_assert_slot_byte_access(slot, byte_index, "and");
        match slot.space {
            AddressSpace::Absolute => {
                self.emitter
                    .emit_and_absolute(slot.absolute_byte(byte_index));
                self.processor.set_a_logic_result(
                    LogicFactOp::And,
                    self.slot_byte_value_fact(slot, byte_index),
                );
            }
            AddressSpace::AbsoluteX => {
                self.emitter
                    .emit_and_absolute_x(AbsoluteX::new(slot.byte_address(byte_index)));
                self.processor.set_a_logic_result(
                    LogicFactOp::And,
                    self.slot_byte_value_fact(slot, byte_index),
                );
            }
            AddressSpace::ZeroPage => {
                self.emitter
                    .emit_and_zero_page(slot.zero_page_byte(byte_index));
                self.processor.set_a_logic_result(
                    LogicFactOp::And,
                    self.slot_byte_value_fact(slot, byte_index),
                );
            }
            AddressSpace::IndirectIndexedY => {
                self.ensure_y_imm(slot.y_index(byte_index));
                self.emitter
                    .emit_and_indirect_indexed_y(IndirectIndexedY::new(slot.zero_page_byte(0)));
                self.processor.set_a_logic_result(
                    LogicFactOp::And,
                    self.slot_byte_value_fact(slot, byte_index),
                );
            }
        }
    }

    pub(super) fn emit_ora_slot_byte(&mut self, slot: StorageSlot, byte_index: u16) {
        debug_assert_slot_byte_access(slot, byte_index, "or");
        match slot.space {
            AddressSpace::Absolute => {
                self.emitter
                    .emit_ora_absolute(slot.absolute_byte(byte_index));
                self.processor.set_a_logic_result(
                    LogicFactOp::Or,
                    self.slot_byte_value_fact(slot, byte_index),
                );
            }
            AddressSpace::AbsoluteX => {
                self.emitter
                    .emit_ora_absolute_x(AbsoluteX::new(slot.byte_address(byte_index)));
                self.processor.set_a_logic_result(
                    LogicFactOp::Or,
                    self.slot_byte_value_fact(slot, byte_index),
                );
            }
            AddressSpace::ZeroPage => {
                self.emitter
                    .emit_ora_zero_page(slot.zero_page_byte(byte_index));
                self.processor.set_a_logic_result(
                    LogicFactOp::Or,
                    self.slot_byte_value_fact(slot, byte_index),
                );
            }
            AddressSpace::IndirectIndexedY => {
                self.ensure_y_imm(slot.y_index(byte_index));
                self.emitter
                    .emit_ora_indirect_indexed_y(IndirectIndexedY::new(slot.zero_page_byte(0)));
                self.processor.set_a_logic_result(
                    LogicFactOp::Or,
                    self.slot_byte_value_fact(slot, byte_index),
                );
            }
        }
    }

    pub(super) fn emit_eor_slot_byte(&mut self, slot: StorageSlot, byte_index: u16) {
        debug_assert_slot_byte_access(slot, byte_index, "xor");
        match slot.space {
            AddressSpace::Absolute => {
                self.emitter
                    .emit_eor_absolute(slot.absolute_byte(byte_index));
                self.processor.set_a_logic_result(
                    LogicFactOp::Xor,
                    self.slot_byte_value_fact(slot, byte_index),
                );
            }
            AddressSpace::AbsoluteX => {
                self.emitter
                    .emit_eor_absolute_x(AbsoluteX::new(slot.byte_address(byte_index)));
                self.processor.set_a_logic_result(
                    LogicFactOp::Xor,
                    self.slot_byte_value_fact(slot, byte_index),
                );
            }
            AddressSpace::ZeroPage => {
                self.emitter
                    .emit_eor_zero_page(slot.zero_page_byte(byte_index));
                self.processor.set_a_logic_result(
                    LogicFactOp::Xor,
                    self.slot_byte_value_fact(slot, byte_index),
                );
            }
            AddressSpace::IndirectIndexedY => {
                self.ensure_y_imm(slot.y_index(byte_index));
                self.emitter
                    .emit_eor_indirect_indexed_y(IndirectIndexedY::new(slot.zero_page_byte(0)));
                self.processor.set_a_logic_result(
                    LogicFactOp::Xor,
                    self.slot_byte_value_fact(slot, byte_index),
                );
            }
        }
    }
}
