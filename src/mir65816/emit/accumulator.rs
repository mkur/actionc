//! Adjacent-operation A16 forwarding. A fact certifies the complete private word
//! and its N/Z; any emitted instruction or label makes its cursor stale.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ResidentWord {
    temp: TempId,
    slot: Slot,
    delta: u32,
    cursor: (usize, usize),
}
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
        self.resident_word = None;
        if self.delta == 0
            && let Some(&Location::Stack(slot)) = self.frame.temps.get(&temp)
            && slot.width == 2
            && let Some(cursor) = self.code.word_cursor()
        {
            self.resident_word = Some(ResidentWord {
                temp,
                slot,
                delta: self.delta,
                cursor,
            });
        }
    }
    /// The caller has preflighted *all* operand extents before reaching here.
    pub(super) fn load_checked_word(&mut self, operand: WordOperand, temp: Option<TempId>) {
        if let Some(fact) = self.resident_word.take()
            && temp == Some(fact.temp)
            && self.frame.temps.get(&fact.temp) == Some(&Location::Stack(fact.slot))
            && operand == WordOperand::Stack(fact.slot.offset as u8)
            && self.delta == 0
            && fact.delta == self.delta
            && self.code.word_cursor() == Some(fact.cursor)
        {
            return;
        }
        match operand {
            WordOperand::Immediate(value) => self.code.word(0xa9, value),
            WordOperand::Stack(offset) => self.code.byte(0xa3, offset),
        }
    }
    pub(super) fn word_store(
        &mut self,
        address: &Mir65816Address,
        value: &Mir65816Value,
        bytes: u8,
        volatile: bool,
    ) -> Result<bool, String> {
        if self.delta != 0
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
        self.resident_word = None;
        Ok(true)
    }
}
