//! Physical bytes, separate from logical ownership. Stack coordinates use S at
//! invocation entry; DP coordinates use the current execution domain. ABI v1
//! requires disjoint nonwrapping stack/DP reservations and preserves both across
//! preemption. Unresolved addresses still potentially alias every tracked byte.
use super::super::effects::{Access, Memory};
use super::super::selected::{Action, SelectedRoutine};
use super::super::state::Environment;
use super::super::{AllocatedFrame, Location, Mir65816AbiHome, Mir65816Routine};
use super::sites::Node;
use crate::mir65816::{Mir65816FrameObjectId, abi::generated::*};
use crate::nir::{ParamId, TempId};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HomeByte {
    Stack(i32),
    DirectPage(u16),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HomeOwner {
    Temporary(TempId),
    EdgeStaging(usize),
    FrameObject(Mir65816FrameObjectId),
    Incoming(ParamId),
    ReturnAddress,
    DomainScratch,
    DomainMetadata,
    /// Includes outgoing arguments, transfer storage, padding and unowned bytes.
    ProtectedStack,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HomeInfo {
    pub owners: BTreeSet<HomeOwner>,
    pub private: bool,
    pub entry_defined: bool,
}
impl HomeInfo {
    fn new(owner: HomeOwner, private: bool, entry_defined: bool) -> Self {
        Self {
            owners: [owner].into(),
            private,
            entry_defined,
        }
    }
}

/// Sealed provenance captured after MIR verification and allocation checks.
/// Numeric stack/DP operands alone cannot construct private ownership.
#[derive(Clone, Debug)]
pub(in crate::mir65816::emit) struct HomeContract {
    homes: BTreeMap<HomeByte, HomeInfo>,
    stack_min: i32,
    stack_max: i32,
}
impl HomeContract {
    pub fn from_verified(
        routine: &Mir65816Routine,
        frame: &AllocatedFrame,
    ) -> Result<Self, String> {
        let mut this = Self {
            homes: BTreeMap::new(),
            stack_min: 1 - i32::from(frame.peak_below_entry),
            stack_max: i32::try_from(
                CALL_RETURN_ADDRESS_BYTES + routine.frame.incoming_extent.get(),
            )
            .map_err(|_| "home incoming range overflow")?,
        };
        if frame.extent > frame.peak_below_entry
            || this.stack_max > 65535
            || i64::from(this.stack_max) - i64::from(this.stack_min) >= 65536
        {
            return Err("home contract exceeds nonwrapping bank-zero stack".into());
        }
        for (&temp, &location) in &frame.temps {
            let slot = location.slot();
            for byte in 0..slot.width {
                let home = match location {
                    Location::Stack(_) => HomeByte::Stack(
                        i32::from(slot.offset) + i32::from(byte) - i32::from(frame.extent),
                    ),
                    Location::DirectPage(_) => HomeByte::DirectPage(slot.offset + u16::from(byte)),
                };
                this.insert(home, HomeOwner::Temporary(temp), true, false);
            }
        }
        for (index, slot) in frame.edge_copies.iter().enumerate() {
            for byte in 0..slot.width {
                this.insert(
                    HomeByte::Stack(
                        i32::from(slot.offset) + i32::from(byte) - i32::from(frame.extent),
                    ),
                    HomeOwner::EdgeStaging(index),
                    true,
                    false,
                );
            }
        }
        for object in &routine.frame.objects {
            let start = i32::try_from(object.stack_offset.get())
                .map_err(|_| "home object offset overflow")?
                - i32::from(frame.extent);
            for byte in 0..object.size.get() {
                let offset = start
                    .checked_add(i32::try_from(byte).map_err(|_| "home object size overflow")?)
                    .ok_or("home object range overflow")?;
                this.insert(
                    HomeByte::Stack(offset),
                    HomeOwner::FrameObject(object.id),
                    !object.addressable,
                    false,
                );
            }
        }
        for offset in 1..=CALL_RETURN_ADDRESS_BYTES as i32 {
            this.insert(
                HomeByte::Stack(offset),
                HomeOwner::ReturnAddress,
                false,
                true,
            );
        }
        for param in &routine.frame.parameters {
            let Mir65816AbiHome::StackArgument { offset, size, .. } = param.incoming else {
                return Err("home contract requires native stack parameters".into());
            };
            for byte in 0..size.get() {
                let offset = i32::try_from(CALL_ENTRY_FIRST_ARGUMENT_OFFSET + offset.get() + byte)
                    .map_err(|_| "home parameter overflow")?;
                this.insert(
                    HomeByte::Stack(offset),
                    HomeOwner::Incoming(param.param),
                    false,
                    true,
                );
            }
        }
        // Fixed helper scratch and promoted temps share these exact bytes.
        for offset in 0..DP_SIZE as u16 {
            let scratch = u32::from(offset) < DP_SCRATCH_SIZE;
            this.insert(
                HomeByte::DirectPage(offset),
                if scratch {
                    HomeOwner::DomainScratch
                } else {
                    HomeOwner::DomainMetadata
                },
                scratch,
                !scratch,
            );
        }
        if this.homes.keys().any(|home| match *home {
            HomeByte::Stack(offset) => !(this.stack_min..=this.stack_max).contains(&offset),
            HomeByte::DirectPage(offset) => u32::from(offset) >= DP_SIZE,
        }) {
            return Err("allocated home outside verified native reservation".into());
        }
        Ok(this)
    }
    fn insert(&mut self, home: HomeByte, owner: HomeOwner, private: bool, entry_defined: bool) {
        self.homes
            .entry(home)
            .and_modify(|info| {
                info.owners.insert(owner);
                info.private &= private;
                info.entry_defined |= entry_defined;
            })
            .or_insert_with(|| HomeInfo::new(owner, private, entry_defined));
    }
    pub(in crate::mir65816::emit) fn range(
        &self,
        memory: Memory,
        env: Environment,
    ) -> Option<BTreeSet<HomeByte>> {
        if !env.native {
            return None;
        }
        match memory {
            Memory::Stack {
                displacement,
                bytes,
            } => {
                let start = i64::from(displacement).checked_sub(env.depth)?;
                let end = start.checked_add(i64::from(bytes).checked_sub(1)?)?;
                if bytes == 0
                    || start < i64::from(self.stack_min)
                    || end > i64::from(self.stack_max)
                {
                    return None;
                }
                Some(
                    (start..=end)
                        .map(|offset| HomeByte::Stack(offset as i32))
                        .collect(),
                )
            }
            Memory::DirectPage { offset, bytes } if env.current_domain => {
                let end = u32::from(offset).checked_add(u32::from(bytes))?;
                // Page-aligned D plus offsets within the full ABI block cannot wrap.
                if bytes == 0 || end > DP_SIZE {
                    return None;
                }
                Some(
                    (u32::from(offset)..end)
                        .map(|offset| HomeByte::DirectPage(offset as u16))
                        .collect(),
                )
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HomeAccess {
    pub access: Access,
    pub homes: BTreeSet<HomeByte>,
    /// Potential aliases, not exact accesses. Never kills liveness/definitions.
    pub uncertain: bool,
}
#[derive(Clone, Debug)]
pub(in crate::mir65816::emit) struct Homes {
    pub info: BTreeMap<HomeByte, HomeInfo>,
    pub accesses: BTreeMap<Node, Vec<HomeAccess>>,
}
impl Homes {
    pub fn analyze(selected: &SelectedRoutine) -> Result<Self, String> {
        let contract = selected
            .home_contract
            .as_ref()
            .ok_or("missing verified home contract")?;
        let mut info = contract.homes.clone();
        // Collect all exact ranges first, so unresolved reads see the complete
        // universe, including outgoing/transfer bytes found later in selection.
        for record in selected.records() {
            if let Action::Instruction { effects, .. } = &record.action {
                for effect in &effects.memory {
                    if let Some(range) = contract.range(effect.memory, record.before.env) {
                        for home in range {
                            info.entry(home).or_insert_with(|| {
                                HomeInfo::new(HomeOwner::ProtectedStack, false, false)
                            });
                        }
                    }
                }
            }
        }
        let universe = info.keys().copied().collect::<BTreeSet<_>>();
        let protected = info
            .iter()
            .filter_map(|(&home, info)| (!info.private).then_some(home))
            .collect::<BTreeSet<_>>();
        let mut accesses = BTreeMap::new();
        for (index, record) in selected.records().iter().enumerate() {
            let mut ordered = Vec::new();
            match &record.action {
                Action::Instruction { effects, .. } => {
                    for effect in &effects.memory {
                        let range = contract.range(effect.memory, record.before.env);
                        let uncertain = range.is_none();
                        ordered.push(HomeAccess {
                            access: if uncertain && effect.access == Access::Write {
                                Access::MayWrite
                            } else {
                                effect.access
                            },
                            homes: range.unwrap_or_else(|| universe.clone()),
                            uncertain,
                        });
                    }
                }
                // Stack arguments, return storage, addressable objects and
                // metadata remain protected; results themselves are registers.
                Action::ReturnExit => ordered.push(HomeAccess {
                    access: Access::Read,
                    homes: protected.clone(),
                    uncertain: false,
                }),
                // A terminal raw fault adapter has no narrower observation contract.
                Action::FaultExit => ordered.push(HomeAccess {
                    access: Access::Read,
                    homes: universe.clone(),
                    uncertain: true,
                }),
                _ => {}
            }
            accesses.insert(Node(index), ordered);
        }
        Ok(Self { info, accesses })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn contract() -> HomeContract {
        HomeContract {
            homes: BTreeMap::new(),
            stack_min: -19,
            stack_max: 10,
        }
    }
    #[test]
    fn stack_identity_uses_entry_s_and_checked_reservation() {
        let c = contract();
        let env = Environment {
            depth: 8,
            ..super::super::super::state::State65816::default().env
        };
        assert_eq!(
            c.range(
                Memory::Stack {
                    displacement: 7,
                    bytes: 2
                },
                env
            ),
            Some([HomeByte::Stack(-1), HomeByte::Stack(0)].into())
        );
        let moved = Environment { depth: 12, ..env };
        assert_eq!(
            c.range(
                Memory::Stack {
                    displacement: 11,
                    bytes: 2
                },
                moved
            ),
            c.range(
                Memory::Stack {
                    displacement: 7,
                    bytes: 2
                },
                env
            )
        );
        assert_ne!(
            c.range(
                Memory::Stack {
                    displacement: 7,
                    bytes: 2
                },
                moved
            ),
            c.range(
                Memory::Stack {
                    displacement: 7,
                    bytes: 2
                },
                env
            )
        );
        for (displacement, bytes) in [(-20, 1), (10, 2), (1, 0), (i32::MAX, 2)] {
            assert!(
                c.range(
                    Memory::Stack {
                        displacement,
                        bytes
                    },
                    super::super::super::state::State65816::default().env
                )
                .is_none()
            );
        }
        assert!(
            c.range(
                Memory::Stack {
                    displacement: 0,
                    bytes: 1
                },
                Environment {
                    depth: i64::MIN,
                    ..env
                }
            )
            .is_none()
        );
    }
    #[test]
    fn domain_ranges_alias_scratch_but_do_not_assume_wrap_or_numeric_disjointness() {
        let c = contract();
        let env = super::super::super::state::State65816::default().env;
        assert_eq!(
            c.range(
                Memory::DirectPage {
                    offset: 3,
                    bytes: 3
                },
                env
            ),
            Some([3, 4, 5].map(HomeByte::DirectPage).into())
        );
        for (offset, bytes) in [(255, 2), (65535, 2), (0, 0)] {
            assert!(c.range(Memory::DirectPage { offset, bytes }, env).is_none());
        }
        assert!(
            c.range(
                Memory::DirectPage {
                    offset: 0,
                    bytes: 2
                },
                Environment {
                    current_domain: false,
                    ..env
                }
            )
            .is_none()
        );
        assert!(
            c.range(
                Memory::Long {
                    address: 0x1234,
                    bytes: 2
                },
                env
            )
            .is_none()
        );
        assert!(
            c.range(
                Memory::IndirectLong {
                    pointer: 3,
                    indexed_y: true,
                    bytes: 2
                },
                env
            )
            .is_none()
        );
    }
    #[test]
    fn reused_names_share_physical_bytes_and_protected_ownership_wins() {
        let mut c = contract();
        let home = HomeByte::Stack(-2);
        c.insert(home, HomeOwner::Temporary(TempId(0)), true, false);
        c.insert(home, HomeOwner::Temporary(TempId(1)), true, false);
        c.insert(home, HomeOwner::EdgeStaging(0), true, false);
        assert_eq!(c.homes.len(), 1);
        assert_eq!(c.homes[&home].owners.len(), 3);
        assert!(c.homes[&home].private);
        c.insert(
            home,
            HomeOwner::FrameObject(Mir65816FrameObjectId(1)),
            false,
            false,
        );
        assert!(!c.homes[&home].private);
    }
}
