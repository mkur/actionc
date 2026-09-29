//! Pure 24-bit address producers and their complete scalar store consumers.
use super::*;

struct ComponentStore {
    source: Slot,
    offset: u16,
    subtract: bool,
}

/// Reuse the admitted sole-consumer chain, but send each arithmetic component
/// directly to memory instead of constructing an ABI return value in A/X.
pub(super) struct Plan {
    omitted: BTreeSet<usize>,
    stores: BTreeMap<usize, ComponentStore>,
}

impl Plan {
    pub(super) fn new(
        routine: &Mir65816Routine,
        frame: &AllocatedFrame,
        block: &Mir65816Block,
        demand: &home_demand::Plan,
        pointers: &pointer_forwarding::Plan,
    ) -> Result<Self, String> {
        let mut plan = Self {
            omitted: BTreeSet::new(),
            stores: BTreeMap::new(),
        };
        for (index, op) in block.ops.iter().enumerate() {
            let Some((base, offset, subtract)) = home_demand::pointer_expression(op) else {
                continue;
            };
            let Some(mut range) = demand.producer(block.id, index) else {
                continue;
            };
            if range.bytes != 3 {
                continue;
            }
            let mut omitted = vec![index];
            let store = loop {
                let next = range.consumer;
                if let Some(Mir65816Op::Store {
                    address,
                    width,
                    volatile: false,
                    ..
                }) = block.ops.get(next)
                    && width.get() == 3
                    && home_demand::indirect_store(routine, address, 3)
                {
                    break Some(next);
                }
                let Some(next_range) = demand.producer(block.id, next) else {
                    break None;
                };
                if next_range.bytes != 3
                    || pointer_forwarding::pointer_alias(routine, &block.ops[next]).is_none()
                {
                    break None;
                }
                omitted.push(next);
                range = next_range;
            };
            let Some(store) = store else { continue };
            // Only the existing checked alias chain and omitted private reads
            // may intervene. No observable source read or store is moved.
            if (index + 1..store).any(|i| {
                !omitted.contains(&i)
                    && !liveness::operation_output(&block.ops[i])
                        .is_some_and(|id| pointers.temps().any(|borrowed| borrowed == id))
            }) {
                continue;
            }
            let source = match base {
                Mir65816Value::Temp(id, width) if width.get() == 3 => pointers
                    .read_home((block.id, index), *id)
                    .or_else(|| frame.temps.get(id).and_then(|home| home.stack().ok())),
                Mir65816Value::Param(id) => {
                    let parameter = routine
                        .frame
                        .parameters
                        .iter()
                        .find(|p| p.param == *id)
                        .ok_or("missing address-component parameter")?;
                    if parameter.frame_object.is_some_and(|id| {
                        routine
                            .frame
                            .objects
                            .iter()
                            .any(|o| o.id == id && o.addressable)
                    }) {
                        continue;
                    }
                    let (offset, width) = frame.parameter_home(routine, *id)?;
                    Some(Slot {
                        offset: u16::try_from(offset).map_err(|_| "component source overflow")?,
                        width,
                    })
                }
                _ => None,
            };
            let Some(source) = source else { continue };
            if source.width != 3 {
                return Err("incomplete address-component source".into());
            }
            abi::stack::access_displacement(
                ByteOffset::new(source.offset.into()),
                ByteSize::new(3),
                ByteSize::ZERO,
            )
            .map_err(|e| e.to_string())?;
            plan.omitted.extend(omitted);
            plan.stores.insert(
                store,
                ComponentStore {
                    source,
                    offset: offset as u16,
                    subtract,
                },
            );
        }
        Ok(plan)
    }

    pub(super) fn emit(
        &self,
        b: &mut Builder<'_>,
        index: usize,
        op: &Mir65816Op,
    ) -> Result<bool, String> {
        if self.omitted.contains(&index) {
            return Ok(true);
        }
        let Some(store) = self.stores.get(&index) else {
            return Ok(false);
        };
        let Mir65816Op::Store { address, .. } = op else {
            return Err("missing component store".into());
        };
        let source = Memory::Stack(store.source.offset.into());
        let destination = b.prepare_address(address)?;
        b.check_transfer(source, destination, 3)?;
        b.code.barrier();
        b.code.a16();
        b.load_memory(source, 0)?;
        b.code.op(if store.subtract {
            Implied::Sec
        } else {
            Implied::Clc
        });
        b.code.word(
            if store.subtract {
                WordOp::SbcImm
            } else {
                WordOp::AdcImm
            },
            store.offset,
        );
        b.store_memory(destination, 0)?;
        // The authoritative source is an unexposed invocation home, not the
        // destination's DP address cache. STA, mode changes and LDY/INY leave
        // carry intact; no full 24-bit result or high-lane cleanup is needed.
        b.code.a8();
        b.load_memory(source, 2)?;
        b.code.byte(
            if store.subtract {
                ByteOp::SbcImm
            } else {
                ByteOp::AdcImm
            },
            0,
        );
        if !b.next_pointer_piece(destination, ByteOp::StaIndirectY) {
            b.store_memory(destination, 2)?;
        }
        b.code.barrier();
        Ok(true)
    }
}

impl Builder<'_> {
    pub(super) fn address_expression(&mut self, op: &Mir65816Op) -> Result<bool, String> {
        let Some((base, offset, subtract)) = home_demand::pointer_expression(op) else {
            return Ok(false);
        };
        self.long_expression_return(
            3,
            if subtract {
                NirBinaryOp::Sub
            } else {
                NirBinaryOp::Add
            },
            base,
            &Mir65816Value::U24(offset),
        )?;
        Ok(true)
    }

    pub(super) fn expression_store(
        &mut self,
        address: &Mir65816Address,
        bytes: u8,
    ) -> Result<(), String> {
        let indirect = matches!(address.base, Mir65816AddressBase::Indirect(_));
        // Address setup owns A but not X/Y for the admitted unindexed form.
        // Keep the low result in Y until setup finishes; X retains the bank.
        if indirect {
            self.code.a16();
            if bytes == 1 {
                self.code.word(WordOp::AndImm, 0xff);
            }
            self.code.op(Implied::Tay);
        }
        let destination = self.prepare_address(address)?;
        self.check_transfer(destination, destination, bytes)?;
        if indirect {
            self.code.a16();
            self.code.op(Implied::Tya);
        }
        let frame = if bytes == 2 {
            self.frame_word(address)?
        } else {
            None
        };
        if let Some((_, slot)) = frame {
            self.code.register_home(slot);
        }
        self.code.barrier();
        if bytes == 1 {
            self.code.a8();
        } else {
            self.code.a16();
        }
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
            self.code
                .remember_frame_word(object, address.displacement.get(), slot);
        }
        Ok(())
    }
}
