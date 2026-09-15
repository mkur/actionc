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
    pub temps: BTreeMap<TempId, Slot>,
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
                    reserve(width(ty.width.ok_or("temporary has no scalar width")?)?)?,
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
                    Some(plan.outgoing_bytes.get() + 3)
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
