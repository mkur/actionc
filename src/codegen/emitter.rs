use super::*;

// Extracted from src/codegen.rs: emitter
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Emitter {
    pub(super) origin: u16,
    pub(super) bytes: Vec<u8>,
    pub(super) labels: HashMap<String, usize>,
    pub(super) patches: Vec<Patch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Patch {
    pub(super) label: String,
    pub(super) offset: usize,
    pub(super) addend: i32,
    pub(super) kind: PatchKind,
    pub(super) span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PatchKind {
    Absolute16,
    AbsoluteLow8,
    AbsoluteHigh8,
    Relative8,
}

impl Emitter {
    pub fn new() -> Self {
        Self::with_origin(CODE_ORIGIN)
    }

    pub fn with_origin(origin: u16) -> Self {
        Self {
            origin,
            ..Self::default()
        }
    }

    pub fn position(&self) -> usize {
        self.bytes.len()
    }

    pub(super) fn label_position(&self, label: &str) -> Option<usize> {
        self.labels.get(label).copied()
    }

    pub fn emit_u8(&mut self, byte: u8) {
        self.bytes.push(byte);
    }

    pub fn emit_u16_le(&mut self, value: u16) {
        self.bytes.extend(value.to_le_bytes());
    }

    pub fn emit_u16_label(&mut self, label: impl Into<String>, span: Span) {
        self.emit_u16_label_offset(label, 0, span);
    }

    pub fn emit_u16_label_offset(&mut self, label: impl Into<String>, addend: i32, span: Span) {
        let offset = self.position();
        self.emit_absolute_operand(Absolute::new(0));
        self.patches.push(Patch {
            label: label.into(),
            offset,
            addend,
            kind: PatchKind::Absolute16,
            span,
        });
    }

    pub fn emit_u8_label_low(&mut self, label: impl Into<String>, span: Span) {
        self.emit_u8_label_low_offset(label, 0, span);
    }

    pub fn emit_u8_label_low_offset(&mut self, label: impl Into<String>, addend: i32, span: Span) {
        let offset = self.position();
        self.emit_u8(0);
        self.patches.push(Patch {
            label: label.into(),
            offset,
            addend,
            kind: PatchKind::AbsoluteLow8,
            span,
        });
    }

    pub fn emit_u8_label_high(&mut self, label: impl Into<String>, span: Span) {
        self.emit_u8_label_high_offset(label, 0, span);
    }

    pub fn emit_u8_label_high_offset(&mut self, label: impl Into<String>, addend: i32, span: Span) {
        let offset = self.position();
        self.emit_u8(0);
        self.patches.push(Patch {
            label: label.into(),
            offset,
            addend,
            kind: PatchKind::AbsoluteHigh8,
            span,
        });
    }

    pub fn emit_lda_label_low(&mut self, label: impl Into<String>, span: Span) {
        self.emit_u8(opcode::LDA_IMM);
        let offset = self.position();
        self.emit_u8(0);
        self.patches.push(Patch {
            label: label.into(),
            offset,
            addend: 0,
            kind: PatchKind::AbsoluteLow8,
            span,
        });
    }

    pub fn emit_lda_label_high(&mut self, label: impl Into<String>, span: Span) {
        self.emit_u8(opcode::LDA_IMM);
        let offset = self.position();
        self.emit_u8(0);
        self.patches.push(Patch {
            label: label.into(),
            offset,
            addend: 0,
            kind: PatchKind::AbsoluteHigh8,
            span,
        });
    }

    pub fn emit_zeroes(&mut self, count: u16) {
        self.bytes
            .extend(std::iter::repeat_n(0, usize::from(count)));
    }

    pub fn emit_rts(&mut self) {
        self.emit_u8(opcode::RTS);
    }

    pub fn emit_lda_imm(&mut self, value: u8) {
        self.emit_u8(opcode::LDA_IMM);
        self.emit_u8(value);
    }

    pub fn emit_lda_immediate(&mut self, immediate: Immediate, byte_index: u16) {
        self.emit_lda_imm(immediate.byte(byte_index));
    }

    pub fn emit_lda_abs(&mut self, address: u16) {
        self.emit_u8(opcode::LDA_ABS);
        self.emit_u16_le(address);
    }

    pub fn emit_lda_absolute(&mut self, absolute: Absolute) {
        self.emit_lda_abs(absolute.address());
    }

    pub fn emit_lda_abs_x(&mut self, address: u16) {
        self.emit_u8(opcode::LDA_ABS_X);
        self.emit_u16_le(address);
    }

    pub fn emit_lda_absolute_x(&mut self, absolute_x: AbsoluteX) {
        self.emit_lda_abs_x(absolute_x.absolute().address());
    }

    pub fn emit_lda_abs_y(&mut self, address: u16) {
        self.emit_u8(opcode::LDA_ABS_Y);
        self.emit_u16_le(address);
    }

    pub fn emit_lda_absolute_y(&mut self, absolute: Absolute) {
        self.emit_lda_abs_y(absolute.address());
    }

    pub fn emit_lda_zp(&mut self, address: u8) {
        self.emit_u8(opcode::LDA_ZP);
        self.emit_u8(address);
    }

    pub fn emit_lda_zero_page(&mut self, zero_page: ZeroPage) {
        self.emit_lda_zp(zero_page.address());
    }

    pub fn emit_lda_zp_x(&mut self, address: u8) {
        self.emit_u8(opcode::LDA_ZP_X);
        self.emit_u8(address);
    }

    pub fn emit_lda_zero_page_x(&mut self, zero_page_x: ZeroPageX) {
        self.emit_lda_zp_x(zero_page_x.zero_page().address());
    }

    pub fn emit_lda_indexed_indirect_x(&mut self, indexed: IndexedIndirectX) {
        self.emit_u8(opcode::LDA_IZX);
        self.emit_u8(indexed.pointer().address());
    }

    pub fn emit_lda_indirect_indexed_y(&mut self, indexed: IndirectIndexedY) {
        self.emit_u8(opcode::LDA_IZY);
        self.emit_u8(indexed.pointer().address());
    }

    pub fn emit_ldy_imm(&mut self, value: u8) {
        self.emit_u8(opcode::LDY_IMM);
        self.emit_u8(value);
    }

    pub fn emit_ldx_imm(&mut self, value: u8) {
        self.emit_u8(opcode::LDX_IMM);
        self.emit_u8(value);
    }

    pub fn emit_ldx_abs(&mut self, address: u16) {
        self.emit_u8(opcode::LDX_ABS);
        self.emit_u16_le(address);
    }

    pub fn emit_ldx_absolute(&mut self, absolute: Absolute) {
        self.emit_ldx_abs(absolute.address());
    }

    pub fn emit_ldy_abs(&mut self, address: u16) {
        self.emit_u8(opcode::LDY_ABS);
        self.emit_u16_le(address);
    }

    pub fn emit_ldy_absolute(&mut self, absolute: Absolute) {
        self.emit_ldy_abs(absolute.address());
    }

    pub fn emit_ldx_zero_page(&mut self, zero_page: ZeroPage) {
        self.emit_u8(opcode::LDX_ZP);
        self.emit_u8(zero_page.address());
    }

    pub fn emit_ldy_zero_page(&mut self, zero_page: ZeroPage) {
        self.emit_u8(opcode::LDY_ZP);
        self.emit_u8(zero_page.address());
    }

    pub fn emit_tax(&mut self) {
        self.emit_u8(opcode::TAX);
    }

    pub fn emit_tay(&mut self) {
        self.emit_u8(opcode::TAY);
    }

    pub fn emit_iny(&mut self) {
        self.emit_u8(opcode::INY);
    }

    pub fn emit_dey(&mut self) {
        self.emit_u8(opcode::DEY);
    }

    pub fn emit_txa(&mut self) {
        self.emit_u8(opcode::TXA);
    }

    pub fn emit_tya(&mut self) {
        self.emit_u8(opcode::TYA);
    }

    pub fn emit_adc_imm(&mut self, value: u8) {
        self.emit_u8(opcode::ADC_IMM);
        self.emit_u8(value);
    }

    pub fn emit_adc_immediate(&mut self, immediate: Immediate, byte_index: u16) {
        self.emit_adc_imm(immediate.byte(byte_index));
    }

    pub fn emit_adc_abs(&mut self, address: u16) {
        self.emit_u8(opcode::ADC_ABS);
        self.emit_u16_le(address);
    }

    pub fn emit_adc_absolute(&mut self, absolute: Absolute) {
        self.emit_adc_abs(absolute.address());
    }

    pub fn emit_adc_abs_x(&mut self, address: u16) {
        self.emit_u8(opcode::ADC_ABS_X);
        self.emit_u16_le(address);
    }

    pub fn emit_adc_absolute_x(&mut self, absolute_x: AbsoluteX) {
        self.emit_adc_abs_x(absolute_x.absolute().address());
    }

    pub fn emit_adc_zp(&mut self, address: u8) {
        self.emit_u8(opcode::ADC_ZP);
        self.emit_u8(address);
    }

    pub fn emit_adc_zero_page(&mut self, zero_page: ZeroPage) {
        self.emit_adc_zp(zero_page.address());
    }

    pub fn emit_adc_indexed_indirect_x(&mut self, indexed: IndexedIndirectX) {
        self.emit_u8(opcode::ADC_IZX);
        self.emit_u8(indexed.pointer().address());
    }

    pub fn emit_adc_indirect_indexed_y(&mut self, indexed: IndirectIndexedY) {
        self.emit_u8(opcode::ADC_IZY);
        self.emit_u8(indexed.pointer().address());
    }

    pub fn emit_and_imm(&mut self, value: u8) {
        self.emit_u8(opcode::AND_IMM);
        self.emit_u8(value);
    }

    pub fn emit_and_immediate(&mut self, immediate: Immediate, byte_index: u16) {
        self.emit_and_imm(immediate.byte(byte_index));
    }

    pub fn emit_and_abs(&mut self, address: u16) {
        self.emit_u8(opcode::AND_ABS);
        self.emit_u16_le(address);
    }

    pub fn emit_and_absolute(&mut self, absolute: Absolute) {
        self.emit_and_abs(absolute.address());
    }

    pub fn emit_and_absolute_x(&mut self, absolute_x: AbsoluteX) {
        self.emit_u8(opcode::AND_ABS_X);
        self.emit_u16_le(absolute_x.absolute().address());
    }

    pub fn emit_and_zp(&mut self, address: u8) {
        self.emit_u8(opcode::AND_ZP);
        self.emit_u8(address);
    }

    pub fn emit_and_zero_page(&mut self, zero_page: ZeroPage) {
        self.emit_and_zp(zero_page.address());
    }

    pub fn emit_and_indirect_indexed_y(&mut self, indexed: IndirectIndexedY) {
        self.emit_u8(opcode::AND_IZY);
        self.emit_u8(indexed.pointer().address());
    }

    pub fn emit_ora_imm(&mut self, value: u8) {
        self.emit_u8(opcode::ORA_IMM);
        self.emit_u8(value);
    }

    pub fn emit_ora_immediate(&mut self, immediate: Immediate, byte_index: u16) {
        self.emit_ora_imm(immediate.byte(byte_index));
    }

    pub fn emit_ora_abs(&mut self, address: u16) {
        self.emit_u8(opcode::ORA_ABS);
        self.emit_u16_le(address);
    }

    pub fn emit_ora_absolute(&mut self, absolute: Absolute) {
        self.emit_ora_abs(absolute.address());
    }

    pub fn emit_ora_absolute_x(&mut self, absolute_x: AbsoluteX) {
        self.emit_u8(opcode::ORA_ABS_X);
        self.emit_u16_le(absolute_x.absolute().address());
    }

    pub fn emit_ora_zp(&mut self, address: u8) {
        self.emit_u8(opcode::ORA_ZP);
        self.emit_u8(address);
    }

    pub fn emit_ora_zero_page(&mut self, zero_page: ZeroPage) {
        self.emit_ora_zp(zero_page.address());
    }

    pub fn emit_ora_indirect_indexed_y(&mut self, indexed: IndirectIndexedY) {
        self.emit_u8(opcode::ORA_IZY);
        self.emit_u8(indexed.pointer().address());
    }

    pub fn emit_eor_imm(&mut self, value: u8) {
        self.emit_u8(opcode::EOR_IMM);
        self.emit_u8(value);
    }

    pub fn emit_eor_immediate(&mut self, immediate: Immediate, byte_index: u16) {
        self.emit_eor_imm(immediate.byte(byte_index));
    }

    pub fn emit_eor_abs(&mut self, address: u16) {
        self.emit_u8(opcode::EOR_ABS);
        self.emit_u16_le(address);
    }

    pub fn emit_eor_absolute(&mut self, absolute: Absolute) {
        self.emit_eor_abs(absolute.address());
    }

    pub fn emit_eor_absolute_x(&mut self, absolute_x: AbsoluteX) {
        self.emit_u8(opcode::EOR_ABS_X);
        self.emit_u16_le(absolute_x.absolute().address());
    }

    pub fn emit_eor_zp(&mut self, address: u8) {
        self.emit_u8(opcode::EOR_ZP);
        self.emit_u8(address);
    }

    pub fn emit_eor_zero_page(&mut self, zero_page: ZeroPage) {
        self.emit_eor_zp(zero_page.address());
    }

    pub fn emit_eor_indirect_indexed_y(&mut self, indexed: IndirectIndexedY) {
        self.emit_u8(opcode::EOR_IZY);
        self.emit_u8(indexed.pointer().address());
    }

    pub fn emit_sbc_imm(&mut self, value: u8) {
        self.emit_u8(opcode::SBC_IMM);
        self.emit_u8(value);
    }

    pub fn emit_sbc_immediate(&mut self, immediate: Immediate, byte_index: u16) {
        self.emit_sbc_imm(immediate.byte(byte_index));
    }

    pub fn emit_sbc_abs(&mut self, address: u16) {
        self.emit_u8(opcode::SBC_ABS);
        self.emit_u16_le(address);
    }

    pub fn emit_sbc_absolute(&mut self, absolute: Absolute) {
        self.emit_sbc_abs(absolute.address());
    }

    pub fn emit_sbc_zp(&mut self, address: u8) {
        self.emit_u8(opcode::SBC_ZP);
        self.emit_u8(address);
    }

    pub fn emit_sbc_zero_page(&mut self, zero_page: ZeroPage) {
        self.emit_sbc_zp(zero_page.address());
    }

    pub fn emit_sbc_indexed_indirect_x(&mut self, indexed: IndexedIndirectX) {
        self.emit_u8(opcode::SBC_IZX);
        self.emit_u8(indexed.pointer().address());
    }

    pub fn emit_sbc_indirect_indexed_y(&mut self, indexed: IndirectIndexedY) {
        self.emit_u8(opcode::SBC_IZY);
        self.emit_u8(indexed.pointer().address());
    }

    pub fn emit_cmp_imm(&mut self, value: u8) {
        self.emit_u8(opcode::CMP_IMM);
        self.emit_u8(value);
    }

    pub fn emit_cmp_immediate(&mut self, immediate: Immediate, byte_index: u16) {
        self.emit_cmp_imm(immediate.byte(byte_index));
    }

    pub fn emit_cmp_abs(&mut self, address: u16) {
        self.emit_u8(opcode::CMP_ABS);
        self.emit_u16_le(address);
    }

    pub fn emit_cmp_absolute(&mut self, absolute: Absolute) {
        self.emit_cmp_abs(absolute.address());
    }

    pub fn emit_cmp_zp(&mut self, address: u8) {
        self.emit_u8(opcode::CMP_ZP);
        self.emit_u8(address);
    }

    pub fn emit_cmp_zero_page(&mut self, zero_page: ZeroPage) {
        self.emit_cmp_zp(zero_page.address());
    }

    pub fn emit_cmp_indirect_indexed_y(&mut self, indexed: IndirectIndexedY) {
        self.emit_u8(opcode::CMP_IZY);
        self.emit_u8(indexed.pointer().address());
    }

    pub fn emit_sta_abs(&mut self, address: u16) {
        self.emit_u8(opcode::STA_ABS);
        self.emit_u16_le(address);
    }

    pub fn emit_sta_absolute(&mut self, absolute: Absolute) {
        self.emit_sta_abs(absolute.address());
    }

    pub fn emit_sta_abs_x(&mut self, address: u16) {
        self.emit_u8(opcode::STA_ABS_X);
        self.emit_u16_le(address);
    }

    pub fn emit_sta_abs_y(&mut self, address: u16) {
        self.emit_u8(opcode::STA_ABS_Y);
        self.emit_u16_le(address);
    }

    pub fn emit_sta_absolute_x(&mut self, absolute_x: AbsoluteX) {
        self.emit_sta_abs_x(absolute_x.absolute().address());
    }

    pub fn emit_sta_zp(&mut self, address: u8) {
        self.emit_u8(opcode::STA_ZP);
        self.emit_u8(address);
    }

    pub fn emit_sta_zero_page(&mut self, zero_page: ZeroPage) {
        self.emit_sta_zp(zero_page.address());
    }

    pub fn emit_sta_zp_x(&mut self, address: u8) {
        self.emit_u8(opcode::STA_ZP_X);
        self.emit_u8(address);
    }

    pub fn emit_sta_zero_page_x(&mut self, zero_page_x: ZeroPageX) {
        self.emit_sta_zp_x(zero_page_x.zero_page().address());
    }

    pub fn emit_stx_abs(&mut self, address: u16) {
        self.emit_u8(opcode::STX_ABS);
        self.emit_u16_le(address);
    }

    pub fn emit_stx_absolute(&mut self, absolute: Absolute) {
        self.emit_stx_abs(absolute.address());
    }

    pub fn emit_stx_zero_page(&mut self, zero_page: ZeroPage) {
        self.emit_u8(opcode::STX_ZP);
        self.emit_u8(zero_page.address());
    }

    pub fn emit_sty_abs(&mut self, address: u16) {
        self.emit_u8(opcode::STY_ABS);
        self.emit_u16_le(address);
    }

    pub fn emit_sty_absolute(&mut self, absolute: Absolute) {
        self.emit_sty_abs(absolute.address());
    }

    pub fn emit_sty_zero_page(&mut self, zero_page: ZeroPage) {
        self.emit_u8(opcode::STY_ZP);
        self.emit_u8(zero_page.address());
    }

    pub fn emit_inc_absolute(&mut self, absolute: Absolute) {
        self.emit_u8(opcode::INC_ABS);
        self.emit_absolute_operand(absolute);
    }

    pub fn emit_inc_absolute_x(&mut self, absolute_x: AbsoluteX) {
        self.emit_u8(opcode::INC_ABS_X);
        self.emit_absolute_operand(absolute_x.absolute());
    }

    pub fn emit_inc_zero_page(&mut self, zero_page: ZeroPage) {
        self.emit_u8(opcode::INC_ZP);
        self.emit_u8(zero_page.address());
    }

    pub fn emit_dec_absolute(&mut self, absolute: Absolute) {
        self.emit_u8(opcode::DEC_ABS);
        self.emit_absolute_operand(absolute);
    }

    pub fn emit_dec_absolute_x(&mut self, absolute_x: AbsoluteX) {
        self.emit_u8(opcode::DEC_ABS_X);
        self.emit_absolute_operand(absolute_x.absolute());
    }

    pub fn emit_dec_zero_page(&mut self, zero_page: ZeroPage) {
        self.emit_u8(opcode::DEC_ZP);
        self.emit_u8(zero_page.address());
    }

    pub fn emit_sta_indexed_indirect_x(&mut self, indexed: IndexedIndirectX) {
        self.emit_u8(opcode::STA_IZX);
        self.emit_u8(indexed.pointer().address());
    }

    pub fn emit_sta_indirect_indexed_y(&mut self, indexed: IndirectIndexedY) {
        self.emit_u8(opcode::STA_IZY);
        self.emit_u8(indexed.pointer().address());
    }

    pub fn emit_clc(&mut self) {
        self.emit_u8(opcode::CLC);
    }

    pub fn emit_sec(&mut self) {
        self.emit_u8(opcode::SEC);
    }

    pub fn emit_php(&mut self) {
        self.emit_u8(opcode::PHP);
    }

    pub fn emit_plp(&mut self) {
        self.emit_u8(opcode::PLP);
    }

    pub fn emit_pha(&mut self) {
        self.emit_u8(opcode::PHA);
    }

    pub fn emit_pla(&mut self) {
        self.emit_u8(opcode::PLA);
    }

    pub fn emit_asl_a(&mut self) {
        self.emit_u8(opcode::ASL_A);
    }

    pub fn emit_asl_absolute(&mut self, absolute: Absolute) {
        self.emit_u8(opcode::ASL_ABS);
        self.emit_absolute_operand(absolute);
    }

    pub fn emit_lsr_a(&mut self) {
        self.emit_u8(opcode::LSR_A);
    }

    pub fn emit_lsr_absolute(&mut self, absolute: Absolute) {
        self.emit_u8(opcode::LSR_ABS);
        self.emit_absolute_operand(absolute);
    }

    pub fn emit_rol_a(&mut self) {
        self.emit_u8(opcode::ROL_A);
    }

    pub fn emit_rol_absolute(&mut self, absolute: Absolute) {
        self.emit_u8(opcode::ROL_ABS);
        self.emit_absolute_operand(absolute);
    }

    pub fn emit_ror_a(&mut self) {
        self.emit_u8(opcode::ROR_A);
    }

    pub fn emit_ror_absolute(&mut self, absolute: Absolute) {
        self.emit_u8(opcode::ROR_ABS);
        self.emit_absolute_operand(absolute);
    }

    pub fn bind_label(&mut self, label: impl Into<String>, span: Span) -> Result<(), Diagnostic> {
        self.bind_label_at_position(label, self.position(), span)
    }

    pub(super) fn bind_label_at_position(
        &mut self,
        label: impl Into<String>,
        position: usize,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let label = label.into();
        if self.labels.insert(label.clone(), position).is_some() {
            return Err(Diagnostic::new(
                span,
                format!("duplicate code label `{label}`"),
            ));
        }
        Ok(())
    }

    pub fn emit_jmp_label(&mut self, label: impl Into<String>, span: Span) {
        self.emit_u8(opcode::JMP_ABS);
        let offset = self.position();
        self.emit_absolute_operand(Absolute::new(0));
        self.patches.push(Patch {
            label: label.into(),
            offset,
            addend: 0,
            kind: PatchKind::Absolute16,
            span,
        });
    }

    pub fn emit_jmp_abs(&mut self, address: u16) {
        self.emit_u8(opcode::JMP_ABS);
        self.emit_u16_le(address);
    }

    pub fn emit_jmp_indirect(&mut self, address: u16) {
        self.emit_u8(opcode::JMP_IND);
        self.emit_u16_le(address);
    }

    pub fn emit_jmp_absolute(&mut self, absolute: Absolute) {
        self.emit_jmp_abs(absolute.address());
    }

    pub fn emit_jsr_abs(&mut self, address: u16) {
        self.emit_u8(opcode::JSR_ABS);
        self.emit_u16_le(address);
    }

    pub fn emit_jsr_absolute(&mut self, absolute: Absolute) {
        self.emit_jsr_abs(absolute.address());
    }

    pub fn emit_jsr_label(&mut self, label: impl Into<String>, span: Span) {
        self.emit_u8(opcode::JSR_ABS);
        let offset = self.position();
        self.emit_absolute_operand(Absolute::new(0));
        self.patches.push(Patch {
            label: label.into(),
            offset,
            addend: 0,
            kind: PatchKind::Absolute16,
            span,
        });
    }

    pub fn emit_branch_label(&mut self, opcode: u8, label: impl Into<String>, span: Span) {
        self.emit_u8(opcode);
        let offset = self.position();
        self.emit_u8(0);
        self.patches.push(Patch {
            label: label.into(),
            offset,
            addend: 0,
            kind: PatchKind::Relative8,
            span,
        });
    }

    fn emit_absolute_operand(&mut self, absolute: Absolute) {
        self.emit_u8(absolute.low());
        self.emit_u8(absolute.high());
    }

    pub fn finish(mut self) -> Result<Vec<u8>, Vec<Diagnostic>> {
        let mut diagnostics = Vec::new();

        for patch in &self.patches {
            let Some(&target) = self.labels.get(&patch.label) else {
                diagnostics.push(Diagnostic::new(
                    patch.span,
                    format!("unknown code label `{}`", patch.label),
                ));
                continue;
            };

            match patch.kind {
                PatchKind::Absolute16 => {
                    let target = Absolute::new(
                        self.origin
                            .wrapping_add(target as u16)
                            .wrapping_add(patch.addend as u16),
                    );
                    self.bytes[patch.offset] = target.low();
                    self.bytes[patch.offset + 1] = target.high();
                }
                PatchKind::AbsoluteLow8 => {
                    let target = Absolute::new(
                        self.origin
                            .wrapping_add(target as u16)
                            .wrapping_add(patch.addend as u16),
                    );
                    self.bytes[patch.offset] = target.low();
                }
                PatchKind::AbsoluteHigh8 => {
                    let target = Absolute::new(
                        self.origin
                            .wrapping_add(target as u16)
                            .wrapping_add(patch.addend as u16),
                    );
                    self.bytes[patch.offset] = target.high();
                }
                PatchKind::Relative8 => {
                    let origin = patch.offset + 1;
                    let delta = target as isize - origin as isize;
                    if !(-128..=127).contains(&delta) {
                        diagnostics.push(Diagnostic::new(
                            patch.span,
                            format!("branch to `{}` is out of range", patch.label),
                        ));
                    } else {
                        self.bytes[patch.offset] = delta as i8 as u8;
                    }
                }
            }
        }

        if diagnostics.is_empty() {
            Ok(self.bytes)
        } else {
            Err(diagnostics)
        }
    }
}

// Generator-facing opcode wrappers keep emission and processor-state tracking together.
impl Generator {
    pub(super) fn emit_ldy_imm(&mut self, value: u8) {
        if self.profile.enables_modern_optimizations()
            && self.processor.y_immediate() == Some(value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                2,
                None,
                format!("suppressed redundant LDY #${value:02X}"),
            );
            return;
        }
        self.emitter.emit_ldy_imm(value);
        self.processor.set_y_immediate(value);
    }

    pub(super) fn emit_lda_imm(&mut self, value: u8) {
        if self.profile.enables_modern_optimizations()
            && self.processor.a_immediate() == Some(value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                2,
                None,
                format!("suppressed redundant LDA #${value:02X}"),
            );
            return;
        }
        if self.profile.enables_modern_optimizations()
            && self.processor.x_immediate() == Some(value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                1,
                None,
                format!("reused X via TXA instead of LDA #${value:02X}"),
            );
            self.emit_txa();
            return;
        }
        if self.profile.enables_modern_optimizations()
            && self.processor.y_immediate() == Some(value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                1,
                None,
                format!("reused Y via TYA instead of LDA #${value:02X}"),
            );
            self.emit_tya();
            return;
        }
        self.emitter.emit_lda_imm(value);
        self.processor.set_a_immediate(value);
    }

    pub(super) fn emit_lda_immediate(&mut self, immediate: Immediate, byte_index: u16) {
        self.emit_lda_imm(immediate.byte(byte_index));
    }

    pub(super) fn emit_lda_label_low(&mut self, label: impl Into<String>, span: Span) {
        self.emitter.emit_lda_label_low(label, span);
        self.processor.invalidate_accumulator();
    }

    pub(super) fn emit_lda_label_high(&mut self, label: impl Into<String>, span: Span) {
        self.emitter.emit_lda_label_high(label, span);
        self.processor.invalidate_accumulator();
    }

    pub(super) fn emit_lda_zero_page(&mut self, zero_page: ZeroPage) {
        let tracked_value = self.processor.zp_value(zero_page);
        let slot = StorageSlot::zero_page(zero_page.address(), 1);
        let memory_value = self.processor.memory_value(slot, 0);
        let load_value = match tracked_value {
            RegisterValue::Immediate(value) => Some(ValueFact::Immediate(value)),
            RegisterValue::Fact(fact) => Some(fact),
            RegisterValue::Unknown => memory_value,
        };
        if let Some(load_value) = load_value
            && self.profile.enables_modern_optimizations()
            && self.processor.accumulator_matches_load_result(load_value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                2,
                None,
                format!("suppressed redundant LDA ${:02X}", zero_page.address()),
            );
            return;
        }
        if let Some(load_value) = load_value
            && self.profile.enables_modern_optimizations()
            && self.processor.x_value_matches(load_value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                1,
                None,
                format!(
                    "reused X via TXA instead of LDA ${:02X}",
                    zero_page.address()
                ),
            );
            self.emit_txa();
            return;
        }
        if let Some(load_value) = load_value
            && self.profile.enables_modern_optimizations()
            && self.processor.y_value_matches(load_value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                1,
                None,
                format!(
                    "reused Y via TYA instead of LDA ${:02X}",
                    zero_page.address()
                ),
            );
            self.emit_tya();
            return;
        }
        self.emitter.emit_lda_zero_page(zero_page);
        match tracked_value {
            RegisterValue::Immediate(value) => self.processor.set_a_immediate(value),
            RegisterValue::Unknown => {
                self.processor
                    .set_a_fact(memory_value.unwrap_or(ValueFact::SlotByte {
                        slot,
                        byte_index: 0,
                    }));
            }
            RegisterValue::Fact(fact) => self.processor.set_a_fact(fact),
        }
    }

    pub(super) fn emit_lda_absolute(&mut self, absolute: Absolute) {
        let slot = StorageSlot::absolute(absolute.address(), 1);
        let memory_value = self.processor.memory_value(slot, 0);
        let load_value = memory_value.unwrap_or(ValueFact::SlotByte {
            slot,
            byte_index: 0,
        });
        if let Some(memory_value) = memory_value
            && self.profile.enables_modern_optimizations()
            && self.processor.accumulator_matches_load_result(memory_value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                3,
                None,
                "suppressed accumulator reload from known absolute memory alias",
            );
            return;
        }
        if self.profile.enables_modern_optimizations() && self.processor.x_value_matches(load_value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                2,
                None,
                "reused X via TXA instead of absolute accumulator reload",
            );
            self.emit_txa();
            return;
        }
        if self.profile.enables_modern_optimizations() && self.processor.y_value_matches(load_value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                2,
                None,
                "reused Y via TYA instead of absolute accumulator reload",
            );
            self.emit_tya();
            return;
        }
        self.emitter.emit_lda_absolute(absolute);
        self.processor.set_a_fact(load_value);
    }

    pub(super) fn emit_lda_absolute_x(&mut self, absolute_x: AbsoluteX) {
        self.emitter.emit_lda_absolute_x(absolute_x);
        self.processor.set_a_fact(ValueFact::SlotByte {
            slot: StorageSlot::absolute_x(absolute_x.absolute().address(), 1),
            byte_index: 0,
        });
    }

    pub(super) fn emit_lda_absolute_y(&mut self, absolute: Absolute) {
        self.emitter.emit_lda_absolute_y(absolute);
        self.processor.invalidate_accumulator();
    }

    pub(super) fn emit_lda_indirect_indexed_y(&mut self, indexed: IndirectIndexedY) {
        self.emitter.emit_lda_indirect_indexed_y(indexed);
        self.processor.invalidate_accumulator();
    }

    pub(super) fn emit_adc_imm(&mut self, value: u8) {
        self.emitter.emit_adc_imm(value);
        self.processor.invalidate_accumulator();
        self.processor.invalidate_carry();
    }

    pub(super) fn emit_adc_immediate(&mut self, immediate: Immediate, byte_index: u16) {
        self.emitter.emit_adc_immediate(immediate, byte_index);
        self.processor.invalidate_accumulator();
        self.processor.invalidate_carry();
    }

    pub(super) fn emit_adc_zero_page(&mut self, zero_page: ZeroPage) {
        self.emitter.emit_adc_zero_page(zero_page);
        self.processor.invalidate_accumulator();
        self.processor.invalidate_carry();
    }

    pub(super) fn emit_sbc_zero_page(&mut self, zero_page: ZeroPage) {
        self.emitter.emit_sbc_zero_page(zero_page);
        self.processor.set_a_subtract_result(ValueFact::SlotByte {
            slot: StorageSlot::zero_page(zero_page.address(), 1),
            byte_index: 0,
        });
    }

    pub(super) fn emit_adc_absolute(&mut self, absolute: Absolute) {
        self.emitter.emit_adc_absolute(absolute);
        self.processor.invalidate_accumulator();
        self.processor.invalidate_carry();
    }

    pub(super) fn emit_adc_indirect_indexed_y(&mut self, indexed: IndirectIndexedY) {
        self.emitter.emit_adc_indirect_indexed_y(indexed);
        self.processor.invalidate_accumulator();
        self.processor.invalidate_carry();
    }

    pub(super) fn emit_sbc_imm(&mut self, value: u8) {
        self.emitter.emit_sbc_imm(value);
        self.processor
            .set_a_subtract_result(ValueFact::Immediate(value));
    }

    pub(super) fn emit_sbc_immediate(&mut self, immediate: Immediate, byte_index: u16) {
        self.emit_sbc_imm(immediate.byte(byte_index));
    }

    pub(super) fn emit_sbc_absolute(&mut self, absolute: Absolute) {
        self.emitter.emit_sbc_absolute(absolute);
        self.processor.set_a_subtract_result(ValueFact::SlotByte {
            slot: StorageSlot::absolute(absolute.address(), 1),
            byte_index: 0,
        });
    }

    pub(super) fn emit_cmp_imm(&mut self, value: u8) {
        self.emitter.emit_cmp_imm(value);
        self.processor.set_compare_flags(CompareFact::Byte {
            left: self.processor.a_value_fact(),
            right: ValueFact::Immediate(value),
        });
    }

    pub(super) fn emit_cmp_immediate(&mut self, immediate: Immediate, byte_index: u16) {
        self.emit_cmp_imm(immediate.byte(byte_index));
    }

    pub(super) fn emit_cmp_slot_fact(&mut self, value: ValueFact) {
        self.processor.set_compare_flags(CompareFact::Byte {
            left: self.processor.a_value_fact(),
            right: value,
        });
    }

    pub(super) fn emit_and_imm(&mut self, value: u8) {
        self.emitter.emit_and_imm(value);
        self.processor
            .set_a_logic_result(LogicFactOp::And, ValueFact::Immediate(value));
    }

    pub(super) fn emit_and_immediate(&mut self, immediate: Immediate, byte_index: u16) {
        self.emit_and_imm(immediate.byte(byte_index));
    }

    pub(super) fn emit_ora_imm(&mut self, value: u8) {
        self.emitter.emit_ora_imm(value);
        self.processor
            .set_a_logic_result(LogicFactOp::Or, ValueFact::Immediate(value));
    }

    pub(super) fn emit_ora_immediate(&mut self, immediate: Immediate, byte_index: u16) {
        self.emit_ora_imm(immediate.byte(byte_index));
    }

    pub(super) fn emit_eor_imm(&mut self, value: u8) {
        self.emitter.emit_eor_imm(value);
        self.processor
            .set_a_logic_result(LogicFactOp::Xor, ValueFact::Immediate(value));
    }

    pub(super) fn emit_eor_immediate(&mut self, immediate: Immediate, byte_index: u16) {
        self.emit_eor_imm(immediate.byte(byte_index));
    }

    pub(super) fn emit_asl_a(&mut self) {
        self.emitter.emit_asl_a();
        self.processor.clear_a();
        self.processor.invalidate_carry();
    }

    pub(super) fn emit_lsr_a(&mut self) {
        self.emitter.emit_lsr_a();
        self.processor.clear_a();
        self.processor.invalidate_carry();
    }

    pub(super) fn emit_rol_a(&mut self) {
        self.emitter.emit_rol_a();
        self.processor.clear_a();
        self.processor.invalidate_carry();
    }

    pub(super) fn emit_ror_a(&mut self) {
        self.emitter.emit_ror_a();
        self.processor.clear_a();
        self.processor.invalidate_carry();
    }

    pub(super) fn emit_ldx_imm(&mut self, value: u8) {
        if self.profile.enables_modern_optimizations()
            && self.processor.x_immediate() == Some(value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                2,
                None,
                format!("suppressed redundant LDX #${value:02X}"),
            );
            return;
        }
        self.emitter.emit_ldx_imm(value);
        self.processor.set_x_immediate(value);
    }

    pub(super) fn emit_ldx_zero_page(&mut self, zero_page: ZeroPage) {
        let tracked_value = self.processor.zp_value(zero_page);
        let slot = StorageSlot::zero_page(zero_page.address(), 1);
        let memory_value = self.processor.memory_value(slot, 0);
        let load_value = match tracked_value {
            RegisterValue::Immediate(value) => Some(ValueFact::Immediate(value)),
            RegisterValue::Fact(fact) => Some(fact),
            RegisterValue::Unknown => memory_value,
        };
        if let Some(load_value) = load_value
            && self.profile.enables_modern_optimizations()
            && self.processor.x_matches_load_result(load_value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                2,
                None,
                format!("suppressed redundant LDX ${:02X}", zero_page.address()),
            );
            return;
        }
        if let Some(load_value) = load_value
            && self.profile.enables_modern_optimizations()
            && self.processor.accumulator_value_matches(load_value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                1,
                None,
                format!(
                    "reused accumulator via TAX instead of LDX ${:02X}",
                    zero_page.address()
                ),
            );
            self.emit_tax();
            return;
        }
        self.emitter.emit_ldx_zero_page(zero_page);
        match tracked_value {
            RegisterValue::Immediate(value) => self.processor.set_x_immediate(value),
            RegisterValue::Fact(fact) => self.processor.set_x_fact(fact),
            RegisterValue::Unknown => {
                self.processor
                    .set_x_fact(memory_value.unwrap_or(ValueFact::SlotByte {
                        slot,
                        byte_index: 0,
                    }));
            }
        }
    }

    pub(super) fn emit_ldy_zero_page(&mut self, zero_page: ZeroPage) {
        let tracked_value = self.processor.zp_value(zero_page);
        let slot = StorageSlot::zero_page(zero_page.address(), 1);
        let memory_value = self.processor.memory_value(slot, 0);
        let load_value = match tracked_value {
            RegisterValue::Immediate(value) => Some(ValueFact::Immediate(value)),
            RegisterValue::Fact(fact) => Some(fact),
            RegisterValue::Unknown => memory_value,
        };
        if let Some(load_value) = load_value
            && self.profile.enables_modern_optimizations()
            && self.processor.y_matches_load_result(load_value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                2,
                None,
                format!("suppressed redundant LDY ${:02X}", zero_page.address()),
            );
            return;
        }
        if let Some(load_value) = load_value
            && self.profile.enables_modern_optimizations()
            && self.processor.accumulator_value_matches(load_value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                1,
                None,
                format!(
                    "reused accumulator via TAY instead of LDY ${:02X}",
                    zero_page.address()
                ),
            );
            self.emit_tay();
            return;
        }
        self.emitter.emit_ldy_zero_page(zero_page);
        match tracked_value {
            RegisterValue::Immediate(value) => self.processor.set_y_immediate(value),
            RegisterValue::Fact(fact) => self.processor.set_y_fact(fact),
            RegisterValue::Unknown => {
                self.processor
                    .set_y_fact(memory_value.unwrap_or(ValueFact::SlotByte {
                        slot,
                        byte_index: 0,
                    }));
            }
        }
    }

    pub(super) fn emit_ldy_absolute(&mut self, absolute: Absolute) {
        let slot = StorageSlot::absolute(absolute.address(), 1);
        let memory_value = self.processor.memory_value(slot, 0);
        let load_value = memory_value.unwrap_or(ValueFact::SlotByte {
            slot,
            byte_index: 0,
        });
        if let Some(memory_value) = memory_value
            && self.profile.enables_modern_optimizations()
            && self.processor.y_matches_load_result(memory_value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                3,
                None,
                "suppressed Y reload from known absolute memory alias",
            );
            return;
        }
        if self.profile.enables_modern_optimizations()
            && self.processor.accumulator_value_matches(load_value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                2,
                None,
                "reused accumulator via TAY instead of absolute Y reload",
            );
            self.emit_tay();
            return;
        }
        self.emitter.emit_ldy_absolute(absolute);
        self.processor.set_y_fact(load_value);
    }

    pub(super) fn emit_tay(&mut self) {
        self.emitter.emit_tay();
        self.processor
            .set_y_fact(self.processor.a.value_fact(RegisterName::A));
    }

    pub(super) fn emit_tax(&mut self) {
        self.emitter.emit_tax();
        self.processor
            .set_x_fact(self.processor.a.value_fact(RegisterName::A));
    }

    pub(super) fn emit_txa(&mut self) {
        self.emitter.emit_txa();
        self.processor
            .set_a_fact(self.processor.x.value_fact(RegisterName::X));
    }

    pub(super) fn emit_tya(&mut self) {
        self.emitter.emit_tya();
        self.processor
            .set_a_fact(self.processor.y.value_fact(RegisterName::Y));
    }

    pub(super) fn emit_pla(&mut self) {
        self.emitter.emit_pla();
        self.processor.invalidate_accumulator();
    }

    pub(super) fn emit_clc(&mut self) {
        self.emitter.emit_clc();
        self.processor.set_carry(false);
    }

    pub(super) fn emit_sec(&mut self) {
        self.emitter.emit_sec();
        self.processor.set_carry(true);
    }

    pub(super) fn emit_plp(&mut self) {
        self.emitter.emit_plp();
        self.processor.invalidate_carry();
    }

    pub(super) fn emit_sta_zero_page(&mut self, zero_page: ZeroPage) {
        self.emitter.emit_sta_zero_page(zero_page);
        self.record_current_zero_page_write(zero_page);
        self.processor.set_zp_from_a(zero_page);
        self.processor
            .set_memory_byte_from_a(StorageSlot::zero_page(zero_page.address(), 1), 0);
    }

    pub(super) fn emit_sta_absolute(&mut self, absolute: Absolute) {
        self.emitter.emit_sta_absolute(absolute);
        self.record_current_absolute_write(absolute.address(), 1);
        self.processor
            .invalidate_prepared_pointers_touching_range(absolute.address(), 1);
        if let Some(zero_page) = absolute_zero_page_alias(absolute) {
            self.processor.set_zp_from_a(zero_page);
        }
        self.processor
            .set_memory_byte_from_a(StorageSlot::absolute(absolute.address(), 1), 0);
    }

    pub(super) fn emit_stx_absolute(&mut self, absolute: Absolute) {
        self.emitter.emit_stx_absolute(absolute);
        self.record_current_absolute_write(absolute.address(), 1);
        self.processor
            .invalidate_prepared_pointers_touching_range(absolute.address(), 1);
        if let Some(zero_page) = absolute_zero_page_alias(absolute) {
            self.processor.set_zp_from_x(zero_page);
        }
        self.processor
            .set_memory_byte_from_x(StorageSlot::absolute(absolute.address(), 1), 0);
    }

    pub(super) fn emit_stx_zero_page(&mut self, zero_page: ZeroPage) {
        self.emitter.emit_stx_zero_page(zero_page);
        self.record_current_zero_page_write(zero_page);
        self.processor.set_zp_from_x(zero_page);
        self.processor
            .set_memory_byte_from_x(StorageSlot::zero_page(zero_page.address(), 1), 0);
    }

    pub(super) fn emit_sty_zero_page(&mut self, zero_page: ZeroPage) {
        self.emitter.emit_sty_zero_page(zero_page);
        self.record_current_zero_page_write(zero_page);
        self.processor.set_zp_from_y(zero_page);
        self.processor
            .set_memory_byte_from_y(StorageSlot::zero_page(zero_page.address(), 1), 0);
    }

    pub(super) fn emit_sty_absolute(&mut self, absolute: Absolute) {
        self.emitter.emit_sty_absolute(absolute);
        self.record_current_absolute_write(absolute.address(), 1);
        self.processor
            .invalidate_prepared_pointers_touching_range(absolute.address(), 1);
        if let Some(zero_page) = absolute_zero_page_alias(absolute) {
            self.processor.set_zp_from_y(zero_page);
        }
        self.processor
            .set_memory_byte_from_y(StorageSlot::absolute(absolute.address(), 1), 0);
    }

    pub(super) fn emit_inc_zero_page(&mut self, zero_page: ZeroPage) {
        self.emitter.emit_inc_zero_page(zero_page);
        self.record_current_zero_page_write(zero_page);
        self.processor.invalidate_zp(zero_page);
        self.processor
            .invalidate_memory_byte(StorageSlot::zero_page(zero_page.address(), 1), 0);
    }

    pub(super) fn emit_inc_absolute(&mut self, absolute: Absolute) {
        self.emitter.emit_inc_absolute(absolute);
        self.record_current_absolute_write(absolute.address(), 1);
        self.processor
            .invalidate_prepared_pointers_touching_range(absolute.address(), 1);
        if let Some(zero_page) = absolute_zero_page_alias(absolute) {
            self.processor.invalidate_zp(zero_page);
        }
        self.processor
            .invalidate_memory_byte(StorageSlot::absolute(absolute.address(), 1), 0);
    }

    pub(super) fn emit_inc_absolute_x(&mut self, absolute_x: AbsoluteX, slot: StorageSlot) {
        self.emitter.emit_inc_absolute_x(absolute_x);
        self.record_current_unknown_absolute_write();
        self.processor.invalidate_memory_byte(slot, 0);
    }

    pub(super) fn emit_dec_absolute_x(&mut self, absolute_x: AbsoluteX, slot: StorageSlot) {
        self.emitter.emit_dec_absolute_x(absolute_x);
        self.record_current_unknown_absolute_write();
        self.processor.invalidate_memory_byte(slot, 0);
    }

    pub(super) fn emit_dec_zero_page(&mut self, zero_page: ZeroPage) {
        self.emitter.emit_dec_zero_page(zero_page);
        self.record_current_zero_page_write(zero_page);
        self.processor.invalidate_zp(zero_page);
        self.processor
            .invalidate_memory_byte(StorageSlot::zero_page(zero_page.address(), 1), 0);
    }

    pub(super) fn emit_dec_absolute(&mut self, absolute: Absolute) {
        self.emitter.emit_dec_absolute(absolute);
        self.record_current_absolute_write(absolute.address(), 1);
        self.processor
            .invalidate_prepared_pointers_touching_range(absolute.address(), 1);
        if let Some(zero_page) = absolute_zero_page_alias(absolute) {
            self.processor.invalidate_zp(zero_page);
        }
        self.processor
            .invalidate_memory_byte(StorageSlot::absolute(absolute.address(), 1), 0);
    }

    pub(super) fn emit_jmp_label(&mut self, label: impl Into<String>, span: Span) {
        self.emitter.emit_jmp_label(label, span);
        self.processor.invalidate_after_jump();
        self.straight_line_store_y = None;
    }

    pub(super) fn emit_iny(&mut self) {
        self.emitter.emit_iny();
        match self.processor.y_immediate() {
            Some(value) => self.processor.set_y_immediate(value.wrapping_add(1)),
            None => self.processor.invalidate_index_y(),
        }
    }

    pub(super) fn emit_dey(&mut self) {
        self.emitter.emit_dey();
        match self.processor.y_immediate() {
            Some(value) => self.processor.set_y_immediate(value.wrapping_sub(1)),
            None => self.processor.invalidate_index_y(),
        }
    }

    pub(super) fn emit_sta_absolute_label(&mut self, label: impl Into<String>, span: Span) {
        let label = label.into();
        self.emitter.emit_u8(opcode::STA_ABS);
        self.emitter.emit_u16_label(label.clone(), span);
        self.record_current_unknown_absolute_write();
        self.label_byte_values
            .insert(label, self.processor.a_value_fact());
    }

    pub(super) fn emit_lda_absolute_label(&mut self, label: impl Into<String>, span: Span) {
        let label = label.into();
        if let Some(value) = self.label_byte_values.get(&label).copied()
            && self.profile.enables_modern_optimizations()
            && self.processor.accumulator_matches_load_result(value)
        {
            self.record_modern_optimization(
                CodegenOptimizationKind::RegisterReloadRemoved,
                3,
                None,
                "suppressed accumulator reload from tracked label byte",
            );
            return;
        }
        self.emitter.emit_u8(opcode::LDA_ABS);
        self.emitter.emit_u16_label(label.clone(), span);
        if let Some(value) = self.label_byte_values.get(&label).copied() {
            self.processor.set_a_fact(value);
        } else {
            self.processor.invalidate_accumulator();
        }
    }
}

pub fn format_listing(bytes: &[u8]) -> String {
    format_listing_with_origin(bytes, CODE_ORIGIN)
}

pub fn format_listing_with_origin(bytes: &[u8], origin: u16) -> String {
    disassemble_with_origin(bytes, origin)
        .into_iter()
        .map(|line| {
            let raw = format_instruction_bytes(&line.bytes);
            format!("{:04X}  {raw:<8}  {}", line.address, line.text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn disassemble_with_origin(bytes: &[u8], origin: u16) -> Vec<DisassembledInstruction> {
    disassemble_with_origin_and_inline_jsr_data(bytes, origin, default_inline_jsr_data_len)
}

pub fn disassemble_with_origin_and_inline_jsr_data(
    bytes: &[u8],
    origin: u16,
    inline_jsr_data_len: impl Fn(u16) -> Option<usize>,
) -> Vec<DisassembledInstruction> {
    let mut lines = Vec::new();
    let mut pc = 0usize;

    while pc < bytes.len() {
        let opcode = bytes[pc];
        let Some(instruction) = decode_instruction(opcode) else {
            lines.push(DisassembledInstruction {
                address: origin.wrapping_add(pc as u16),
                bytes: vec![opcode],
                mnemonic: ".BYTE",
                mode: None,
                operands: vec![opcode],
                text: format!(".BYTE ${opcode:02X}"),
            });
            pc += 1;
            continue;
        };

        let available = bytes.len() - pc;
        if available < instruction.len {
            let truncated = bytes[pc..].to_vec();
            let text = format!(
                ".BYTE {}",
                truncated
                    .iter()
                    .map(|byte| format!("${byte:02X}"))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            lines.push(DisassembledInstruction {
                address: origin.wrapping_add(pc as u16),
                bytes: truncated.clone(),
                mnemonic: ".BYTE",
                mode: None,
                operands: truncated,
                text,
            });
            break;
        }

        let operands = &bytes[pc + 1..pc + instruction.len];
        let address = origin.wrapping_add(pc as u16);
        let asm = format_instruction(instruction, operands, address);
        lines.push(DisassembledInstruction {
            address,
            bytes: bytes[pc..pc + instruction.len].to_vec(),
            mnemonic: instruction.mnemonic,
            mode: Some(instruction.mode),
            operands: operands.to_vec(),
            text: asm,
        });
        pc += instruction.len;

        let inline_data_len = absolute_jsr_target(instruction, operands).and_then(|target| {
            inline_jsr_data_len(target).or_else(|| default_inline_jsr_data_len(target))
        });
        if let Some(inline_data_len) = inline_data_len
            && bytes.len().saturating_sub(pc) >= inline_data_len
        {
            let metadata = &bytes[pc..pc + inline_data_len];
            let metadata_address = origin.wrapping_add(pc as u16);
            lines.push(DisassembledInstruction {
                address: metadata_address,
                bytes: metadata.to_vec(),
                mnemonic: ".BYTE",
                mode: None,
                operands: metadata.to_vec(),
                text: format!(
                    ".BYTE {}",
                    metadata
                        .iter()
                        .map(|byte| format!("${byte:02X}"))
                        .collect::<Vec<_>>()
                        .join(",")
                ),
            });
            pc += inline_data_len;
        }
    }

    lines
}

fn default_inline_jsr_data_len(address: u16) -> Option<usize> {
    (address == runtime_helper::CARTRIDGE_SARGS.address()
        || address == runtime_helper::SARGS_SLOT.address())
    .then_some(3)
}

fn absolute_jsr_target(instruction: Instruction, operands: &[u8]) -> Option<u16> {
    (instruction.mnemonic == "JSR" && instruction.mode == AddressingMode::Absolute)
        .then(|| le_u16(operands))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisassembledInstruction {
    pub address: u16,
    pub bytes: Vec<u8>,
    pub mnemonic: &'static str,
    pub mode: Option<AddressingMode>,
    pub operands: Vec<u8>,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Instruction {
    pub(super) mnemonic: &'static str,
    pub(super) mode: AddressingMode,
    pub(super) len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressingMode {
    Implied,
    Accumulator,
    Immediate,
    ZeroPage,
    ZeroPageX,
    ZeroPageY,
    Absolute,
    AbsoluteX,
    AbsoluteY,
    Indirect,
    IndexedIndirectX,
    IndirectIndexedY,
    Relative,
}

pub(super) fn decode_instruction(opcode: u8) -> Option<Instruction> {
    let instruction = match opcode {
        0x00 => Instruction::new("BRK", AddressingMode::Implied),
        0x01 => Instruction::new("ORA", AddressingMode::IndexedIndirectX),
        0x05 => Instruction::new("ORA", AddressingMode::ZeroPage),
        0x06 => Instruction::new("ASL", AddressingMode::ZeroPage),
        0x08 => Instruction::new("PHP", AddressingMode::Implied),
        0x09 => Instruction::new("ORA", AddressingMode::Immediate),
        0x0A => Instruction::new("ASL", AddressingMode::Accumulator),
        0x0D => Instruction::new("ORA", AddressingMode::Absolute),
        0x0E => Instruction::new("ASL", AddressingMode::Absolute),
        0x10 => Instruction::new("BPL", AddressingMode::Relative),
        0x11 => Instruction::new("ORA", AddressingMode::IndirectIndexedY),
        0x15 => Instruction::new("ORA", AddressingMode::ZeroPageX),
        0x16 => Instruction::new("ASL", AddressingMode::ZeroPageX),
        0x18 => Instruction::new("CLC", AddressingMode::Implied),
        0x19 => Instruction::new("ORA", AddressingMode::AbsoluteY),
        0x1D => Instruction::new("ORA", AddressingMode::AbsoluteX),
        0x1E => Instruction::new("ASL", AddressingMode::AbsoluteX),
        0x20 => Instruction::new("JSR", AddressingMode::Absolute),
        0x21 => Instruction::new("AND", AddressingMode::IndexedIndirectX),
        0x24 => Instruction::new("BIT", AddressingMode::ZeroPage),
        0x25 => Instruction::new("AND", AddressingMode::ZeroPage),
        0x26 => Instruction::new("ROL", AddressingMode::ZeroPage),
        0x28 => Instruction::new("PLP", AddressingMode::Implied),
        0x29 => Instruction::new("AND", AddressingMode::Immediate),
        0x2A => Instruction::new("ROL", AddressingMode::Accumulator),
        0x2C => Instruction::new("BIT", AddressingMode::Absolute),
        0x2D => Instruction::new("AND", AddressingMode::Absolute),
        0x2E => Instruction::new("ROL", AddressingMode::Absolute),
        0x30 => Instruction::new("BMI", AddressingMode::Relative),
        0x31 => Instruction::new("AND", AddressingMode::IndirectIndexedY),
        0x35 => Instruction::new("AND", AddressingMode::ZeroPageX),
        0x36 => Instruction::new("ROL", AddressingMode::ZeroPageX),
        0x38 => Instruction::new("SEC", AddressingMode::Implied),
        0x39 => Instruction::new("AND", AddressingMode::AbsoluteY),
        0x3D => Instruction::new("AND", AddressingMode::AbsoluteX),
        0x3E => Instruction::new("ROL", AddressingMode::AbsoluteX),
        0x40 => Instruction::new("RTI", AddressingMode::Implied),
        0x41 => Instruction::new("EOR", AddressingMode::IndexedIndirectX),
        0x45 => Instruction::new("EOR", AddressingMode::ZeroPage),
        0x46 => Instruction::new("LSR", AddressingMode::ZeroPage),
        0x48 => Instruction::new("PHA", AddressingMode::Implied),
        0x49 => Instruction::new("EOR", AddressingMode::Immediate),
        0x4A => Instruction::new("LSR", AddressingMode::Accumulator),
        0x4C => Instruction::new("JMP", AddressingMode::Absolute),
        0x4D => Instruction::new("EOR", AddressingMode::Absolute),
        0x4E => Instruction::new("LSR", AddressingMode::Absolute),
        0x50 => Instruction::new("BVC", AddressingMode::Relative),
        0x51 => Instruction::new("EOR", AddressingMode::IndirectIndexedY),
        0x55 => Instruction::new("EOR", AddressingMode::ZeroPageX),
        0x56 => Instruction::new("LSR", AddressingMode::ZeroPageX),
        0x58 => Instruction::new("CLI", AddressingMode::Implied),
        0x59 => Instruction::new("EOR", AddressingMode::AbsoluteY),
        0x5D => Instruction::new("EOR", AddressingMode::AbsoluteX),
        0x5E => Instruction::new("LSR", AddressingMode::AbsoluteX),
        0x60 => Instruction::new("RTS", AddressingMode::Implied),
        0x61 => Instruction::new("ADC", AddressingMode::IndexedIndirectX),
        0x65 => Instruction::new("ADC", AddressingMode::ZeroPage),
        0x66 => Instruction::new("ROR", AddressingMode::ZeroPage),
        0x68 => Instruction::new("PLA", AddressingMode::Implied),
        0x69 => Instruction::new("ADC", AddressingMode::Immediate),
        0x6A => Instruction::new("ROR", AddressingMode::Accumulator),
        0x6C => Instruction::new("JMP", AddressingMode::Indirect),
        0x6D => Instruction::new("ADC", AddressingMode::Absolute),
        0x6E => Instruction::new("ROR", AddressingMode::Absolute),
        0x70 => Instruction::new("BVS", AddressingMode::Relative),
        0x71 => Instruction::new("ADC", AddressingMode::IndirectIndexedY),
        0x75 => Instruction::new("ADC", AddressingMode::ZeroPageX),
        0x76 => Instruction::new("ROR", AddressingMode::ZeroPageX),
        0x78 => Instruction::new("SEI", AddressingMode::Implied),
        0x79 => Instruction::new("ADC", AddressingMode::AbsoluteY),
        0x7D => Instruction::new("ADC", AddressingMode::AbsoluteX),
        0x7E => Instruction::new("ROR", AddressingMode::AbsoluteX),
        0x81 => Instruction::new("STA", AddressingMode::IndexedIndirectX),
        0x84 => Instruction::new("STY", AddressingMode::ZeroPage),
        0x85 => Instruction::new("STA", AddressingMode::ZeroPage),
        0x86 => Instruction::new("STX", AddressingMode::ZeroPage),
        0x88 => Instruction::new("DEY", AddressingMode::Implied),
        0x8A => Instruction::new("TXA", AddressingMode::Implied),
        0x8C => Instruction::new("STY", AddressingMode::Absolute),
        0x8D => Instruction::new("STA", AddressingMode::Absolute),
        0x8E => Instruction::new("STX", AddressingMode::Absolute),
        0x90 => Instruction::new("BCC", AddressingMode::Relative),
        0x91 => Instruction::new("STA", AddressingMode::IndirectIndexedY),
        0x94 => Instruction::new("STY", AddressingMode::ZeroPageX),
        0x95 => Instruction::new("STA", AddressingMode::ZeroPageX),
        0x96 => Instruction::new("STX", AddressingMode::ZeroPageY),
        0x98 => Instruction::new("TYA", AddressingMode::Implied),
        0x99 => Instruction::new("STA", AddressingMode::AbsoluteY),
        0x9A => Instruction::new("TXS", AddressingMode::Implied),
        0x9D => Instruction::new("STA", AddressingMode::AbsoluteX),
        0xA0 => Instruction::new("LDY", AddressingMode::Immediate),
        0xA1 => Instruction::new("LDA", AddressingMode::IndexedIndirectX),
        0xA2 => Instruction::new("LDX", AddressingMode::Immediate),
        0xA4 => Instruction::new("LDY", AddressingMode::ZeroPage),
        0xA5 => Instruction::new("LDA", AddressingMode::ZeroPage),
        0xA6 => Instruction::new("LDX", AddressingMode::ZeroPage),
        0xA8 => Instruction::new("TAY", AddressingMode::Implied),
        0xA9 => Instruction::new("LDA", AddressingMode::Immediate),
        0xAA => Instruction::new("TAX", AddressingMode::Implied),
        0xAC => Instruction::new("LDY", AddressingMode::Absolute),
        0xAD => Instruction::new("LDA", AddressingMode::Absolute),
        0xAE => Instruction::new("LDX", AddressingMode::Absolute),
        0xB0 => Instruction::new("BCS", AddressingMode::Relative),
        0xB1 => Instruction::new("LDA", AddressingMode::IndirectIndexedY),
        0xB4 => Instruction::new("LDY", AddressingMode::ZeroPageX),
        0xB5 => Instruction::new("LDA", AddressingMode::ZeroPageX),
        0xB6 => Instruction::new("LDX", AddressingMode::ZeroPageY),
        0xB8 => Instruction::new("CLV", AddressingMode::Implied),
        0xB9 => Instruction::new("LDA", AddressingMode::AbsoluteY),
        0xBA => Instruction::new("TSX", AddressingMode::Implied),
        0xBC => Instruction::new("LDY", AddressingMode::AbsoluteX),
        0xBD => Instruction::new("LDA", AddressingMode::AbsoluteX),
        0xBE => Instruction::new("LDX", AddressingMode::AbsoluteY),
        0xC0 => Instruction::new("CPY", AddressingMode::Immediate),
        0xC1 => Instruction::new("CMP", AddressingMode::IndexedIndirectX),
        0xC4 => Instruction::new("CPY", AddressingMode::ZeroPage),
        0xC5 => Instruction::new("CMP", AddressingMode::ZeroPage),
        0xC6 => Instruction::new("DEC", AddressingMode::ZeroPage),
        0xC8 => Instruction::new("INY", AddressingMode::Implied),
        0xC9 => Instruction::new("CMP", AddressingMode::Immediate),
        0xCA => Instruction::new("DEX", AddressingMode::Implied),
        0xCC => Instruction::new("CPY", AddressingMode::Absolute),
        0xCD => Instruction::new("CMP", AddressingMode::Absolute),
        0xCE => Instruction::new("DEC", AddressingMode::Absolute),
        0xD0 => Instruction::new("BNE", AddressingMode::Relative),
        0xD1 => Instruction::new("CMP", AddressingMode::IndirectIndexedY),
        0xD5 => Instruction::new("CMP", AddressingMode::ZeroPageX),
        0xD6 => Instruction::new("DEC", AddressingMode::ZeroPageX),
        0xD8 => Instruction::new("CLD", AddressingMode::Implied),
        0xD9 => Instruction::new("CMP", AddressingMode::AbsoluteY),
        0xDD => Instruction::new("CMP", AddressingMode::AbsoluteX),
        0xDE => Instruction::new("DEC", AddressingMode::AbsoluteX),
        0xE0 => Instruction::new("CPX", AddressingMode::Immediate),
        0xE1 => Instruction::new("SBC", AddressingMode::IndexedIndirectX),
        0xE4 => Instruction::new("CPX", AddressingMode::ZeroPage),
        0xE5 => Instruction::new("SBC", AddressingMode::ZeroPage),
        0xE6 => Instruction::new("INC", AddressingMode::ZeroPage),
        0xE8 => Instruction::new("INX", AddressingMode::Implied),
        0xE9 => Instruction::new("SBC", AddressingMode::Immediate),
        0xEA => Instruction::new("NOP", AddressingMode::Implied),
        0xEC => Instruction::new("CPX", AddressingMode::Absolute),
        0xED => Instruction::new("SBC", AddressingMode::Absolute),
        0xEE => Instruction::new("INC", AddressingMode::Absolute),
        0xF0 => Instruction::new("BEQ", AddressingMode::Relative),
        0xF1 => Instruction::new("SBC", AddressingMode::IndirectIndexedY),
        0xF5 => Instruction::new("SBC", AddressingMode::ZeroPageX),
        0xF6 => Instruction::new("INC", AddressingMode::ZeroPageX),
        0xF8 => Instruction::new("SED", AddressingMode::Implied),
        0xF9 => Instruction::new("SBC", AddressingMode::AbsoluteY),
        0xFD => Instruction::new("SBC", AddressingMode::AbsoluteX),
        0xFE => Instruction::new("INC", AddressingMode::AbsoluteX),
        _ => return None,
    };
    Some(instruction)
}

impl Instruction {
    fn new(mnemonic: &'static str, mode: AddressingMode) -> Self {
        let len = match mode {
            AddressingMode::Implied | AddressingMode::Accumulator => 1,
            AddressingMode::Immediate
            | AddressingMode::ZeroPage
            | AddressingMode::ZeroPageX
            | AddressingMode::ZeroPageY
            | AddressingMode::IndexedIndirectX
            | AddressingMode::IndirectIndexedY
            | AddressingMode::Relative => 2,
            AddressingMode::Absolute
            | AddressingMode::AbsoluteX
            | AddressingMode::AbsoluteY
            | AddressingMode::Indirect => 3,
        };
        Self {
            mnemonic,
            mode,
            len,
        }
    }
}

pub(super) fn format_instruction(instruction: Instruction, operands: &[u8], pc: u16) -> String {
    match instruction.mode {
        AddressingMode::Implied => instruction.mnemonic.to_string(),
        AddressingMode::Accumulator => format!("{} A", instruction.mnemonic),
        AddressingMode::Immediate => format!("{} #${:02X}", instruction.mnemonic, operands[0]),
        AddressingMode::ZeroPage => format!("{} ${:02X}", instruction.mnemonic, operands[0]),
        AddressingMode::ZeroPageX => format!("{} ${:02X},X", instruction.mnemonic, operands[0]),
        AddressingMode::ZeroPageY => format!("{} ${:02X},Y", instruction.mnemonic, operands[0]),
        AddressingMode::Absolute => {
            format!("{} ${:04X}", instruction.mnemonic, le_u16(operands))
        }
        AddressingMode::AbsoluteX => {
            format!("{} ${:04X},X", instruction.mnemonic, le_u16(operands))
        }
        AddressingMode::AbsoluteY => {
            format!("{} ${:04X},Y", instruction.mnemonic, le_u16(operands))
        }
        AddressingMode::Indirect => {
            format!("{} (${:04X})", instruction.mnemonic, le_u16(operands))
        }
        AddressingMode::IndexedIndirectX => {
            format!("{} (${:02X},X)", instruction.mnemonic, operands[0])
        }
        AddressingMode::IndirectIndexedY => {
            format!("{} (${:02X}),Y", instruction.mnemonic, operands[0])
        }
        AddressingMode::Relative => {
            let origin = pc.wrapping_add(2);
            let target = origin.wrapping_add_signed(i16::from(operands[0] as i8));
            format!("{} ${target:04X}", instruction.mnemonic)
        }
    }
}

pub(super) fn format_instruction_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn le_u16(bytes: &[u8]) -> u16 {
    u16::from(bytes[0]) | (u16::from(bytes[1]) << 8)
}

pub mod opcode {
    pub const ASL_A: u8 = 0x0A;
    pub const ASL_ABS: u8 = 0x0E;
    pub const PHP: u8 = 0x08;
    pub const CLC: u8 = 0x18;
    pub const ROL_A: u8 = 0x2A;
    pub const ROL_ABS: u8 = 0x2E;
    pub const PLP: u8 = 0x28;
    pub const PHA: u8 = 0x48;
    pub const PLA: u8 = 0x68;
    pub const LSR_A: u8 = 0x4A;
    pub const LSR_ABS: u8 = 0x4E;
    pub const ROR_A: u8 = 0x6A;
    pub const ROR_ABS: u8 = 0x6E;
    pub const SEC: u8 = 0x38;
    pub const TAX: u8 = 0xAA;
    pub const TAY: u8 = 0xA8;
    pub const DEY: u8 = 0x88;
    pub const INY: u8 = 0xC8;
    pub const TXA: u8 = 0x8A;
    pub const TYA: u8 = 0x98;
    pub const ADC_IMM: u8 = 0x69;
    pub const ADC_ZP: u8 = 0x65;
    pub const ADC_IZX: u8 = 0x61;
    pub const ADC_IZY: u8 = 0x71;
    pub const ADC_ABS: u8 = 0x6D;
    pub const ADC_ABS_X: u8 = 0x7D;
    pub const LDX_IMM: u8 = 0xA2;
    pub const LDY_IMM: u8 = 0xA0;
    pub const LDA_IMM: u8 = 0xA9;
    pub const LDA_ZP: u8 = 0xA5;
    pub const LDA_ZP_X: u8 = 0xB5;
    pub const LDA_IZX: u8 = 0xA1;
    pub const LDA_IZY: u8 = 0xB1;
    pub const LDA_ABS: u8 = 0xAD;
    pub const LDA_ABS_X: u8 = 0xBD;
    pub const LDA_ABS_Y: u8 = 0xB9;
    pub const LDX_ABS: u8 = 0xAE;
    pub const LDY_ABS: u8 = 0xAC;
    pub const LDX_ZP: u8 = 0xA6;
    pub const LDY_ZP: u8 = 0xA4;
    pub const AND_IMM: u8 = 0x29;
    pub const AND_ZP: u8 = 0x25;
    pub const AND_IZY: u8 = 0x31;
    pub const AND_ABS: u8 = 0x2D;
    pub const AND_ABS_X: u8 = 0x3D;
    pub const ORA_IMM: u8 = 0x09;
    pub const ORA_ZP: u8 = 0x05;
    pub const ORA_ABS: u8 = 0x0D;
    pub const ORA_ABS_X: u8 = 0x1D;
    pub const ORA_IZY: u8 = 0x11;
    pub const EOR_IMM: u8 = 0x49;
    pub const EOR_ZP: u8 = 0x45;
    pub const EOR_ABS: u8 = 0x4D;
    pub const EOR_ABS_X: u8 = 0x5D;
    pub const EOR_IZY: u8 = 0x51;
    pub const CMP_IMM: u8 = 0xC9;
    pub const CMP_ZP: u8 = 0xC5;
    pub const CMP_ABS: u8 = 0xCD;
    pub const CMP_IZY: u8 = 0xD1;
    pub const SBC_IMM: u8 = 0xE9;
    pub const SBC_ZP: u8 = 0xE5;
    pub const SBC_IZX: u8 = 0xE1;
    pub const SBC_IZY: u8 = 0xF1;
    pub const SBC_ABS: u8 = 0xED;
    pub const STA_ZP: u8 = 0x85;
    pub const STA_ZP_X: u8 = 0x95;
    pub const STA_IZX: u8 = 0x81;
    pub const STA_IZY: u8 = 0x91;
    pub const STA_ABS: u8 = 0x8D;
    pub const STA_ABS_X: u8 = 0x9D;
    pub const STA_ABS_Y: u8 = 0x99;
    pub const STX_ZP: u8 = 0x86;
    pub const STX_ABS: u8 = 0x8E;
    pub const STY_ZP: u8 = 0x84;
    pub const STY_ABS: u8 = 0x8C;
    pub const DEC_ZP: u8 = 0xC6;
    pub const DEC_ABS: u8 = 0xCE;
    pub const DEC_ABS_X: u8 = 0xDE;
    pub const INC_ZP: u8 = 0xE6;
    pub const INC_ABS: u8 = 0xEE;
    pub const INC_ABS_X: u8 = 0xFE;
    pub const JMP_ABS: u8 = 0x4C;
    pub const JMP_IND: u8 = 0x6C;
    pub const JSR_ABS: u8 = 0x20;
    pub const BCC_REL: u8 = 0x90;
    pub const BCS_REL: u8 = 0xB0;
    pub const BMI_REL: u8 = 0x30;
    pub const BPL_REL: u8 = 0x10;
    pub const BVC_REL: u8 = 0x50;
    pub const BVS_REL: u8 = 0x70;
    pub const BEQ_REL: u8 = 0xF0;
    pub const BNE_REL: u8 = 0xD0;
    pub const RTS: u8 = 0x60;
}
