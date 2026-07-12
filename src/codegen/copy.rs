use super::*;

impl Generator {
    pub(super) fn emit_slot_byte_to_zero_page(
        &mut self,
        source: StorageSlot,
        byte_index: u16,
        target: ZeroPage,
    ) {
        debug_assert_slot_byte_access(source, byte_index, "zero-page copy");
        let memory_value = self.processor.memory_value(source, byte_index);
        let value = memory_value.unwrap_or_else(|| self.slot_byte_value_fact(source, byte_index));
        if self.profile.enables_modern_optimizations() {
            if self.can_forward_recent_a_store(source, byte_index) {
                self.record_modern_optimization(
                    CodegenOptimizationKind::RegisterReloadRemoved,
                    slot_load_instruction_len(source),
                    None,
                    "stored accumulator directly instead of reloading zero-page copy source",
                );
                self.emit_sta_zero_page(target);
                return;
            }
            if self.processor.x_value_matches(value) {
                self.record_modern_optimization(
                    CodegenOptimizationKind::RegisterReloadRemoved,
                    slot_load_instruction_len(source).saturating_add(1),
                    None,
                    "stored X directly instead of transferring through accumulator",
                );
                self.emit_stx_zero_page(target);
                return;
            }
            if self.processor.y_value_matches(value) {
                self.record_modern_optimization(
                    CodegenOptimizationKind::RegisterReloadRemoved,
                    slot_load_instruction_len(source).saturating_add(1),
                    None,
                    "stored Y directly instead of transferring through accumulator",
                );
                self.emit_sty_zero_page(target);
                return;
            }
        }
        self.emit_raw_lda_slot_byte(source, byte_index);
        self.emit_sta_zero_page(target);
    }

    pub(super) fn can_forward_recent_a_store(&self, slot: StorageSlot, byte_index: u16) -> bool {
        self.profile.enables_modern_optimizations()
            && self.last_label_position != Some(self.emitter.position())
            && self.last_instruction_stored_a_to_slot_byte(slot, byte_index)
    }

    fn last_instruction_stored_a_to_slot_byte(&self, slot: StorageSlot, byte_index: u16) -> bool {
        match slot.space {
            AddressSpace::Absolute => {
                let address = slot.byte_address(byte_index);
                let bytes = &self.emitter.bytes;
                bytes.len() >= 3
                    && bytes[bytes.len() - 3] == opcode::STA_ABS
                    && bytes[bytes.len() - 2] == (address & 0x00FF) as u8
                    && bytes[bytes.len() - 1] == (address >> 8) as u8
            }
            AddressSpace::ZeroPage => {
                let address = slot.zero_page_byte(byte_index).address();
                let bytes = &self.emitter.bytes;
                bytes.len() >= 2
                    && bytes[bytes.len() - 2] == opcode::STA_ZP
                    && bytes[bytes.len() - 1] == address
            }
            AddressSpace::AbsoluteX | AddressSpace::IndirectIndexedY => false,
        }
    }

    pub(super) fn emit_store_constant(&mut self, slot: StorageSlot, value: u16) {
        let immediate = Immediate::new(value);
        if self.profile.enables_modern_optimizations()
            && slot.space == AddressSpace::ZeroPage
            && slot.size == 1
            && slot.array.is_none()
            && self.compatible_y_constant_store_slot(slot)
            && self.processor.zero_page_matches_known_byte(
                slot.zero_page_byte(0),
                ValueFact::Immediate(immediate.low()),
            )
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                2,
                None,
                format!(
                    "skipped redundant store of #${:02X} to ${:02X}",
                    immediate.low(),
                    slot.zero_page_byte(0).address()
                ),
            );
            return;
        }
        if self.emit_constant_store_reusing_register(slot, immediate.low()) {
            return;
        }
        if self.segment_storage
            && slot.size == 1
            && self.compatible_y_constant_store_slot(slot)
            && immediate.low() == 1
        {
            match self.straight_line_store_y.or(self.processor.y_immediate()) {
                Some(0) => self.emit_iny(),
                Some(1) => {}
                _ => self.emit_ldy_imm(1),
            }
            self.straight_line_store_y = Some(1);
            self.emit_sty_slot_byte(slot, 0);
            return;
        }
        if self.segment_storage
            && slot.size == 1
            && self.compatible_y_constant_store_slot(slot)
            && immediate.low() == 0
        {
            match self.straight_line_store_y.or(self.processor.y_immediate()) {
                Some(1) => self.emit_dey(),
                Some(0) => {}
                _ => self.emit_ldy_imm(0),
            }
            self.straight_line_store_y = Some(0);
            self.emit_sty_slot_byte(slot, 0);
            return;
        }

        if self.segment_storage && slot.size == 2 {
            let mut high_used_y = false;
            if matches!(immediate.high(), 0 | 1) && self.compatible_y_constant_store_slot(slot) {
                match (
                    self.straight_line_store_y.or(self.processor.y_immediate()),
                    immediate.high(),
                ) {
                    (Some(0), 1) => self.emit_iny(),
                    (Some(current), high) if current == high => {}
                    (_, high) => self.emit_ldy_imm(high),
                }
                self.straight_line_store_y = Some(immediate.high());
                self.emit_sty_slot_byte(slot, 1);
                high_used_y = true;
            } else {
                self.emit_lda_immediate(immediate, 1);
                self.emit_sta_slot_byte(slot, 1);
            }
            if high_used_y
                && matches!(immediate.low(), 0 | 1)
                && self.compatible_y_constant_store_slot(slot)
            {
                match (self.straight_line_store_y, immediate.low()) {
                    (Some(0), 1) => self.emit_iny(),
                    (Some(1), 0) => self.emit_dey(),
                    (Some(current), low) if current == low => {}
                    (_, low) => self.emit_ldy_imm(low),
                }
                self.straight_line_store_y = Some(immediate.low());
                self.emit_sty_slot_byte(slot, 0);
            } else {
                self.emit_lda_immediate(immediate, 0);
                self.emit_sta_slot_byte(slot, 0);
            }
            return;
        }

        self.emit_lda_immediate(immediate, 0);
        self.emit_sta_slot_byte(slot, 0);

        if slot.size > 1 {
            self.emit_lda_immediate(immediate, 1);
            self.emit_sta_slot_byte(slot, 1);
            self.straight_line_store_y = None;
        }
    }

    pub(super) fn emit_constant_store_reusing_register(
        &mut self,
        slot: StorageSlot,
        value: u8,
    ) -> bool {
        if !self.profile.enables_modern_optimizations()
            || slot.size != 1
            || slot.array.is_some()
            || !self.compatible_y_constant_store_slot(slot)
        {
            return false;
        }

        if self.processor.y_immediate() == Some(value) {
            self.record_modern_optimization(
                CodegenOptimizationKind::ConstantStoreReusedRegister,
                2,
                None,
                format!("reused Y=#${value:02X} for constant store"),
            );
            self.straight_line_store_y = Some(value);
            self.emit_sty_slot_byte(slot, 0);
            return true;
        }

        if self.processor.x_immediate() == Some(value) {
            self.record_modern_optimization(
                CodegenOptimizationKind::ConstantStoreReusedRegister,
                2,
                None,
                format!("reused X=#${value:02X} for constant store"),
            );
            match slot.space {
                AddressSpace::Absolute => self.emit_stx_absolute(slot.absolute_byte(0)),
                AddressSpace::ZeroPage => self.emit_stx_zero_page(slot.zero_page_byte(0)),
                AddressSpace::AbsoluteX | AddressSpace::IndirectIndexedY => return false,
            }
            self.processor.set_memory_byte_from_x(slot, 0);
            return true;
        }

        false
    }

    pub(super) fn compatible_y_constant_store_slot(&self, slot: StorageSlot) -> bool {
        match slot.space {
            AddressSpace::Absolute => true,
            AddressSpace::ZeroPage => {
                let address = slot.zero_page_byte(0).address();
                !(runtime_zp::ARGS.address()..=runtime_zp::ARGS.offset(5).address())
                    .contains(&address)
            }
            AddressSpace::AbsoluteX | AddressSpace::IndirectIndexedY => false,
        }
    }

    pub(super) fn emit_store_array_pointer_address(
        &mut self,
        target: StorageSlot,
        address: Absolute,
    ) {
        let immediate = Immediate::new(address.address());
        if self.segment_storage {
            self.emit_lda_immediate(immediate, 1);
            self.emit_sta_slot_byte(target, 1);
            self.emit_lda_immediate(immediate, 0);
            self.emit_sta_slot_byte(target, 0);
            self.processor
                .set_memory_address_word(target, address.address());
            return;
        }
        self.emit_lda_immediate(immediate, 0);
        self.emit_sta_slot_byte(target, 0);
        self.emit_lda_immediate(immediate, 1);
        self.emit_sta_slot_byte(target, 1);
        self.processor
            .set_memory_address_word(target, address.address());
    }

    pub(super) fn emit_copy_slot_to_slot(
        &mut self,
        source: StorageSlot,
        target: StorageSlot,
    ) -> bool {
        debug_assert_copy_slot_shape(source, target);
        if source == target {
            return true;
        }
        debug_assert_indirect_slots_do_not_alias(source, target, "slot copy");
        if self.segment_storage && target.size == 2 {
            if source.size > 1 {
                self.emit_copy_slot_byte_to_slot_byte(source, 1, target, 1);
            } else {
                self.emit_lda_imm(0);
                self.emit_sta_slot_byte(target, 1);
            }
            self.emit_copy_slot_byte_to_slot_byte(source, 0, target, 0);
            return true;
        }

        self.emit_copy_slot_byte_to_slot_byte(source, 0, target, 0);

        if target.size > 1 {
            if source.size > 1 {
                self.emit_copy_slot_byte_to_slot_byte(source, 1, target, 1);
            } else {
                self.emit_lda_imm(0);
                self.emit_sta_slot_byte(target, 1);
            }
        }

        true
    }

    pub(super) fn emit_copy_slot_byte_to_slot_byte(
        &mut self,
        source: StorageSlot,
        source_byte: u16,
        target: StorageSlot,
        target_byte: u16,
    ) {
        if self.can_forward_recent_a_store(source, source_byte) {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                slot_load_instruction_len(source),
                None,
                "stored accumulator directly instead of reloading slot copy source",
            );
            self.emit_sta_slot_byte(target, target_byte);
            return;
        }
        self.emit_lda_slot_byte_value_only(source, source_byte);
        self.emit_sta_slot_byte(target, target_byte);
    }

    pub(super) fn emit_sta_slot_byte(&mut self, slot: StorageSlot, byte_index: u16) {
        debug_assert_slot_byte_access(slot, byte_index, "store");
        match slot.space {
            AddressSpace::Absolute => self.emit_sta_absolute(slot.absolute_byte(byte_index)),
            AddressSpace::AbsoluteX => self
                .emitter
                .emit_sta_absolute_x(AbsoluteX::new(slot.byte_address(byte_index))),
            AddressSpace::ZeroPage => self.emit_sta_zero_page(slot.zero_page_byte(byte_index)),
            AddressSpace::IndirectIndexedY => {
                self.ensure_y_imm(slot.y_index(byte_index));
                self.emitter
                    .emit_sta_indirect_indexed_y(IndirectIndexedY::new(slot.zero_page_byte(0)));
                self.processor.invalidate_prepared_pointers();
            }
        }
        self.processor.set_memory_byte_from_a(slot, byte_index);
    }

    pub(super) fn emit_stx_slot_byte(&mut self, slot: StorageSlot, byte_index: u16) {
        debug_assert_slot_byte_access(slot, byte_index, "store x");
        match slot.space {
            AddressSpace::Absolute => self.emit_stx_absolute(slot.absolute_byte(byte_index)),
            AddressSpace::AbsoluteX | AddressSpace::ZeroPage | AddressSpace::IndirectIndexedY => {
                self.emit_stx_absolute(Absolute::new(slot.byte_address(byte_index)));
                if slot.space == AddressSpace::ZeroPage {
                    self.processor
                        .set_zp_from_x(slot.zero_page_byte(byte_index));
                }
            }
        }
        self.processor.set_memory_byte_from_x(slot, byte_index);
    }

    pub(super) fn emit_sty_slot_byte(&mut self, slot: StorageSlot, byte_index: u16) {
        debug_assert_slot_byte_access(slot, byte_index, "store y");
        match slot.space {
            AddressSpace::Absolute => self.emit_sty_absolute(slot.absolute_byte(byte_index)),
            AddressSpace::ZeroPage => self.emit_sty_zero_page(slot.zero_page_byte(byte_index)),
            AddressSpace::AbsoluteX | AddressSpace::IndirectIndexedY => {
                self.emit_sty_absolute(Absolute::new(slot.byte_address(byte_index)));
            }
        }
        self.processor.set_memory_byte_from_y(slot, byte_index);
    }
}
