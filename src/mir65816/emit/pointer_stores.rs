//! A bounded ordinary store may preserve its compiler-owned address base.
use super::*;

/// Sealed input derived from verified MIR and final homes, not a saved success
/// bit. Physical memory effects remain conservative for every other analysis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mir65816::emit) struct Contract {
    site: selected::Source,
    origin: PointerOrigin,
    source: Slot,
    offset: u16,
    bytes: u8,
}

impl Contract {
    fn new(
        b: &Builder<'_>,
        site: selected::Source,
        op: &Mir65816Op,
    ) -> Result<Option<Self>, String> {
        let Mir65816Op::Store {
            address,
            width,
            volatile: false,
            ..
        } = op
        else {
            return Ok(None);
        };
        let Mir65816AddressBase::Indirect(value) = &address.base else {
            return Ok(None);
        };
        if address.index.is_some()
            || !(1..=4).contains(&width.get())
            || address.displacement.get() > u32::from(u16::MAX) - 3
        {
            return Ok(None);
        }
        let Some((origin, source)) = b.stack_pointer_source(value)? else {
            return Ok(None);
        };
        // Immutable incoming arguments cannot escape. Mutable/address-taken
        // parameters have distinct frame objects; only unexposed homes qualify.
        let private = match origin {
            PointerOrigin::Temporary(id) => {
                b.frame.temps.get(&id) == Some(&Location::Stack(source))
            }
            PointerOrigin::Parameter(id) => b
                .routine
                .frame
                .parameters
                .iter()
                .any(|p| p.param == id && p.frame_object.is_none()),
            PointerOrigin::Frame(id) => b.routine.frame.objects.iter().any(|o| {
                o.id == id
                    && !o.addressable
                    && o.size.get() == 3
                    && o.stack_offset.get() == u32::from(source.offset)
            }),
        };
        if !private {
            return Ok(None);
        }
        Ok(Some(Self {
            site,
            origin,
            source,
            offset: address.displacement.get() as u16,
            bytes: width.get() as u8,
        }))
    }

    pub(in crate::mir65816::emit) fn site(self) -> selected::Source {
        self.site
    }

    pub(in crate::mir65816::emit) fn covers(
        self,
        origin: PointerOrigin,
        source: Slot,
        offset: u16,
        bytes: u8,
    ) -> bool {
        self.origin == origin
            && self.source == source
            && bytes != 0
            && offset >= self.offset
            && u32::from(offset) + u32::from(bytes)
                <= u32::from(self.offset) + u32::from(self.bytes)
    }
}

impl Builder<'_> {
    pub(super) fn enter_pointer_operation(
        &mut self,
        pointers: &pointer_forwarding::Plan,
        block: BlockId,
        index: usize,
        op: &Mir65816Op,
    ) -> Result<bool, String> {
        let omitted = pointers.enter(self, block, index);
        if let Some(contract) = Contract::new(self, selected::Source { block, index }, op)? {
            self.code.allow_pointer_store(contract);
        }
        Ok(omitted)
    }

    pub(super) fn stack_pointer_source(
        &self,
        value: &Mir65816Value,
    ) -> Result<Option<(PointerOrigin, Slot)>, String> {
        if self.code.delta() != 0 || self.value_width(value)? != 3 {
            return Ok(None);
        }
        let Some(Memory::Stack(offset)) = self.value_memory(value)? else {
            return Ok(None);
        };
        let origin = match value {
            Mir65816Value::Temp(id, _) => self
                .borrowed
                .get(id)
                .map_or(PointerOrigin::Temporary(*id), |s| s.origin()),
            Mir65816Value::Param(id) => PointerOrigin::Parameter(*id),
            _ => return Ok(None),
        };
        self.displacement(offset, 2)?;
        Ok(Some((
            origin,
            Slot {
                offset: offset as u16,
                width: 3,
            },
        )))
    }
}
