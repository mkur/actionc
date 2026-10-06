//! Select adjacent 24-bit loads into an unexposed local's final home.
use super::*;

#[derive(PartialEq, Eq)]
struct Load {
    temp: TempId,
    destination: Memory,
}

#[derive(Default, PartialEq, Eq)]
pub(in crate::mir65816::emit) struct Plan {
    loads: BTreeMap<(BlockId, usize), Load>,
    copies: BTreeSet<(BlockId, usize)>,
}

impl Plan {
    pub(in crate::mir65816::emit) fn placement_captures(
        &self,
    ) -> Vec<(TempId, crate::mir65816::analysis::ProgramPoint, Slot)> {
        self.loads
            .iter()
            .map(|(&(block, index), load)| {
                let Memory::Stack(offset) = load.destination else {
                    unreachable!("checked local destination")
                };
                (
                    load.temp,
                    crate::mir65816::analysis::ProgramPoint { block, index },
                    Slot {
                        offset: offset as u16,
                        width: 3,
                    },
                )
            })
            .collect()
    }

    pub(in crate::mir65816::emit) fn new(
        routine: &Mir65816Routine,
        demand: &home_demand::Plan,
        counts: &BTreeMap<TempId, usize>,
        definitions: &BTreeMap<TempId, usize>,
    ) -> Self {
        let mut plan = Self::default();
        for block in &routine.blocks {
            for (index, ops) in block.ops.windows(2).enumerate() {
                let (
                    Mir65816Op::Load {
                        dest,
                        width,
                        address: source,
                        volatile: false,
                    },
                    Mir65816Op::Store {
                        address,
                        value: Mir65816Value::Temp(input, size),
                        width: stored,
                        volatile: false,
                    },
                ) = (&ops[0], &ops[1])
                else {
                    continue;
                };
                if width.get() != 3
                    || width != size
                    || width != stored
                    || dest != input
                    || source.index.is_some()
                    || demand.omits(*dest)
                    || counts.get(dest) != Some(&1)
                    || definitions.get(dest) != Some(&1)
                {
                    continue;
                }
                // Reuse the routine-wide ownership proof, including escape,
                // partial/volatile access and aggregate-copy rejection. A
                // pointer read cannot alias an unexposed local's storage.
                let Ok(Some(local)) = pointer_forwarding::local(routine, address) else {
                    continue;
                };
                plan.loads.insert(
                    (block.id, index),
                    Load {
                        temp: *dest,
                        destination: local.memory(),
                    },
                );
                plan.copies.insert((block.id, index + 1));
            }
        }
        plan
    }

    pub(in crate::mir65816::emit) fn temps(&self) -> impl Iterator<Item = TempId> + '_ {
        self.loads.values().map(|load| load.temp)
    }

    pub(in crate::mir65816::emit) fn owns(&self, block: BlockId, index: usize) -> bool {
        self.loads.contains_key(&(block, index)) || self.copies.contains(&(block, index))
    }

    pub(super) fn emit(
        &self,
        b: &mut Builder<'_>,
        block: BlockId,
        index: usize,
        op: &Mir65816Op,
    ) -> Result<bool, String> {
        if self.copies.contains(&(block, index)) {
            b.code.barrier();
            return Ok(true);
        }
        let Some(load) = self.loads.get(&(block, index)) else {
            return Ok(false);
        };
        let Mir65816Op::Load { address, .. } = op else {
            return Err("missing direct local load".into());
        };
        if b.frame.temps.contains_key(&load.temp) {
            return Err("direct local load retains a captured temporary".into());
        }
        b.code.barrier();
        let source = b.prepare_address(address)?;
        b.transfer(source, load.destination, 3, true)?;
        Ok(true)
    }
}
