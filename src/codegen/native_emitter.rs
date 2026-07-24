use crate::diagnostic::Diagnostic;
use crate::source::Span;

use super::emitter::Emitter;
use super::native_state::NativeProcessorState;
use super::*;

pub(crate) struct NativeTrackedEmitter {
    emitter: Emitter,
    state: NativeProcessorState,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub(super) struct NativeOptimizationGuard<'a> {
    name: &'a str,
    state: &'a NativeProcessorState,
}

#[cfg(test)]
impl<'a> NativeOptimizationGuard<'a> {
    pub(super) fn name(self) -> &'a str {
        self.name
    }

    pub(super) fn state(self) -> &'a NativeProcessorState {
        self.state
    }

    pub(super) fn assert_state_available(self) {
        let _ = self.state;
    }
}

impl NativeTrackedEmitter {
    pub(crate) fn with_origin(origin: u16) -> Self {
        Self {
            emitter: Emitter::with_origin(origin),
            state: NativeProcessorState::default(),
        }
    }

    #[cfg(test)]
    pub(super) fn state(&self) -> &NativeProcessorState {
        &self.state
    }

    #[cfg(test)]
    pub(super) fn optimization_guard(&self, name: &'static str) -> NativeOptimizationGuard<'_> {
        NativeOptimizationGuard {
            name,
            state: &self.state,
        }
    }

    #[cfg(test)]
    pub(super) fn state_snapshot(&self) -> super::native_state::NativeProcessorSnapshot {
        self.state.snapshot()
    }

    pub(crate) fn position(&self) -> usize {
        self.emitter.position()
    }

    pub(crate) fn label_position(&self, label: &str) -> Option<usize> {
        self.emitter.label_position(label)
    }

    pub(super) fn patch_absolute_bytes(&mut self, address: u16, value: u16, width: u16) -> bool {
        if address < self.emitter.origin {
            return false;
        }
        let offset = usize::from(address.wrapping_sub(self.emitter.origin));
        let width = usize::from(width.min(2));
        if offset
            .checked_add(width)
            .is_none_or(|end| end > self.emitter.bytes.len())
        {
            return false;
        }
        self.emitter.bytes[offset] = (value & 0x00FF) as u8;
        if width > 1 {
            self.emitter.bytes[offset + 1] = (value >> 8) as u8;
        }
        true
    }

    pub(crate) fn finish(self) -> Result<Vec<u8>, Vec<Diagnostic>> {
        self.emitter.finish()
    }

    pub(super) fn bind_label_at_position(
        &mut self,
        label: impl Into<String>,
        position: usize,
        span: Span,
    ) -> Result<(), Diagnostic> {
        self.state.bind_label();
        self.emitter.bind_label_at_position(label, position, span)
    }

    pub(crate) fn bind_label(
        &mut self,
        label: impl Into<String>,
        span: Span,
    ) -> Result<(), Diagnostic> {
        self.state.bind_label();
        self.emitter.bind_label(label, span)
    }

    pub(crate) fn emit_u8(&mut self, value: u8) {
        self.emitter.emit_u8(value);
        self.state.call_unknown();
    }

    pub(crate) fn emit_u16_le(&mut self, value: u16) {
        self.emitter.emit_u16_le(value);
        self.state.call_unknown();
    }

    pub(crate) fn emit_u16_label(&mut self, label: impl Into<String>, span: Span) {
        self.emitter.emit_u16_label(label, span);
        self.state.call_unknown();
    }

    pub(crate) fn emit_u16_label_offset(
        &mut self,
        label: impl Into<String>,
        addend: i32,
        span: Span,
    ) {
        self.emitter.emit_u16_label_offset(label, addend, span);
        self.state.call_unknown();
    }

    pub(crate) fn emit_u8_label_low(&mut self, label: impl Into<String>, span: Span) {
        self.emitter.emit_u8_label_low(label, span);
        self.state.call_unknown();
    }

    pub(crate) fn emit_u8_label_low_offset(
        &mut self,
        label: impl Into<String>,
        addend: i32,
        span: Span,
    ) {
        self.emitter.emit_u8_label_low_offset(label, addend, span);
        self.state.call_unknown();
    }

    pub(crate) fn emit_u8_label_high(&mut self, label: impl Into<String>, span: Span) {
        self.emitter.emit_u8_label_high(label, span);
        self.state.call_unknown();
    }

    pub(crate) fn emit_u8_label_high_offset(
        &mut self,
        label: impl Into<String>,
        addend: i32,
        span: Span,
    ) {
        self.emitter.emit_u8_label_high_offset(label, addend, span);
        self.state.call_unknown();
    }

    pub(crate) fn emit_zeroes(&mut self, count: u16) {
        self.emitter.emit_zeroes(count);
        self.state.call_unknown();
    }

    pub(crate) fn emit_rts(&mut self) {
        self.emitter.emit_rts();
        self.state.call_unknown();
    }

    pub(crate) fn emit_jmp_abs(&mut self, address: u16) {
        self.emitter.emit_jmp_abs(address);
        self.state.call_unknown();
    }

    pub(crate) fn emit_jmp_label(&mut self, label: impl Into<String>, span: Span) {
        self.emitter.emit_jmp_label(label, span);
        self.state.call_unknown();
    }

    pub(crate) fn emit_jmp_indirect(&mut self, address: u16) {
        self.emitter.emit_jmp_indirect(address);
        self.state.call_unknown();
    }

    pub(crate) fn emit_jsr_abs(&mut self, address: u16) {
        self.emitter.emit_jsr_abs(address);
        self.state.call_unknown();
    }

    pub(crate) fn emit_jsr_label(&mut self, label: impl Into<String>, span: Span) {
        self.emitter.emit_jsr_label(label, span);
        self.state.call_unknown();
    }

    pub(crate) fn emit_branch_label(&mut self, opcode: u8, label: impl Into<String>, span: Span) {
        self.emitter.emit_branch_label(opcode, label, span);
    }

    pub(crate) fn emit_lda_imm(&mut self, value: u8) {
        self.emitter.emit_lda_imm(value);
        self.state.load_a_immediate(value);
    }

    pub(crate) fn emit_lda_abs(&mut self, address: u16) {
        if self.state.can_skip_load_a_memory(address) && self.state.flags_match_a_value() {
            return;
        }
        if let Some(zero_page) = direct_zero_page(address) {
            self.emit_lda_zero_page(zero_page);
            return;
        }
        self.emitter.emit_lda_abs(address);
        self.state.load_a_memory(address);
    }

    pub(crate) fn emit_lda_abs_x(&mut self, address: u16) {
        self.emitter.emit_lda_abs_x(address);
        self.state.load_a_unknown();
    }

    pub(crate) fn emit_lda_abs_y(&mut self, address: u16) {
        self.emitter.emit_lda_abs_y(address);
        self.state.load_a_unknown();
    }

    pub(crate) fn emit_lda_zero_page(&mut self, zero_page: ZeroPage) {
        self.emitter.emit_lda_zero_page(zero_page);
        self.state.load_a_memory(u16::from(zero_page.address()));
    }

    pub(crate) fn emit_lda_zero_page_x(&mut self, zero_page_x: ZeroPageX) {
        self.emitter.emit_lda_zero_page_x(zero_page_x);
        self.state.load_a_unknown();
    }

    pub(crate) fn emit_lda_indirect_indexed_y(&mut self, indexed: IndirectIndexedY) {
        self.emitter.emit_lda_indirect_indexed_y(indexed);
        self.state.load_a_unknown();
    }

    pub(crate) fn emit_ldx_imm(&mut self, value: u8) {
        self.emitter.emit_ldx_imm(value);
        self.state.load_x_immediate(value);
    }

    pub(crate) fn emit_ldx_abs(&mut self, address: u16) {
        if let Some(zero_page) = direct_zero_page(address) {
            self.emit_ldx_zero_page(zero_page);
            return;
        }
        self.emitter.emit_ldx_abs(address);
        self.state.load_x_memory(address);
    }

    pub(crate) fn emit_ldx_zero_page(&mut self, zero_page: ZeroPage) {
        self.emitter.emit_ldx_zero_page(zero_page);
        self.state.load_x_memory(u16::from(zero_page.address()));
    }

    pub(crate) fn emit_ldy_imm(&mut self, value: u8) {
        if self.state.can_skip_load_y_immediate(value) {
            return;
        }
        if let Some(current) = self.state.y_immediate() {
            if current.wrapping_add(1) == value {
                self.emit_iny();
                return;
            }
            if current.wrapping_sub(1) == value {
                self.emit_dey();
                return;
            }
        }
        self.emitter.emit_ldy_imm(value);
        self.state.load_y_immediate(value);
    }

    pub(crate) fn emit_ldy_abs(&mut self, address: u16) {
        if let Some(zero_page) = direct_zero_page(address) {
            self.emit_ldy_zero_page(zero_page);
            return;
        }
        self.emitter.emit_ldy_abs(address);
        self.state.load_y_memory(address);
    }

    pub(crate) fn emit_ldy_zero_page(&mut self, zero_page: ZeroPage) {
        self.emitter.emit_ldy_zero_page(zero_page);
        self.state.load_y_memory(u16::from(zero_page.address()));
    }

    pub(crate) fn emit_tax(&mut self) {
        self.emitter.emit_tax();
        self.state.transfer_a_to_x();
    }

    pub(crate) fn emit_tay(&mut self) {
        self.emitter.emit_tay();
        self.state.transfer_a_to_y();
    }

    pub(crate) fn emit_txa(&mut self) {
        self.emitter.emit_txa();
        self.state.load_a_unknown();
    }

    pub(crate) fn emit_tya(&mut self) {
        self.emitter.emit_tya();
        self.state.load_a_unknown();
    }

    pub(crate) fn emit_sta_zero_page(&mut self, zero_page: ZeroPage) {
        self.emitter.emit_sta_zero_page(zero_page);
        self.state.store_a_memory(u16::from(zero_page.address()));
    }

    pub(crate) fn emit_sta_absolute(&mut self, absolute: Absolute) {
        if let Some(zero_page) = direct_zero_page(absolute.address()) {
            self.emit_sta_zero_page(zero_page);
            return;
        }
        self.emitter.emit_sta_absolute(absolute);
        self.state.store_a_memory(absolute.address());
    }

    pub(crate) fn emit_sta_abs_x(&mut self, address: u16) {
        self.emitter.emit_sta_abs_x(address);
        self.state.mutate_unknown_memory();
    }

    pub(crate) fn emit_sta_abs_y(&mut self, address: u16) {
        self.emitter.emit_sta_abs_y(address);
        self.state.mutate_unknown_memory();
    }

    pub(crate) fn emit_sta_zero_page_x(&mut self, zero_page_x: ZeroPageX) {
        self.emitter.emit_sta_zero_page_x(zero_page_x);
        self.state.mutate_unknown_memory();
    }

    pub(crate) fn emit_sta_indirect_indexed_y(&mut self, indexed: IndirectIndexedY) {
        self.emitter.emit_sta_indirect_indexed_y(indexed);
        self.state.mutate_unknown_memory();
    }

    pub(crate) fn emit_stx_absolute(&mut self, absolute: Absolute) {
        if let Some(zero_page) = direct_zero_page(absolute.address()) {
            self.emit_stx_zero_page(zero_page);
            return;
        }
        self.emitter.emit_stx_absolute(absolute);
        self.state.store_x_memory(absolute.address());
    }

    pub(crate) fn emit_stx_zero_page(&mut self, zero_page: ZeroPage) {
        self.emitter.emit_stx_zero_page(zero_page);
        self.state.store_x_memory(u16::from(zero_page.address()));
    }

    pub(crate) fn emit_sty_absolute(&mut self, absolute: Absolute) {
        if let Some(zero_page) = direct_zero_page(absolute.address()) {
            self.emit_sty_zero_page(zero_page);
            return;
        }
        self.emitter.emit_sty_absolute(absolute);
        self.state.store_y_memory(absolute.address());
    }

    pub(crate) fn emit_sty_zero_page(&mut self, zero_page: ZeroPage) {
        self.emitter.emit_sty_zero_page(zero_page);
        self.state.store_y_memory(u16::from(zero_page.address()));
    }

    pub(crate) fn emit_clc(&mut self) {
        self.emitter.emit_clc();
        self.state.clc();
    }

    pub(crate) fn emit_sec(&mut self) {
        self.emitter.emit_sec();
        self.state.sec();
    }

    pub(crate) fn emit_adc_imm(&mut self, value: u8) {
        self.emitter.emit_adc_imm(value);
        self.state.arithmetic_a();
    }

    pub(crate) fn emit_adc_abs(&mut self, address: u16) {
        if let Some(zero_page) = direct_zero_page(address) {
            self.emit_adc_zero_page(zero_page);
            return;
        }
        self.emitter.emit_adc_abs(address);
        self.state.arithmetic_a();
    }

    pub(crate) fn emit_adc_zero_page(&mut self, zero_page: ZeroPage) {
        self.emitter.emit_adc_zero_page(zero_page);
        self.state.arithmetic_a();
    }

    pub(crate) fn emit_adc_indirect_indexed_y(&mut self, indexed: IndirectIndexedY) {
        self.emitter.emit_adc_indirect_indexed_y(indexed);
        self.state.arithmetic_a();
    }

    pub(crate) fn emit_sbc_imm(&mut self, value: u8) {
        self.emitter.emit_sbc_imm(value);
        self.state.arithmetic_a();
    }

    pub(crate) fn emit_sbc_abs(&mut self, address: u16) {
        if let Some(zero_page) = direct_zero_page(address) {
            self.emit_sbc_zero_page(zero_page);
            return;
        }
        self.emitter.emit_sbc_abs(address);
        self.state.arithmetic_a();
    }

    pub(crate) fn emit_sbc_zero_page(&mut self, zero_page: ZeroPage) {
        self.emitter.emit_sbc_zero_page(zero_page);
        self.state.arithmetic_a();
    }

    pub(crate) fn emit_sbc_indirect_indexed_y(&mut self, indexed: IndirectIndexedY) {
        self.emitter.emit_sbc_indirect_indexed_y(indexed);
        self.state.arithmetic_a();
    }

    pub(super) fn emit_lsr_absolute(&mut self, absolute: Absolute) {
        self.emitter.emit_lsr_absolute(absolute);
        self.state.call_unknown();
    }

    pub(super) fn emit_ror_absolute(&mut self, absolute: Absolute) {
        self.emitter.emit_ror_absolute(absolute);
        self.state.call_unknown();
    }

    pub(crate) fn emit_dec_absolute(&mut self, absolute: Absolute) {
        if let Some(zero_page) = direct_zero_page(absolute.address()) {
            self.emit_dec_zero_page(zero_page);
            return;
        }
        self.emitter.emit_dec_absolute(absolute);
        self.state.call_unknown();
    }

    pub(crate) fn emit_dec_absolute_x(&mut self, absolute_x: AbsoluteX) {
        self.emitter.emit_dec_absolute_x(absolute_x);
        self.state.call_unknown();
    }

    pub(crate) fn emit_dec_zero_page(&mut self, zero_page: ZeroPage) {
        self.emitter.emit_dec_zero_page(zero_page);
        self.state.call_unknown();
    }

    pub(crate) fn emit_and_imm(&mut self, value: u8) {
        self.emitter.emit_and_imm(value);
        self.state.load_a_unknown();
    }

    pub(crate) fn emit_and_abs(&mut self, address: u16) {
        if let Some(zero_page) = direct_zero_page(address) {
            self.emit_and_zero_page(zero_page);
            return;
        }
        self.emitter.emit_and_abs(address);
        self.state.load_a_unknown();
    }

    pub(crate) fn emit_and_zero_page(&mut self, zero_page: ZeroPage) {
        self.emitter.emit_and_zero_page(zero_page);
        self.state.load_a_unknown();
    }

    pub(crate) fn emit_ora_imm(&mut self, value: u8) {
        self.emitter.emit_ora_imm(value);
        self.state.load_a_unknown();
    }

    pub(crate) fn emit_ora_abs(&mut self, address: u16) {
        if let Some(zero_page) = direct_zero_page(address) {
            self.emit_ora_zero_page(zero_page);
            return;
        }
        self.emitter.emit_ora_abs(address);
        self.state.load_a_unknown();
    }

    pub(crate) fn emit_ora_zero_page(&mut self, zero_page: ZeroPage) {
        self.emitter.emit_ora_zero_page(zero_page);
        self.state.load_a_unknown();
    }

    pub(crate) fn emit_eor_imm(&mut self, value: u8) {
        self.emitter.emit_eor_imm(value);
        self.state.load_a_unknown();
    }

    pub(crate) fn emit_eor_abs(&mut self, address: u16) {
        if let Some(zero_page) = direct_zero_page(address) {
            self.emit_eor_zero_page(zero_page);
            return;
        }
        self.emitter.emit_eor_abs(address);
        self.state.load_a_unknown();
    }

    pub(crate) fn emit_eor_zero_page(&mut self, zero_page: ZeroPage) {
        self.emitter.emit_eor_zero_page(zero_page);
        self.state.load_a_unknown();
    }

    pub(crate) fn emit_cmp_imm(&mut self, value: u8) {
        self.emitter.emit_cmp_imm(value);
        self.state.invalidate_flags();
    }

    pub(crate) fn emit_cmp_imm_for_z_branch(&mut self, value: u8) {
        if value == 0 && self.state.flags_match_a_value() {
            return;
        }
        self.emit_cmp_imm(value);
    }

    pub(crate) fn emit_cmp_abs(&mut self, address: u16) {
        if let Some(zero_page) = direct_zero_page(address) {
            self.emit_cmp_zero_page(zero_page);
            return;
        }
        self.emitter.emit_cmp_abs(address);
        self.state.invalidate_flags();
    }

    pub(crate) fn emit_cmp_zero_page(&mut self, zero_page: ZeroPage) {
        self.emitter.emit_cmp_zero_page(zero_page);
        self.state.invalidate_flags();
    }

    pub(crate) fn emit_cmp_indirect_indexed_y(&mut self, indexed: IndirectIndexedY) {
        self.emitter.emit_cmp_indirect_indexed_y(indexed);
        self.state.invalidate_flags();
    }

    pub(crate) fn emit_asl_a(&mut self) {
        self.emitter.emit_asl_a();
        self.state.arithmetic_a();
    }

    pub(crate) fn emit_lsr_a(&mut self) {
        self.emitter.emit_lsr_a();
        self.state.arithmetic_a();
    }

    pub(crate) fn emit_rol_a(&mut self) {
        self.emitter.emit_rol_a();
        self.state.arithmetic_a();
    }

    pub(crate) fn emit_iny(&mut self) {
        self.emitter.emit_iny();
        self.state.increment_y();
    }

    pub(super) fn emit_dey(&mut self) {
        self.emitter.emit_dey();
        self.state.decrement_y();
    }

    pub(crate) fn emit_inc_zero_page(&mut self, zero_page: ZeroPage) {
        self.emitter.emit_inc_zero_page(zero_page);
        self.state.mutate_memory(u16::from(zero_page.address()));
    }

    pub(crate) fn emit_inc_absolute(&mut self, absolute: Absolute) {
        if let Some(zero_page) = direct_zero_page(absolute.address()) {
            self.emit_inc_zero_page(zero_page);
            return;
        }
        self.emitter.emit_inc_absolute(absolute);
        self.state.mutate_memory(absolute.address());
    }

    pub(crate) fn emit_inc_absolute_x(&mut self, absolute_x: AbsoluteX) {
        self.emitter.emit_inc_absolute_x(absolute_x);
        self.state.call_unknown();
    }

    pub(crate) fn emit_pha(&mut self) {
        self.emitter.emit_pha();
    }

    pub(crate) fn emit_php(&mut self) {
        self.emitter.emit_php();
    }

    pub(crate) fn emit_pla(&mut self) {
        self.emitter.emit_pla();
        self.state.load_a_unknown();
    }

    pub(crate) fn emit_plp(&mut self) {
        self.emitter.emit_plp();
        self.state.invalidate_flags();
    }
}

fn direct_zero_page(address: u16) -> Option<ZeroPage> {
    (address <= 0x00FF).then(|| ZeroPage::new(address as u8))
}

#[cfg(test)]
mod tests {
    use super::super::emitter::opcode;
    use super::super::native_state::{NativeMemoryFact, NativeValue};
    use super::*;

    #[test]
    fn native_tracked_emitter_tracks_load_store_aliases() {
        let mut emitter = NativeTrackedEmitter::with_origin(0x3000);

        emitter.emit_lda_imm(0x44);
        emitter.emit_sta_absolute(Absolute::new(0x3100));
        emitter.emit_lda_abs(0x3100);

        assert_eq!(emitter.state().a(), NativeValue::Immediate(0x44));
        assert_eq!(
            emitter.state().memory_value(0x3100),
            Some(NativeValue::Immediate(0x44))
        );
        assert_eq!(
            emitter.finish().unwrap(),
            [opcode::LDA_IMM, 0x44, opcode::STA_ABS, 0x00, 0x31]
        );
    }

    #[test]
    fn native_tracked_emitter_selects_zero_page_direct_opcodes() {
        let mut emitter = NativeTrackedEmitter::with_origin(0x3000);

        emitter.emit_lda_abs(0x00E0);
        emitter.emit_sta_absolute(Absolute::new(0x00E1));
        emitter.emit_adc_abs(0x00E2);
        emitter.emit_cmp_abs(0x00E3);
        emitter.emit_inc_absolute(Absolute::new(0x00E4));
        emitter.emit_dec_absolute(Absolute::new(0x00E5));

        assert_eq!(
            emitter.finish().unwrap(),
            [
                opcode::LDA_ZP,
                0xE0,
                opcode::STA_ZP,
                0xE1,
                opcode::ADC_ZP,
                0xE2,
                opcode::CMP_ZP,
                0xE3,
                opcode::INC_ZP,
                0xE4,
                opcode::DEC_ZP,
                0xE5,
            ]
        );
    }

    #[test]
    fn native_tracked_emitter_removes_adjacent_reload_after_store() {
        let mut emitter = NativeTrackedEmitter::with_origin(0x3000);

        emitter.emit_lda_abs(0x3100);
        emitter.emit_sta_absolute(Absolute::new(0x3101));
        emitter.emit_lda_abs(0x3101);

        assert_eq!(
            emitter.finish().unwrap(),
            [opcode::LDA_ABS, 0x00, 0x31, opcode::STA_ABS, 0x01, 0x31,]
        );
    }

    #[test]
    fn native_tracked_emitter_suppresses_redundant_ldy_immediate() {
        let mut emitter = NativeTrackedEmitter::with_origin(0x3000);

        emitter.emit_ldy_imm(1);
        emitter.emit_ldy_imm(1);

        assert_eq!(emitter.finish().unwrap(), [opcode::LDY_IMM, 1]);
    }

    #[test]
    fn native_tracked_emitter_keeps_ldy_immediate_when_flags_differ() {
        let mut emitter = NativeTrackedEmitter::with_origin(0x3000);

        emitter.emit_ldy_imm(1);
        emitter.emit_lda_imm(0);
        emitter.emit_ldy_imm(1);

        assert_eq!(
            emitter.finish().unwrap(),
            [opcode::LDY_IMM, 1, opcode::LDA_IMM, 0, opcode::LDY_IMM, 1]
        );
    }

    #[test]
    fn native_tracked_emitter_steps_adjacent_ldy_immediates() {
        let mut emitter = NativeTrackedEmitter::with_origin(0x3000);

        emitter.emit_ldy_imm(0);
        emitter.emit_ldy_imm(1);
        emitter.emit_ldy_imm(0);

        assert_eq!(
            emitter.finish().unwrap(),
            [opcode::LDY_IMM, 0, opcode::INY, opcode::DEY]
        );
    }

    #[test]
    fn native_tracked_emitter_step_ldy_immediate_sets_matching_flags() {
        let mut emitter = NativeTrackedEmitter::with_origin(0x3000);

        emitter.emit_ldy_imm(0);
        emitter.emit_lda_imm(0x80);
        emitter.emit_ldy_imm(1);

        let snapshot = emitter.state_snapshot();
        assert_eq!(snapshot.y, NativeValue::Immediate(1));
        assert_eq!(snapshot.flags.zero, NativeValue::Immediate(1));
        assert_eq!(snapshot.flags.negative, NativeValue::Immediate(1));
        assert_eq!(
            emitter.finish().unwrap(),
            [opcode::LDY_IMM, 0, opcode::LDA_IMM, 0x80, opcode::INY]
        );
    }

    #[test]
    fn native_tracked_emitter_keeps_reload_across_barriers() {
        let mut call_barrier = NativeTrackedEmitter::with_origin(0x3000);
        call_barrier.emit_lda_imm(0x44);
        call_barrier.emit_sta_absolute(Absolute::new(0x3100));
        call_barrier.emit_jsr_abs(0x4000);
        call_barrier.emit_lda_abs(0x3100);
        assert_eq!(
            call_barrier.finish().unwrap(),
            [
                opcode::LDA_IMM,
                0x44,
                opcode::STA_ABS,
                0x00,
                0x31,
                opcode::JSR_ABS,
                0x00,
                0x40,
                opcode::LDA_ABS,
                0x00,
                0x31,
            ]
        );

        let mut label_barrier = NativeTrackedEmitter::with_origin(0x3000);
        label_barrier.emit_lda_imm(0x44);
        label_barrier.emit_sta_absolute(Absolute::new(0x3100));
        label_barrier
            .bind_label("join", Span::new(0, 0))
            .expect("bind label");
        label_barrier.emit_lda_abs(0x3100);
        assert_eq!(
            label_barrier.finish().unwrap(),
            [
                opcode::LDA_IMM,
                0x44,
                opcode::STA_ABS,
                0x00,
                0x31,
                opcode::LDA_ABS,
                0x00,
                0x31,
            ]
        );

        let mut raw_barrier = NativeTrackedEmitter::with_origin(0x3000);
        raw_barrier.emit_lda_imm(0x44);
        raw_barrier.emit_sta_absolute(Absolute::new(0x3100));
        raw_barrier.emit_u8(0xEA);
        raw_barrier.emit_lda_abs(0x3100);
        assert_eq!(
            raw_barrier.finish().unwrap(),
            [
                opcode::LDA_IMM,
                0x44,
                opcode::STA_ABS,
                0x00,
                0x31,
                0xEA,
                opcode::LDA_ABS,
                0x00,
                0x31,
            ]
        );
    }

    #[test]
    fn native_tracked_emitter_clears_state_across_calls_and_labels() {
        let mut emitter = NativeTrackedEmitter::with_origin(0x3000);

        emitter.emit_lda_imm(1);
        emitter.emit_jsr_abs(0x4000);
        assert_eq!(emitter.state().a(), NativeValue::Unknown);

        emitter.emit_lda_imm(2);
        emitter.bind_label("join", Span::new(0, 0)).unwrap();
        assert_eq!(emitter.state().a(), NativeValue::Unknown);
    }

    #[test]
    fn native_tracked_emitter_retests_unknown_call_result_for_zero_branch() {
        let mut emitter = NativeTrackedEmitter::with_origin(0x3000);

        emitter.emit_jsr_abs(0x4000);
        emitter.emit_cmp_imm_for_z_branch(0);

        assert_eq!(
            emitter.finish().unwrap(),
            [opcode::JSR_ABS, 0x00, 0x40, opcode::CMP_IMM, 0x00]
        );
    }

    #[test]
    fn native_tracked_emitter_clears_state_on_raw_data_barriers() {
        let mut emitter = NativeTrackedEmitter::with_origin(0x3000);

        emitter.emit_lda_imm(1);
        emitter.emit_u8(0xEA);
        assert_eq!(emitter.state().a(), NativeValue::Unknown);

        emitter.emit_lda_imm(2);
        emitter.emit_u16_le(0x1234);
        assert_eq!(emitter.state().a(), NativeValue::Unknown);

        emitter.emit_lda_imm(3);
        emitter.emit_u16_label("target", Span::new(0, 0));
        assert_eq!(emitter.state().a(), NativeValue::Unknown);

        emitter.emit_lda_imm(4);
        emitter.emit_zeroes(2);
        assert_eq!(emitter.state().a(), NativeValue::Unknown);
    }

    #[test]
    fn native_tracked_emitter_exposes_state_snapshots_for_tests() {
        let mut emitter = NativeTrackedEmitter::with_origin(0x3000);

        emitter.emit_lda_imm(0x7F);
        emitter.emit_sta_absolute(Absolute::new(0x3200));
        let snapshot = emitter.state_snapshot();

        assert_eq!(snapshot.a, NativeValue::Immediate(0x7F));
        assert_eq!(snapshot.x, NativeValue::Unknown);
        assert_eq!(snapshot.y, NativeValue::Unknown);
        assert_eq!(
            snapshot.memory,
            vec![NativeMemoryFact {
                address: 0x3200,
                value: NativeValue::Immediate(0x7F),
            }]
        );
        assert_eq!(snapshot.flags.zero, NativeValue::Immediate(0x7F));
        assert_eq!(snapshot.flags.negative, NativeValue::Immediate(0x7F));
    }

    #[test]
    fn native_tracked_emitter_requires_explicit_optimization_guard() {
        let mut emitter = NativeTrackedEmitter::with_origin(0x3000);

        emitter.emit_lda_imm(0);
        let guard = emitter.optimization_guard("redundant LDA #imm");

        assert_eq!(guard.name(), "redundant LDA #imm");
        assert_eq!(guard.state().a(), NativeValue::Immediate(0));
        guard.assert_state_available();
    }
}
