//! Adjacent-operation A16 forwarding. A fact certifies the complete private word
//! and its N/Z; any emitted instruction or label makes its cursor stale.
use super::*;

impl Builder<'_> {
    /// This slice excludes incoming parameters, address-taken objects and all
    /// indirect/indexed/external accesses. Both bytes belong to this object.
    fn frame_word(
        &self,
        address: &Mir65816Address,
    ) -> Result<Option<(Mir65816FrameObjectId, Slot)>, String> {
        let Mir65816AddressBase::AutomaticFrame(id) = address.base else {
            return Ok(None);
        };
        if address.index.is_some() || self.code.delta() != 0 {
            return Ok(None);
        }
        let object = self
            .routine
            .frame
            .objects
            .iter()
            .find(|o| o.id == id)
            .ok_or("unknown frame object")?;
        if object.addressable {
            return Ok(None);
        }
        let byte = address.displacement.get();
        if byte
            .checked_add(2)
            .is_none_or(|end| end > object.size.get())
        {
            return Err("frame word exceeds object extent".into());
        }
        let offset = self
            .object(id)?
            .checked_add(byte)
            .ok_or("frame word offset overflow")?;
        let offset = self.word_displacement(offset)?;
        Ok(Some((
            id,
            Slot {
                offset: u16::from(offset),
                width: 2,
            },
        )))
    }
    pub(super) fn frame_word_load(
        &mut self,
        dest: TempId,
        bytes: u8,
        address: &Mir65816Address,
        volatile: bool,
    ) -> Result<bool, String> {
        if volatile || bytes != 2 {
            return Ok(false);
        }
        let Some((object, source)) = self.frame_word(address)? else {
            return Ok(false);
        };
        let destination = self.temp(dest)?;
        if destination.slot().width != 2 {
            return Err("load temporary width mismatch".into());
        }
        // Preflight the retained destination store even when the load vanishes.
        let offset = word_home(destination, self.code.delta())?;
        if !self.code.consume_frame_word(
            object,
            address.displacement.get(),
            source,
            source.offset as u8,
        ) {
            return Ok(false);
        }
        self.code.store_word(offset);
        self.remember_word(dest);
        Ok(true)
    }
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
        if let Some(&home) = self.frame.temps.get(&temp) {
            self.code.remember_word(temp, home);
        }
    }
    /// Complete operand/home preflight remains the selector's responsibility.
    pub(super) fn load_checked_word(&mut self, operand: WordOperand, temp: Option<TempId>) {
        let slot = temp.and_then(|id| self.frame.temps.get(&id).copied());
        let offset = operand.home();
        if self.code.consume_word(temp, slot, offset) || self.code.load_x_word(temp, slot) {
            return;
        }
        match operand {
            WordOperand::Immediate(value) => self.code.word(WordOp::LdaImm, value),
            WordOperand::Stack(offset) => self.code.byte(ByteOp::LdaStack, offset),
            WordOperand::DirectPage(offset) => self.code.byte(ByteOp::LdaDp, offset),
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
        let Some(source) = self.word_operand(value)? else {
            return Ok(false);
        };
        // Direct address resolution emits no instructions. All extent checks
        // still run even if a resident source makes its LDA unnecessary.
        let destination = self.prepare_address(address)?;
        let Some(home) = source.home() else {
            return Ok(false);
        };
        self.check_transfer(Location::from(home).into(), destination, 2)?;
        if let Memory::Stack(offset) = destination {
            self.word_displacement(offset)?;
        }
        let frame = self.frame_word(address)?;
        if let Some((_, slot)) = frame {
            self.code.register_home(slot);
        }
        self.code.a16();
        if let Some((object, slot)) = frame {
            let temp = Self::word_temp(value).unwrap();
            if self
                .code
                .store_incoming_capture(temp, self.temp(temp)?, slot)
            {
                self.code
                    .remember_frame_word(object, address.displacement.get(), slot);
                return Ok(true);
            }
        }
        self.load_checked_word(source, Self::word_temp(value));
        self.store_memory(destination, 0)?;
        self.code.barrier();
        if let Some((object, slot)) = frame {
            self.code
                .remember_frame_word(object, address.displacement.get(), slot);
        }
        Ok(true)
    }
}
