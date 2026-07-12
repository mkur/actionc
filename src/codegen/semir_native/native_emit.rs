use super::*;

impl<'a, 'm> SemIrNativeEmitter<'a, 'm> {
    pub(super) fn ensure_y_zero(&mut self) {
        if !self.y_known_zero {
            self.emit_ldy_imm(0);
        }
    }

    pub(super) fn emit_ldy_imm(&mut self, value: u8) {
        if value == 0 && self.y_known_zero {
            return;
        }
        self.emitter.emit_ldy_imm(value);
        self.y_known_zero = value == 0;
    }

    pub(super) fn emit_y_one(&mut self) {
        if self.y_known_zero {
            self.emitter.emit_iny();
        } else {
            self.emitter.emit_ldy_imm(1);
        }
        self.y_known_zero = false;
    }

    pub(super) fn emit_lda_imm(&mut self, value: u8) {
        self.emitter.emit_lda_imm(value);
    }

    pub(super) fn emit_ldx_imm(&mut self, value: u8) {
        self.emitter.emit_ldx_imm(value);
    }

    pub(super) fn emit_tax(&mut self) {
        self.emitter.emit_tax();
    }

    pub(super) fn emit_tay(&mut self) {
        self.emitter.emit_tay();
        self.y_known_zero = false;
    }

    pub(super) fn emit_asl_a(&mut self) {
        self.emitter.emit_asl_a();
    }

    pub(super) fn emit_lsr_a(&mut self) {
        self.emitter.emit_lsr_a();
    }

    pub(super) fn emit_rol_a(&mut self) {
        self.emitter.emit_rol_a();
    }

    pub(super) fn emit_php(&mut self) {
        self.emitter.emit_php();
    }

    pub(super) fn emit_plp(&mut self) {
        self.emitter.emit_plp();
    }

    pub(super) fn emit_pha(&mut self) {
        self.emitter.emit_pha();
    }

    pub(super) fn emit_pla(&mut self) {
        self.emitter.emit_pla();
    }

    pub(super) fn emit_dey(&mut self) {
        self.emitter.emit_dey();
        self.y_known_zero = true;
    }

    pub(super) fn emit_jmp_label(&mut self, label: impl Into<String>, span: Span) {
        self.emitter.emit_jmp_label(label, span);
    }

    pub(super) fn emit_jmp_addr(&mut self, address: u16) {
        self.emitter.emit_jmp_abs(address);
    }

    pub(super) fn emit_jsr_addr(&mut self, address: u16) {
        self.emitter.emit_jsr_abs(address);
        self.y_known_zero = false;
    }

    pub(super) fn emit_jsr_label(&mut self, label: impl Into<String>, span: Span) {
        self.emitter.emit_jsr_label(label, span);
        self.y_known_zero = false;
    }

    pub(super) fn emit_jsr_runtime_helper(&mut self, helper: RuntimeHelperTarget, span: Span) {
        match helper {
            RuntimeHelperTarget::Absolute(address) => self.emit_jsr_addr(address.address()),
            RuntimeHelperTarget::Label(label) => self.emit_jsr_label(label, span),
        }
    }

    pub(super) fn emit_rts(&mut self) {
        self.emitter.emit_rts();
    }

    pub(super) fn emit_clc(&mut self) {
        self.emitter.emit_clc();
    }

    pub(super) fn emit_sec(&mut self) {
        self.emitter.emit_sec();
    }

    pub(super) fn emit_adc_imm(&mut self, value: u8) {
        self.emitter.emit_adc_imm(value);
    }

    pub(super) fn emit_sbc_imm(&mut self, value: u8) {
        self.emitter.emit_sbc_imm(value);
    }

    pub(super) fn emit_cmp_imm(&mut self, value: u8) {
        self.emitter.emit_cmp_imm(value);
    }

    pub(super) fn emit_eor_imm(&mut self, value: u8) {
        self.emitter.emit_eor_imm(value);
    }

    pub(super) fn emit_and_imm(&mut self, value: u8) {
        self.emitter.emit_and_imm(value);
    }

    pub(super) fn emit_ora_imm(&mut self, value: u8) {
        self.emitter.emit_ora_imm(value);
    }

    pub(super) fn emit_raw_u8(&mut self, value: u8) {
        self.emitter.emit_u8(value);
    }

    pub(super) fn emit_raw_u16_le(&mut self, value: u16) {
        self.emitter.emit_u16_le(value);
    }

    pub(super) fn emit_raw_u16_label(&mut self, label: impl Into<String>, span: Span) {
        self.emitter.emit_u16_label(label, span);
    }

    pub(super) fn emit_raw_u8_label_low(&mut self, label: impl Into<String>, span: Span) {
        self.emitter.emit_u8_label_low(label, span);
    }

    pub(super) fn emit_raw_u8_label_high(&mut self, label: impl Into<String>, span: Span) {
        self.emitter.emit_u8_label_high(label, span);
    }

    pub(super) fn emit_raw_zeroes(&mut self, count: u16) {
        self.emitter.emit_zeroes(count);
    }

    pub(super) fn emit_raw_bytes(&mut self, bytes: impl IntoIterator<Item = u8>) {
        for byte in bytes {
            self.emit_raw_u8(byte);
        }
    }

    pub(super) fn emit_beq_label(&mut self, label: impl Into<String>, span: Span) {
        self.emit_conditional_branch_label(opcode::BEQ_REL, label, span);
    }

    pub(super) fn emit_bne_label(&mut self, label: impl Into<String>, span: Span) {
        self.emit_conditional_branch_label(opcode::BNE_REL, label, span);
    }

    pub(super) fn emit_bcc_label(&mut self, label: impl Into<String>, span: Span) {
        self.emit_conditional_branch_label(opcode::BCC_REL, label, span);
    }

    pub(super) fn emit_bcs_label(&mut self, label: impl Into<String>, span: Span) {
        self.emit_conditional_branch_label(opcode::BCS_REL, label, span);
    }

    fn emit_conditional_branch_label(&mut self, opcode: u8, label: impl Into<String>, span: Span) {
        let label = label.into();
        if self.branch_target_is_bound_and_in_range(&label) {
            self.emitter.emit_branch_label(opcode, label, span);
            return;
        }

        let take_label = self.next_label("long_branch_take");
        let after_label = self.next_label("long_branch_after");
        self.emitter
            .emit_branch_label(opcode, take_label.clone(), span);
        self.emit_jmp_label(after_label.clone(), span);
        self.bind_label(&take_label, span)
            .expect("generated SemIR native long-branch label should be unique");
        self.emit_jmp_label(label, span);
        self.bind_label(&after_label, span)
            .expect("generated SemIR native long-branch label should be unique");
    }

    fn branch_target_is_bound_and_in_range(&self, label: &str) -> bool {
        let Some(target) = self.emitter.label_position(label) else {
            return false;
        };
        let origin = self.emitter.position() + 2;
        let delta = target as isize - origin as isize;
        (-128..=127).contains(&delta)
    }

    pub(super) fn emit_lda_args(&mut self, byte_offset: u8) {
        self.emitter
            .emit_lda_zero_page(runtime_zp::ARGS.offset(byte_offset));
    }

    pub(super) fn emit_ldx_args(&mut self, byte_offset: u8) {
        self.emitter
            .emit_ldx_zero_page(runtime_zp::ARGS.offset(byte_offset));
    }

    pub(super) fn emit_sta_args(&mut self, byte_offset: u8) {
        self.emitter
            .emit_sta_zero_page(runtime_zp::ARGS.offset(byte_offset));
    }

    pub(super) fn emit_lda_array_addr(&mut self, byte_offset: u8) {
        self.emitter
            .emit_lda_zero_page(runtime_zp::ARRAY_ADDR.offset(byte_offset));
    }

    pub(super) fn emit_sta_array_addr(&mut self, byte_offset: u8) {
        self.emitter
            .emit_sta_zero_page(runtime_zp::ARRAY_ADDR.offset(byte_offset));
    }

    pub(super) fn emit_lda_array_addr_indirect_y(&mut self) {
        self.emitter
            .emit_lda_indirect_indexed_y(IndirectIndexedY::new(runtime_zp::ARRAY_ADDR));
    }

    pub(super) fn emit_sta_array_addr_indirect_y(&mut self) {
        self.emitter
            .emit_sta_indirect_indexed_y(IndirectIndexedY::new(runtime_zp::ARRAY_ADDR));
    }

    pub(super) fn emit_cmp_array_addr_indirect_y(&mut self) {
        self.emitter
            .emit_cmp_indirect_indexed_y(IndirectIndexedY::new(runtime_zp::ARRAY_ADDR));
    }

    pub(super) fn emit_lda_element_addr(&mut self) {
        self.emitter.emit_lda_zero_page(runtime_zp::ELEMENT_ADDR);
    }

    pub(super) fn emit_sta_element_addr(&mut self) {
        self.emitter.emit_sta_zero_page(runtime_zp::ELEMENT_ADDR);
    }

    pub(super) fn emit_adc_element_addr(&mut self) {
        self.emitter.emit_adc_zero_page(runtime_zp::ELEMENT_ADDR);
    }

    pub(super) fn emit_adc_afcur(&mut self) {
        self.emitter.emit_adc_zero_page(runtime_zp::AFCUR);
    }

    pub(super) fn emit_sbc_element_addr(&mut self) {
        self.emitter.emit_sbc_zero_page(runtime_zp::ELEMENT_ADDR);
    }

    pub(super) fn emit_sbc_afcur(&mut self) {
        self.emitter.emit_sbc_zero_page(runtime_zp::AFCUR);
    }

    pub(super) fn emit_eor_element_addr(&mut self) {
        self.emitter.emit_eor_zero_page(runtime_zp::ELEMENT_ADDR);
    }

    pub(super) fn emit_sta_afcur(&mut self) {
        self.emitter.emit_sta_zero_page(runtime_zp::AFCUR);
    }

    pub(super) fn emit_sta_afcur_high(&mut self) {
        self.emitter.emit_sta_zero_page(runtime_zp::AFCUR.offset(1));
    }

    pub(super) fn emit_lda_addr(&mut self, address: u16) {
        if let Some(zero_page) = native_zero_page(address) {
            self.emitter.emit_lda_zero_page(zero_page);
        } else {
            self.emitter.emit_lda_abs(address);
        }
    }

    pub(super) fn emit_sta_addr(&mut self, address: u16) {
        if let Some(zero_page) = native_zero_page(address) {
            self.emitter.emit_sta_zero_page(zero_page);
        } else {
            self.emitter.emit_sta_absolute(Absolute::new(address));
        }
    }

    pub(super) fn emit_lda_addr_x(&mut self, address: u16) {
        self.emitter.emit_lda_abs_x(address);
    }

    pub(super) fn emit_sta_addr_x(&mut self, address: u16) {
        self.emitter.emit_sta_abs_x(address);
    }

    pub(super) fn emit_sty_addr(&mut self, address: u16) {
        if let Some(zero_page) = native_zero_page(address) {
            self.emitter.emit_sty_zero_page(zero_page);
        } else {
            self.emitter.emit_sty_absolute(Absolute::new(address));
        }
    }

    pub(super) fn emit_ldy_addr(&mut self, address: u16) {
        if let Some(zero_page) = native_zero_page(address) {
            self.emitter.emit_ldy_zero_page(zero_page);
        } else {
            self.emitter.emit_ldy_abs(address);
        }
        self.y_known_zero = false;
    }

    pub(super) fn emit_ldx_addr(&mut self, address: u16) {
        if let Some(zero_page) = native_zero_page(address) {
            self.emitter.emit_ldx_zero_page(zero_page);
        } else {
            self.emitter.emit_ldx_abs(address);
        }
    }

    pub(super) fn emit_stx_addr(&mut self, address: u16) {
        if let Some(zero_page) = native_zero_page(address) {
            self.emitter.emit_stx_zero_page(zero_page);
        } else {
            self.emitter.emit_stx_absolute(Absolute::new(address));
        }
    }

    pub(super) fn emit_adc_addr(&mut self, address: u16) {
        if let Some(zero_page) = native_zero_page(address) {
            self.emitter.emit_adc_zero_page(zero_page);
        } else {
            self.emitter.emit_adc_abs(address);
        }
    }

    pub(super) fn emit_cmp_addr(&mut self, address: u16) {
        if let Some(zero_page) = native_zero_page(address) {
            self.emitter.emit_cmp_zero_page(zero_page);
        } else {
            self.emitter.emit_cmp_abs(address);
        }
    }

    pub(super) fn emit_cmp_afcur(&mut self) {
        self.emitter.emit_cmp_zero_page(runtime_zp::AFCUR);
    }

    pub(super) fn emit_eor_addr(&mut self, address: u16) {
        if let Some(zero_page) = native_zero_page(address) {
            self.emitter.emit_eor_zero_page(zero_page);
        } else {
            self.emitter.emit_eor_abs(address);
        }
    }

    pub(super) fn emit_and_addr(&mut self, address: u16) {
        if let Some(zero_page) = native_zero_page(address) {
            self.emitter.emit_and_zero_page(zero_page);
        } else {
            self.emitter.emit_and_abs(address);
        }
    }

    pub(super) fn emit_ora_addr(&mut self, address: u16) {
        if let Some(zero_page) = native_zero_page(address) {
            self.emitter.emit_ora_zero_page(zero_page);
        } else {
            self.emitter.emit_ora_abs(address);
        }
    }

    pub(super) fn emit_inc_addr(&mut self, address: u16) {
        if let Some(zero_page) = native_zero_page(address) {
            self.emitter.emit_inc_zero_page(zero_page);
        } else {
            self.emitter.emit_inc_absolute(Absolute::new(address));
        }
    }

    pub(super) fn emit_dec_addr(&mut self, address: u16) {
        if let Some(zero_page) = native_zero_page(address) {
            self.emitter.emit_dec_zero_page(zero_page);
        } else {
            self.emitter.emit_dec_absolute(Absolute::new(address));
        }
    }

    pub(super) fn emit_sbc_addr(&mut self, address: u16) {
        if let Some(zero_page) = native_zero_page(address) {
            self.emitter.emit_sbc_zero_page(zero_page);
        } else {
            self.emitter.emit_sbc_abs(address);
        }
    }

    pub(super) fn emit_lsr_addr(&mut self, address: u16) {
        self.emitter.emit_lsr_absolute(Absolute::new(address));
    }

    pub(super) fn emit_ror_addr(&mut self, address: u16) {
        self.emitter.emit_ror_absolute(Absolute::new(address));
    }
}
