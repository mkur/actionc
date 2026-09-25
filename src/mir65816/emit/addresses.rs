//! Exact-width selection for structured native addresses.
use super::*;

#[cfg(test)]
#[path = "address_tests.rs"]
mod tests;

/// Direct symbolic places already use checked relocation addends in the
/// fallback. Indirect symbolic values only admit zero here: folding their
/// modular displacement requires a separate object-extent proof.
fn direct_symbol(address: &Mir65816Address) -> Option<(Target, u32)> {
    if address.index.is_some() {
        return None;
    }
    let id = match &address.base {
        Mir65816AddressBase::Static(NirStorageId::Global(id))
        | Mir65816AddressBase::External(Mir65816ExternalAddress::Global(id)) => {
            Mir65816DataId::Global(*id)
        }
        Mir65816AddressBase::Indirect(value) if address.displacement.get() == 0 => match value {
            Mir65816Value::GlobalAddress(id, bytes) if bytes.get() == 3 => {
                Mir65816DataId::Global(*id)
            }
            Mir65816Value::StaticAddress(id, bytes) if bytes.get() == 3 => {
                Mir65816DataId::Static(*id)
            }
            _ => return None,
        },
        _ => return None,
    };
    Some((Target::Data(id), address.displacement.get()))
}

impl Builder<'_> {
    pub(super) fn symbol_address(
        &mut self,
        dest: TempId,
        address: &Mir65816Address,
    ) -> Result<bool, String> {
        let Some((target, addend)) = direct_symbol(address) else {
            return Ok(false);
        };
        let home = self.temp(dest)?;
        if home.slot().width != 3 {
            return Err("symbol address requires a complete three-byte home".into());
        }
        let Location::Stack(slot) = home else {
            return Ok(false);
        };
        // Check the entire destination before committing any instruction/fixup.
        abi::stack::access_displacement(
            ByteOffset::new(slot.offset.into()),
            ByteSize::new(3),
            ByteSize::new(self.code.delta()),
        )
        .map_err(|e| e.to_string())?;
        self.code.barrier();
        self.code.a8();
        for byte in 0..3 {
            self.code
                .reference(ReferenceOp::LdaByte, target, addend, Some(byte));
            self.store_memory(home.into(), byte.into())?;
        }
        Ok(true)
    }
}
