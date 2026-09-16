use super::super::*;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub offset: u16,
    pub width: u8,
}

#[derive(Debug, Clone)]
pub struct AllocatedFrame {
    pub extent: u16,
    pub spill_bytes: u16,
    /// Exact local reservation/transfer peak, excluding callees' reservations
    /// and the platform's separately reserved interrupt headroom.
    pub peak_below_entry: u16,
    pub temps: BTreeMap<TempId, Location>,
    pub edge_copies: Vec<Slot>,
}

pub(super) fn width(width: ByteSize) -> Result<u8, String> {
    match width.get() {
        1..=4 => Ok(width.get() as u8),
        _ => Err(format!("unsupported native scalar width {}", width.get())),
    }
}

impl AllocatedFrame {
    pub(super) fn new(routine: &Mir65816Routine) -> Result<Self, String> {
        if let Some(frame) = Self::pointer_leaf(routine)? {
            return Ok(frame);
        }
        let mut cursor = routine.frame.extent.get() + 1;
        let mut reserve = |width: u8| -> Result<Slot, String> {
            if width > 1 {
                cursor = (cursor + 1) & !1;
            }
            let offset = cursor;
            cursor += u32::from(width);
            abi::stack::access_displacement(
                ByteOffset::new(offset),
                ByteSize::new(width.into()),
                ByteSize::ZERO,
            )
            .map_err(|e| e.to_string())?;
            Ok(Slot {
                offset: offset as u16,
                width,
            })
        };
        let mut temps = BTreeMap::new();
        for (id, ty) in &routine.temps {
            if temps
                .insert(
                    *id,
                    Location::Stack(reserve(width(
                        ty.width.ok_or("temporary has no scalar width")?,
                    )?)?),
                )
                .is_some()
            {
                return Err("duplicate temporary identity".into());
            }
        }
        let count = routine
            .blocks
            .iter()
            .map(|b| b.params.len())
            .max()
            .unwrap_or(0);
        let edge_copies = (0..count)
            .map(|_| reserve(4))
            .collect::<Result<Vec<_>, _>>()?;
        let extent = abi::stack::fixed_extent(ByteSize::new(cursor - 1))
            .map_err(|e| e.to_string())?
            .get() as u16;
        for parameter in &routine.frame.parameters {
            let Mir65816AbiHome::StackArgument { offset, size, .. } = parameter.incoming else {
                return Err("parameter has no stack home".into());
            };
            abi::stack::incoming_displacement(ByteSize::new(extent.into()), offset, size)
                .map_err(|e| e.to_string())?;
        }
        let call_peak = routine
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .filter_map(|op| {
                if let Mir65816Op::Call { plan, .. } = op {
                    Some(
                        plan.outgoing_bytes.get()
                            + plan
                                .native
                                .expect("verified native call")
                                .transfer
                                .peak_bytes()
                                .get(),
                    )
                } else {
                    None
                }
            })
            .max()
            .unwrap_or(0);
        Ok(Self {
            extent,
            spill_bytes: extent - routine.frame.extent.get() as u16,
            peak_below_entry: extent + call_peak as u16,
            temps,
            edge_copies,
        })
    }
}

/// A physical home for a typed MIR value. Offsets are relative to S or D,
/// respectively; they must never be interpreted interchangeably.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    Stack(Slot),
    DirectPage(Slot),
}

impl Location {
    pub fn slot(self) -> Slot {
        match self {
            Self::Stack(slot) | Self::DirectPage(slot) => slot,
        }
    }
    pub fn stack(self) -> Result<Slot, String> {
        match self {
            Self::Stack(slot) => Ok(slot),
            Self::DirectPage(_) => {
                Err("direct-page location requires resident pointer selection".into())
            }
        }
    }
}

const POINTER_SLOTS: [u16; 3] = [
    abi::generated::DP_POINTER0_OFFSET as u16,
    abi::generated::DP_POINTER1_OFFSET as u16,
    abi::generated::DP_POINTER2_OFFSET as u16,
];

/// Closed operation intervals: the destination and all inputs coexist for the
/// entire operation, including the bank-byte access after the low-word access.
#[derive(Clone, Copy)]
struct Interval {
    first: usize,
    last: usize,
}

/// The whitelist is also the scratch contract: these operations use A/Y/flags,
/// their declared homes and the declared memory address only. No PTR/RESULT/
/// arithmetic scratch is available while these residents exist. Prologue
/// checks read the separate ABI stack-bound slots; there are no call boundaries.
fn leaf_intervals(routine: &Mir65816Routine) -> Option<BTreeMap<TempId, Interval>> {
    let [block] = routine.blocks.as_slice() else {
        return None;
    };
    if !block.params.is_empty()
        || block.ops.len() > 64
        || routine.temps.is_empty()
        || !matches!(
            block.terminator,
            Mir65816Terminator::Return { value: None, .. }
        )
        || routine.temps.iter().any(|(_, ty)| {
            ty.width != Some(ByteSize::new(3))
                || !matches!(ty.kind, crate::nir::NirTypeKind::Pointer { .. })
        })
    {
        return None;
    }
    let mut intervals = BTreeMap::<TempId, Interval>::new();
    fn use_value(
        value: &Mir65816Value,
        at: usize,
        intervals: &mut BTreeMap<TempId, Interval>,
    ) -> Option<()> {
        let Mir65816Value::Temp(id, width) = value else {
            return None;
        };
        if width.get() != 3 {
            return None;
        }
        intervals.get_mut(id)?.last = at;
        Some(())
    }
    for (at, op) in block.ops.iter().enumerate() {
        let (address, dest) = match op {
            Mir65816Op::Load {
                dest,
                width,
                address,
                volatile: false,
            } if width.get() == 3 => (address, Some(*dest)),
            Mir65816Op::Store {
                address,
                value,
                width,
                volatile: false,
            } if width.get() == 3 => {
                use_value(value, at, &mut intervals)?;
                (address, None)
            }
            _ => return None,
        };
        if address.index.is_some() || address.displacement.get() > u32::from(u16::MAX) - 3 {
            return None;
        }
        match &address.base {
            Mir65816AddressBase::Indirect(value) => use_value(value, at, &mut intervals)?,
            Mir65816AddressBase::AutomaticFrame(_)
            | Mir65816AddressBase::Parameter(_)
            | Mir65816AddressBase::External(_)
            | Mir65816AddressBase::Static(NirStorageId::Global(_)) => {}
            _ => return None,
        }
        if let Some(id) = dest {
            if intervals
                .insert(
                    id,
                    Interval {
                        first: at,
                        last: at,
                    },
                )
                .is_some()
            {
                return None;
            }
        }
    }
    if intervals.len() != routine.temps.len()
        || routine
            .temps
            .iter()
            .any(|(id, _)| !intervals.contains_key(id))
    {
        return None;
    }
    Some(intervals)
}

impl AllocatedFrame {
    /// Plan a bounded pointer leaf from verified native MIR. None selects the
    /// complete stack strategy, including when pressure exceeds three slots.
    pub fn pointer_leaf(routine: &Mir65816Routine) -> Result<Option<Self>, String> {
        let Some(intervals) = leaf_intervals(routine) else {
            return Ok(None);
        };
        let mut ordered = intervals.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|(id, range)| (range.first, **id));
        let mut occupied = [None; 3];
        let mut temps = BTreeMap::new();
        for (&id, range) in ordered {
            let Some(index) = occupied
                .iter()
                .position(|last| last.is_none_or(|last| last < range.first))
            else {
                return Ok(None);
            };
            occupied[index] = Some(range.last);
            temps.insert(
                id,
                Location::DirectPage(Slot {
                    offset: POINTER_SLOTS[index],
                    width: 3,
                }),
            );
        }
        let extent = abi::stack::fixed_extent(routine.frame.extent)
            .map_err(|e| e.to_string())?
            .get() as u16;
        let frame = Self {
            extent,
            spill_bytes: 0,
            peak_below_entry: extent,
            temps,
            edge_copies: vec![],
        };
        frame.verify_pointer_leaf(routine)?;
        Ok(Some(frame))
    }

    /// Recheck all identities, types, scratch ownership and closed live ranges
    /// before selection. A final image map cannot establish these MIR proofs.
    pub fn verify_pointer_leaf(&self, routine: &Mir65816Routine) -> Result<(), String> {
        let ranges =
            leaf_intervals(routine).ok_or("unsupported operation in direct-page allocation")?;
        if self.temps.len() != ranges.len()
            || !self.edge_copies.is_empty()
            || u32::from(self.extent) != routine.frame.extent.get()
            || self.spill_bytes != 0
            || self.peak_below_entry != self.extent
        {
            return Err("invalid direct-page frame accounting".into());
        }
        for (&id, range) in &ranges {
            let Some(Location::DirectPage(slot)) = self.temps.get(&id) else {
                return Err("missing direct-page temporary location".into());
            };
            if slot.width != 3 || !POINTER_SLOTS.contains(&slot.offset) {
                return Err("direct-page temporary exceeds owned pointer scratch".into());
            }
            for (&other, other_range) in ranges.range(..id) {
                if self.temps[&other] == self.temps[&id]
                    && range.first <= other_range.last
                    && other_range.first <= range.last
                {
                    return Err("overlapping direct-page temporary lifetimes".into());
                }
            }
        }
        for parameter in &routine.frame.parameters {
            let Mir65816AbiHome::StackArgument { offset, size, .. } = parameter.incoming else {
                return Err("parameter has no stack home".into());
            };
            abi::stack::incoming_displacement(ByteSize::new(self.extent.into()), offset, size)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
