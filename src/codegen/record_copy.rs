use super::*;

pub(super) const RECORD_COPY_TEMP: &str = "$ACTIONC_RECORD_COPY_TEMP";
pub(super) const RECORD_COPY_ADDRESS_TEMP: &str = "$ACTIONC_RECORD_COPY_ADDRESS";

#[derive(Debug, Clone, Default)]
pub(super) struct ClassicRecordCopyFacts {
    statements: HashMap<(String, usize, usize), u16>,
}

impl ClassicRecordCopyFacts {
    pub(super) fn insert(&mut self, scope: Option<&str>, span: Span, size: u16) {
        self.statements.insert(
            (scope.unwrap_or_default().to_owned(), span.start, span.end),
            size,
        );
    }

    pub(super) fn statement_size(&self, scope: Option<&str>, span: Span) -> Option<u16> {
        self.statements
            .get(&(scope.unwrap_or_default().to_owned(), span.start, span.end))
            .copied()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.statements.is_empty()
    }
}

impl Generator {
    pub(super) fn try_emit_record_copy_assignment(
        &mut self,
        target: &Expr,
        value: &Expr,
        span: Span,
    ) -> Option<bool> {
        let size = self
            .record_copies
            .statement_size(self.current_record_copy_scope.as_deref(), span)?;
        let scratch = self.lookup_slot(RECORD_COPY_TEMP)?;
        let saved_destination = self.lookup_slot(RECORD_COPY_ADDRESS_TEMP)?;
        if size == 0
            || scratch.size < size
            || saved_destination.size != 2
            || scratch.array.is_some()
            || saved_destination.array.is_some()
        {
            return Some(false);
        }

        // Action evaluates the destination place before the source place. Save
        // the complete effective destination address because source indexing
        // may reuse pointer scratch or execute another record copy. Keep this
        // capture on the machine stack, not in the shared copy-address cell.
        let destination = match self.lvalue_slot(target) {
            Some(destination) if destination.size == size => destination,
            _ => return Some(false),
        };
        if !self.emit_slot_address(destination, runtime_zp::ARRAY_ADDR) {
            return Some(false);
        }
        self.emit_lda_zero_page(runtime_zp::ARRAY_ADDR.offset(1));
        self.emitter.emit_pha();
        self.emit_lda_zero_page(runtime_zp::ARRAY_ADDR);
        self.emitter.emit_pha();

        let source = match self.lvalue_slot(value) {
            Some(source) if source.size == size => source,
            _ => return Some(false),
        };
        if !self.emit_slot_address(source, runtime_zp::ARRAY_ADDR) {
            return Some(false);
        }
        self.emit_pointer_to_record_scratch(runtime_zp::ARRAY_ADDR, scratch, size, span);

        self.emit_pla();
        self.emit_sta_zero_page(runtime_zp::ARRAY_ADDR);
        self.emit_pla();
        self.emit_sta_zero_page(runtime_zp::ARRAY_ADDR.offset(1));
        self.emit_record_scratch_to_pointer(scratch, runtime_zp::ARRAY_ADDR, size, span);
        Some(true)
    }

    pub(super) fn emit_slot_address(&mut self, slot: StorageSlot, pointer: ZeroPage) -> bool {
        match slot.space {
            AddressSpace::Absolute | AddressSpace::ZeroPage => {
                let immediate = slot.address_immediate();
                self.emit_lda_immediate(immediate, 0);
                self.emit_sta_zero_page(pointer);
                self.emit_lda_immediate(immediate, 1);
                self.emit_sta_zero_page(pointer.offset(1));
            }
            AddressSpace::IndirectIndexedY => {
                let source = slot.zero_page_byte(0);
                if source == pointer && slot.index_offset == 0 {
                    return true;
                }
                let offset = Immediate::new(slot.index_offset);
                self.emit_clc();
                self.emit_lda_zero_page(source);
                self.emit_adc_immediate(offset, 0);
                self.emit_sta_zero_page(pointer);
                self.emit_lda_zero_page(source.offset(1));
                self.emit_adc_immediate(offset, 1);
                self.emit_sta_zero_page(pointer.offset(1));
            }
            AddressSpace::AbsoluteX => {
                let immediate = slot.address_immediate();
                self.emit_txa();
                self.emit_clc();
                self.emit_adc_immediate(immediate, 0);
                self.emit_sta_zero_page(pointer);
                self.emit_lda_immediate(immediate, 1);
                self.emit_adc_imm(0);
                self.emit_sta_zero_page(pointer.offset(1));
            }
        }
        true
    }

    fn emit_pointer_to_record_scratch(
        &mut self,
        pointer: ZeroPage,
        scratch: StorageSlot,
        size: u16,
        span: Span,
    ) {
        let mut copied = 0u16;
        let mut remaining = size;
        while remaining >= 256 {
            let loop_label = self.next_label("record-copy:stage-page");
            self.emit_ldy_imm(0);
            self.bind_codegen_label(loop_label.clone(), span);
            self.emitter
                .emit_lda_indirect_indexed_y(IndirectIndexedY::new(pointer));
            self.emitter
                .emit_sta_absolute_y(scratch.absolute_byte(copied));
            self.emitter.emit_iny();
            self.emitter
                .emit_branch_label(opcode::BNE_REL, loop_label, span);
            self.emit_inc_zero_page(pointer.offset(1));
            copied = copied.wrapping_add(256);
            remaining -= 256;
        }
        if remaining != 0 {
            let loop_label = self.next_label("record-copy:stage-tail");
            self.emit_ldy_imm(0);
            self.bind_codegen_label(loop_label.clone(), span);
            self.emitter
                .emit_lda_indirect_indexed_y(IndirectIndexedY::new(pointer));
            self.emitter
                .emit_sta_absolute_y(scratch.absolute_byte(copied));
            self.emitter.emit_iny();
            self.emitter.emit_cpy_imm(remaining as u8);
            self.emitter
                .emit_branch_label(opcode::BNE_REL, loop_label, span);
        }
        self.record_current_absolute_write(scratch.address, size);
        self.finish_record_copy_phase(false);
    }

    fn emit_record_scratch_to_pointer(
        &mut self,
        scratch: StorageSlot,
        pointer: ZeroPage,
        size: u16,
        span: Span,
    ) {
        let mut copied = 0u16;
        let mut remaining = size;
        while remaining >= 256 {
            let loop_label = self.next_label("record-copy:store-page");
            self.emit_ldy_imm(0);
            self.bind_codegen_label(loop_label.clone(), span);
            self.emitter
                .emit_lda_absolute_y(scratch.absolute_byte(copied));
            self.emitter
                .emit_sta_indirect_indexed_y(IndirectIndexedY::new(pointer));
            self.emitter.emit_iny();
            self.emitter
                .emit_branch_label(opcode::BNE_REL, loop_label, span);
            self.emit_inc_zero_page(pointer.offset(1));
            copied = copied.wrapping_add(256);
            remaining -= 256;
        }
        if remaining != 0 {
            let loop_label = self.next_label("record-copy:store-tail");
            self.emit_ldy_imm(0);
            self.bind_codegen_label(loop_label.clone(), span);
            self.emitter
                .emit_lda_absolute_y(scratch.absolute_byte(copied));
            self.emitter
                .emit_sta_indirect_indexed_y(IndirectIndexedY::new(pointer));
            self.emitter.emit_iny();
            self.emitter.emit_cpy_imm(remaining as u8);
            self.emitter
                .emit_branch_label(opcode::BNE_REL, loop_label, span);
        }
        self.record_current_unknown_absolute_write();
        self.finish_record_copy_phase(true);
    }

    fn finish_record_copy_phase(&mut self, wrote_through_pointer: bool) {
        self.processor.invalidate_accumulator();
        self.processor.invalidate_index_y();
        self.processor.invalidate_carry();
        self.processor.invalidate_all_zp();
        self.processor.invalidate_memory();
        self.straight_line_store_y = None;
        if wrote_through_pointer {
            self.processor.invalidate_prepared_pointers();
        }
    }
}
