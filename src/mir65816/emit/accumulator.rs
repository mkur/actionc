//! Adjacent-operation A16 forwarding. A fact certifies the complete private word
//! and its N/Z; any emitted instruction or label makes its cursor stale.
use super::*;

impl Builder<'_> {
    pub(super) fn word_temp(value: &Mir65816Value) -> Option<TempId> {
        match value {
            Mir65816Value::Temp(id, size) if size.get() == 2 => Some(*id),
            _ => None,
        }
    }
    pub(super) fn direct_word_address(address: &Mir65816Address) -> bool {
        address.index.is_none()
            && matches!(
                address.base,
                Mir65816AddressBase::AutomaticFrame(_)
                    | Mir65816AddressBase::Parameter(_)
                    | Mir65816AddressBase::External(_)
                    | Mir65816AddressBase::Static(NirStorageId::Global(_))
            )
    }
    /// Only call after a checked LDA16 or binary ADC/SBC16 followed by its
    /// retained private STA. STA leaves the full-word N/Z proof intact.
    pub(super) fn remember_word(&mut self, temp: TempId) {
        self.code.barrier();
        if let Some(&Location::Stack(slot)) = self.frame.temps.get(&temp) {
            self.code.remember_word(temp, slot);
        }
    }
    /// Complete operand/home preflight remains the selector's responsibility.
    pub(super) fn load_checked_word(&mut self, operand: WordOperand, temp: Option<TempId>) {
        let slot = temp
            .and_then(|id| self.frame.temps.get(&id))
            .and_then(|home| match home {
                Location::Stack(slot) => Some(*slot),
                _ => None,
            });
        let offset = match operand {
            WordOperand::Stack(offset) => Some(offset),
            _ => None,
        };
        if self.code.consume_word(temp, slot, offset) {
            return;
        }
        match operand {
            WordOperand::Immediate(value) => self.code.word(WordOp::LdaImm, value),
            WordOperand::Stack(offset) => self.code.byte(ByteOp::LdaStack, offset),
        }
    }
    pub(super) fn word_store(
        &mut self,
        address: &Mir65816Address,
        value: &Mir65816Value,
        bytes: u8,
        volatile: bool,
    ) -> Result<bool, String> {
        if self.code.delta() != 0
            || bytes != 2
            || volatile
            || !Self::direct_word_address(address)
            || Self::word_temp(value).is_none()
        {
            return Ok(false);
        }
        let Some(source @ WordOperand::Stack(offset)) = self.word_operand(value)? else {
            return Ok(false);
        };
        // Direct address resolution emits no instructions. All extent checks
        // still run even if a resident source makes its LDA unnecessary.
        let destination = self.prepare_address(address)?;
        self.check_transfer(Memory::Stack(u32::from(offset)), destination, 2)?;
        if let Memory::Stack(offset) = destination {
            self.word_displacement(offset)?;
        }
        self.code.a16();
        self.load_checked_word(source, Self::word_temp(value));
        self.store_memory(destination, 0)?;
        self.code.barrier();
        Ok(true)
    }
}
