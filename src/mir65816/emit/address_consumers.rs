//! Pure 24-bit address producers and their complete scalar store consumers.
use super::*;

impl Builder<'_> {
    pub(super) fn address_expression(&mut self, op: &Mir65816Op) -> Result<bool, String> {
        let Some((base, offset, subtract)) = home_demand::pointer_expression(op) else { return Ok(false); };
        self.long_expression_return(3, if subtract { NirBinaryOp::Sub } else { NirBinaryOp::Add },
            base, &Mir65816Value::U24(offset))?;
        Ok(true)
    }

    pub(super) fn expression_store(&mut self, address: &Mir65816Address, bytes: u8) -> Result<(), String> {
        let indirect = matches!(address.base, Mir65816AddressBase::Indirect(_));
        // Address setup owns A but not X/Y for the admitted unindexed form.
        // Keep the low result in Y until setup finishes; X retains the bank.
        if indirect {
            self.code.a16();
            if bytes == 1 { self.code.word(WordOp::AndImm, 0xff); }
            self.code.op(Implied::Tay);
        }
        let destination = self.prepare_address(address)?;
        self.check_transfer(destination, destination, bytes)?;
        if indirect {
            self.code.a16();
            self.code.op(Implied::Tya);
        }
        let frame = if bytes == 2 { self.frame_word(address)? } else { None };
        if let Some((_, slot)) = frame { self.code.register_home(slot); }
        self.code.barrier();
        if bytes == 1 { self.code.a8(); } else { self.code.a16(); }
        self.store_memory(destination, 0)?;
        if bytes == 3 {
            self.code.op(Implied::Txa);
            self.code.a8();
            if !self.next_pointer_piece(destination, ByteOp::StaIndirectY) {
                self.store_memory(destination, 2)?;
            }
        }
        self.code.barrier();
        if let Some((object, slot)) = frame {
            self.code.remember_frame_word(object, address.displacement.get(), slot);
        }
        Ok(())
    }
}
