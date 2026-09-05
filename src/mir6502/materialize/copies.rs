use super::{DEFAULT_POINTER_PAIR, FreshTemps, pointer_value_from_mem};
use crate::mir6502::ir::{
    MirAddr, MirDef, MirEffects, MirMemoryEffect, MirOp, MirRoutine, MirTemp, MirValue, MirWidth,
};
use crate::mir6502::materialize::values::offset_mem;

#[derive(Debug, Default)]
pub(super) struct AggregateCopySelectionStats {
    pub retained: usize,
    pub selected: usize,
    pub prepared_address: usize,
    pub scalar_fallback: usize,
    pub blocked_address_form: usize,
    pub blocked_offset_range: usize,
}

#[derive(Debug)]
enum EndpointPlan {
    Direct(MirAddr),
    Prepared {
        setup: Vec<MirOp>,
        first_offset: u16,
    },
}

#[derive(Debug)]
enum SelectionBlock {
    AddressForm,
    OffsetRange,
}

pub(super) fn select_aggregate_copies(routine: &mut MirRoutine) -> AggregateCopySelectionStats {
    let mut fresh = FreshTemps::new(&routine.temps);
    let (temps, blocks) = (&mut routine.temps, &mut routine.blocks);
    let mut stats = AggregateCopySelectionStats::default();

    for block in blocks {
        let mut out = Vec::with_capacity(block.ops.len());
        for op in std::mem::take(&mut block.ops) {
            let MirOp::CopyBytes {
                destination,
                source,
                size,
                destination_volatile,
                source_volatile,
            } = op
            else {
                out.push(op);
                continue;
            };

            stats.retained += 1;
            match select_copy(
                &destination,
                &source,
                size,
                destination_volatile,
                source_volatile,
                &mut fresh,
                temps,
            ) {
                Ok((replacement, prepared_address)) => {
                    out.extend(replacement);
                    stats.selected += 1;
                    stats.prepared_address += usize::from(prepared_address);
                }
                Err(reason) => {
                    out.extend(expand_scalar_fallback(
                        &destination,
                        &source,
                        size,
                        destination_volatile,
                        source_volatile,
                        &mut fresh,
                        temps,
                    ));
                    stats.scalar_fallback += 1;
                    match reason {
                        SelectionBlock::AddressForm => stats.blocked_address_form += 1,
                        SelectionBlock::OffsetRange => stats.blocked_offset_range += 1,
                    }
                }
            }
        }
        block.ops = out;
    }

    stats
}

fn select_copy(
    destination: &MirAddr,
    source: &MirAddr,
    size: u16,
    destination_volatile: bool,
    source_volatile: bool,
    fresh: &mut FreshTemps,
    temps: &mut Vec<MirTemp>,
) -> Result<(Vec<MirOp>, bool), SelectionBlock> {
    let source = plan_endpoint(source, size)?;
    let destination = plan_endpoint(destination, size)?;
    let prepared_address = matches!(source, EndpointPlan::Prepared { .. })
        || matches!(destination, EndpointPlan::Prepared { .. });
    let mut out = Vec::new();

    if source_volatile {
        out.push(volatile_barrier());
    }
    let mut staged = Vec::with_capacity(usize::from(size));
    match source {
        EndpointPlan::Direct(addr) => {
            for offset in 0..size {
                let byte = fresh.fresh(temps);
                out.push(MirOp::Load {
                    dst: MirDef::VTemp(byte),
                    src: offset_aggregate_addr(&addr, offset),
                    width: MirWidth::Byte,
                });
                staged.push(byte);
            }
        }
        EndpointPlan::Prepared {
            setup,
            first_offset,
        } => {
            out.extend(setup);
            for offset in 0..size {
                let byte = fresh.fresh(temps);
                out.push(MirOp::LoadIndirect {
                    consumer: DEFAULT_POINTER_PAIR,
                    dst: MirDef::VTemp(byte),
                    offset: first_offset + offset,
                });
                staged.push(byte);
            }
        }
    }
    if source_volatile {
        out.push(volatile_barrier());
    }

    if destination_volatile {
        out.push(volatile_barrier());
    }
    match destination {
        EndpointPlan::Direct(addr) => {
            for (offset, byte) in staged.into_iter().enumerate() {
                out.push(MirOp::Store {
                    dst: offset_aggregate_addr(&addr, offset as u16),
                    src: MirValue::Def(MirDef::VTemp(byte)),
                    width: MirWidth::Byte,
                });
            }
        }
        EndpointPlan::Prepared {
            setup,
            first_offset,
        } => {
            out.extend(setup);
            for (offset, byte) in staged.into_iter().enumerate() {
                out.push(MirOp::StoreIndirect {
                    consumer: DEFAULT_POINTER_PAIR,
                    src: MirValue::Def(MirDef::VTemp(byte)),
                    offset: first_offset + offset as u16,
                });
            }
        }
    }
    if destination_volatile {
        out.push(volatile_barrier());
    }

    Ok((out, prepared_address))
}

fn plan_endpoint(addr: &MirAddr, size: u16) -> Result<EndpointPlan, SelectionBlock> {
    let plan = match addr {
        MirAddr::Direct(_) => Ok(EndpointPlan::Direct(addr.clone())),
        MirAddr::ComputedIndex {
            base,
            index,
            elem_size,
            offset,
        } => {
            let scale = u8::try_from(*elem_size).map_err(|_| SelectionBlock::AddressForm)?;
            if scale == 0 {
                return Err(SelectionBlock::AddressForm);
            }
            Ok(EndpointPlan::Prepared {
                setup: vec![
                    MirOp::MaterializeAddress {
                        consumer: DEFAULT_POINTER_PAIR,
                        value: base.clone(),
                    },
                    MirOp::AdvanceAddress {
                        consumer: DEFAULT_POINTER_PAIR,
                        index: index.clone(),
                        scale,
                    },
                ],
                first_offset: *offset,
            })
        }
        MirAddr::PointerIndex {
            ptr,
            index,
            elem_size,
            offset,
        } => {
            let scale = u8::try_from(*elem_size).map_err(|_| SelectionBlock::AddressForm)?;
            if scale == 0 {
                return Err(SelectionBlock::AddressForm);
            }
            Ok(EndpointPlan::Prepared {
                setup: vec![
                    MirOp::MaterializeAddress {
                        consumer: DEFAULT_POINTER_PAIR,
                        value: pointer_value_from_mem(ptr),
                    },
                    MirOp::AdvanceAddress {
                        consumer: DEFAULT_POINTER_PAIR,
                        index: index.clone(),
                        scale,
                    },
                ],
                first_offset: *offset,
            })
        }
        MirAddr::PointerCell { ptr, offset } => {
            Ok(EndpointPlan::Prepared {
                setup: vec![MirOp::MaterializeAddress {
                    consumer: DEFAULT_POINTER_PAIR,
                    value: pointer_value_from_mem(ptr),
                }],
                first_offset: *offset,
            })
        }
        MirAddr::Deref { ptr, offset } => {
            Ok(EndpointPlan::Prepared {
                setup: vec![MirOp::MaterializeAddress {
                    consumer: DEFAULT_POINTER_PAIR,
                    value: ptr.clone(),
                }],
                first_offset: *offset,
            })
        }
        MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::AbsoluteIndexedX { .. }
        | MirAddr::AbsoluteIndexedY { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => Err(SelectionBlock::AddressForm),
    }?;
    if let EndpointPlan::Prepared { mut setup, first_offset } = plan {
        if checked_indirect_extent(first_offset, size).is_ok() {
            return Ok(EndpointPlan::Prepared { setup, first_offset });
        }
        // A small copy can still straddle or lie beyond Y's byte range.
        // Fold the field offset into the pointer, retaining bounded Y lanes.
        checked_indirect_extent(0, size)?;
        setup.push(MirOp::AdvanceAddress {
            consumer: DEFAULT_POINTER_PAIR,
            index: MirValue::ConstU16(first_offset),
            scale: 1,
        });
        Ok(EndpointPlan::Prepared { setup, first_offset: 0 })
    } else {
        Ok(plan)
    }
}

fn checked_indirect_extent(first_offset: u16, size: u16) -> Result<(), SelectionBlock> {
    let last_offset = first_offset
        .checked_add(size.saturating_sub(1))
        .ok_or(SelectionBlock::OffsetRange)?;
    if last_offset > u16::from(u8::MAX) {
        return Err(SelectionBlock::OffsetRange);
    }
    Ok(())
}

fn expand_scalar_fallback(
    destination: &MirAddr,
    source: &MirAddr,
    size: u16,
    destination_volatile: bool,
    source_volatile: bool,
    fresh: &mut FreshTemps,
    temps: &mut Vec<MirTemp>,
) -> Vec<MirOp> {
    let mut out = Vec::new();
    if source_volatile {
        out.push(volatile_barrier());
    }
    let mut staged = Vec::with_capacity(usize::from(size));
    for offset in 0..size {
        let byte = fresh.fresh(temps);
        out.push(MirOp::Load {
            dst: MirDef::VTemp(byte),
            src: offset_aggregate_addr(source, offset),
            width: MirWidth::Byte,
        });
        staged.push(byte);
    }
    if source_volatile {
        out.push(volatile_barrier());
    }
    if destination_volatile {
        out.push(volatile_barrier());
    }
    for (offset, byte) in staged.into_iter().enumerate() {
        out.push(MirOp::Store {
            dst: offset_aggregate_addr(destination, offset as u16),
            src: MirValue::Def(MirDef::VTemp(byte)),
            width: MirWidth::Byte,
        });
    }
    if destination_volatile {
        out.push(volatile_barrier());
    }
    out
}

pub(super) fn volatile_barrier() -> MirOp {
    MirOp::Barrier {
        effects: MirEffects {
            memory_reads: MirMemoryEffect::All,
            memory_writes: MirMemoryEffect::All,
            opaque: true,
            ..MirEffects::default()
        },
    }
}

fn offset_aggregate_addr(addr: &MirAddr, offset: u16) -> MirAddr {
    match addr {
        MirAddr::Direct(mem) => MirAddr::Direct(offset_mem(mem, offset)),
        MirAddr::AbsoluteIndexedX { base } => MirAddr::AbsoluteIndexedX {
            base: offset_mem(base, offset),
        },
        MirAddr::AbsoluteIndexedY { base } => MirAddr::AbsoluteIndexedY {
            base: offset_mem(base, offset),
        },
        MirAddr::ComputedIndex {
            base,
            index,
            elem_size,
            offset: base_offset,
        } => MirAddr::ComputedIndex {
            base: base.clone(),
            index: index.clone(),
            elem_size: *elem_size,
            offset: base_offset.saturating_add(offset),
        },
        MirAddr::PointerCell {
            ptr,
            offset: base_offset,
        } => MirAddr::PointerCell {
            ptr: ptr.clone(),
            offset: base_offset.saturating_add(offset),
        },
        MirAddr::PointerIndex {
            ptr,
            index,
            elem_size,
            offset: base_offset,
        } => MirAddr::PointerIndex {
            ptr: ptr.clone(),
            index: index.clone(),
            elem_size: *elem_size,
            offset: base_offset.saturating_add(offset),
        },
        MirAddr::Deref {
            ptr,
            offset: base_offset,
        } => MirAddr::Deref {
            ptr: ptr.clone(),
            offset: base_offset.saturating_add(offset),
        },
        MirAddr::Label(_)
        | MirAddr::ZeroPageIndexedX { .. }
        | MirAddr::IndirectIndexedY { .. }
        | MirAddr::FixedIndirectIndexedY { .. } => {
            unreachable!("aggregate copy fallback requires an offsettable address")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn computed(scale: u16, offset: u16) -> MirAddr {
        MirAddr::ComputedIndex {
            base: MirValue::ConstU16(0x8000),
            index: MirValue::ConstU8(7),
            elem_size: scale,
            offset,
        }
    }

    #[test]
    fn computed_copy_endpoints_prepare_one_scaled_address() {
        for scale in [1, 2, 3, 12, 255] {
            let EndpointPlan::Prepared {
                setup,
                first_offset,
            } = plan_endpoint(&computed(scale, 5), 12).expect("prepared endpoint")
            else {
                panic!("computed endpoint was not prepared");
            };
            assert_eq!(first_offset, 5);
            assert_eq!(setup.len(), 2);
            assert!(matches!(
                &setup[1],
                MirOp::AdvanceAddress {
                    scale: actual,
                    ..
                } if u16::from(*actual) == scale
            ));
        }
    }

    #[test]
    fn prepared_copy_stages_all_reads_before_any_write() {
        let mut temps = Vec::new();
        let mut fresh = FreshTemps::new(&temps);
        let (ops, prepared) = select_copy(
            &computed(3, 0),
            &computed(12, 0),
            12,
            true,
            true,
            &mut fresh,
            &mut temps,
        )
        .expect("selected copy");

        assert!(prepared);
        assert_eq!(
            ops.iter()
                .filter(|op| matches!(op, MirOp::LoadIndirect { .. }))
                .count(),
            12
        );
        assert_eq!(
            ops.iter()
                .filter(|op| matches!(op, MirOp::StoreIndirect { .. }))
                .count(),
            12
        );
        let last_read = ops
            .iter()
            .rposition(|op| matches!(op, MirOp::LoadIndirect { .. }))
            .expect("source read");
        let first_write = ops
            .iter()
            .position(|op| matches!(op, MirOp::StoreIndirect { .. }))
            .expect("destination write");
        assert!(last_read < first_write);
        assert_eq!(
            ops.iter()
                .filter(|op| matches!(op, MirOp::Barrier { .. }))
                .count(),
            4
        );
    }

    #[test]
    fn indirect_offset_overflow_advances_the_pointer_before_copying() {
        for offset in [250, 255, 256, 511] {
            let EndpointPlan::Prepared { setup, first_offset } =
                plan_endpoint(&computed(12, offset), 12).expect("prepared high-offset copy")
            else { panic!("expected a prepared address") };
            assert_eq!(first_offset, 0);
            assert!(matches!(setup.last(), Some(MirOp::AdvanceAddress {
                index: MirValue::ConstU16(actual), scale: 1, ..
            }) if *actual == offset));
        }
        assert!(matches!(
            plan_endpoint(&computed(12, 0), 257),
            Err(SelectionBlock::OffsetRange)
        ));
        assert!(matches!(
            plan_endpoint(&computed(256, 0), 1),
            Err(SelectionBlock::AddressForm)
        ));
    }
}
