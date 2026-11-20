//! Two routines overlap on the call stack only when one can run under a call
//! where the other needs the byte. Addressable entries can also be reentered.
use super::*;
use super::interference::{Byte, FrameLives};
use crate::mir6502::analysis::{record_bases::{MirPhysicalMemory, SourceObject}, sites::MirSite};
use super::super::logical::ObjectByte;

pub(super) struct Contexts {
    pub frames: BTreeMap<RoutineId, FrameLives>,
    descendants: BTreeMap<RoutineId, BTreeSet<RoutineId>>,
    targets: BTreeMap<(RoutineId, MirSite), RoutineId>,
    asynchronous: BTreeSet<RoutineId>,
    occupants: BTreeMap<u8, Vec<(RoutineId, Byte)>>,
    pub reserved: BTreeSet<u8>,
}

impl Contexts {
    pub fn new(program: &MirProgram) -> Self {
        let mut frames = BTreeMap::new();
        let mut targets = BTreeMap::new();
        let mut children = BTreeMap::<_, Vec<_>>::new();
        let mut occupants = BTreeMap::<_, Vec<_>>::new();
        let mut reserved: BTreeSet<_> = persistent::slots(program).collect();
        let addresses = ObjectAddressControl::for_home_placement(program);
        for r in &program.routines {
            let lives = MirCfg::from_routine(r).ok().and_then(|cfg| FrameLives::new(r, &cfg));
            for (mem, slot) in r.frame.resident_bytes() {
                if lives.is_none() || !addresses.compiler_controls_address(r.id, &mem) {
                    reserved.insert(slot.0);
                } else if let Some(byte) = ObjectByte::from_mem(&mem, 0) {
                    occupants.entry(slot.0).or_default().push((r.id, Byte::Source(byte)));
                }
            }
            // Compiler scratch can also carry a value through a call. Named
            // reservations are represented by their logical bytes above.
            let named: BTreeSet<_> = r.frame.resident_bytes().into_iter().map(|(_, s)| s).collect();
            for &slot in &r.frame.fixed_zero_page {
                let key = Byte::Home(MirHomeByte::FixedZeroPage(slot));
                if !named.contains(&slot) && lives.as_ref().is_some_and(|f| f.get(key).is_some()) {
                    occupants.entry(slot.0).or_default().push((r.id, key));
                }
            }
            for a in &r.frame.zero_page_allocations {
                for offset in 0..u16::from(a.size) {
                    if let Some(address) = a.start.0.checked_add(offset as u8) {
                        occupants.entry(address).or_default().push((r.id, Byte::Home(
                            MirHomeByte::VirtualZeroPage { slot: a.slot, offset })));
                    }
                }
            }
            for b in &r.blocks {
                for (op_index, op) in b.ops.iter().enumerate() {
                    if let MirOp::Call { target: MirCallTarget::Routine(id), .. } = op {
                        targets.insert((r.id, MirSite::Op { block: b.id, op_index }), *id);
                        children.entry(r.id).or_default().push(*id);
                    }
                }
            }
            if let Some(lives) = lives { frames.insert(r.id, lives); }
        }
        let closure = |roots: Vec<_>| {
            let mut pending = roots;
            let mut reached = BTreeSet::new();
            while let Some(id) = pending.pop() {
                if reached.insert(id) {
                    pending.extend(children.get(&id).into_iter().flatten().copied());
                }
            }
            reached
        };
        let asynchronous = closure(crate::mir6502::materialize::escaped_routines(program).into_iter().collect());
        let descendants = children.iter().map(|(&id, targets)| (id, closure(targets.clone()))).collect();
        Self { frames, descendants, targets, asynchronous, occupants, reserved }
    }

    fn crosses(&self, owner: RoutineId, byte: Byte, other: RoutineId) -> bool {
        let Some(frame) = self.frames.get(&owner) else { return true };
        // Even a read-only callee can have an implicit incoming parameter.
        // An absent range has no live value at any call.
        frame.get(byte).is_some_and(|range| range.across_calls.iter().any(|site| {
            self.targets.get(&(owner, *site)).is_some_and(|&id|
                id == other || self.descendants.get(&id).is_some_and(|desc| desc.contains(&other)))
        }))
    }

    pub fn fits(&self, owner: RoutineId, value: Byte, address: u8) -> bool {
        if self.reserved.contains(&address) || self.crosses(owner, value, owner) { return false; }
        self.occupants.get(&address).into_iter().flatten().all(|&(other, occupant)| {
            if owner == other { return value == occupant; }
            !self.asynchronous.contains(&owner) && !self.asynchronous.contains(&other)
                && !self.crosses(owner, value, other) && !self.crosses(other, occupant, owner)
        })
    }

    pub fn bind(&mut self, owner: RoutineId, value: Byte, address: u8) {
        self.occupants.entry(address).or_default().push((owner, value));
    }

    /// A final source range can cover this byte only through its exact named
    /// resident lane. Fixed aliases and incomplete inventories cannot share.
    pub fn source_allows(&self, memory: &MirPhysicalMemory, address: u8) -> bool {
        memory.source_storage().is_some_and(|inventory| inventory.iter()
            .filter(|record| record.contains(u16::from(address))).all(|record| {
                let SourceObject::Routine { owner, ref base, .. } = record.object else { return false };
                let Some(offset) = record.offset.checked_add(u16::from(address).wrapping_sub(record.address)) else { return false };
                let key = match *base {
                    MirStorageBase::Param(id) => ObjectByte::Param(id, offset),
                    MirStorageBase::Local(id) => ObjectByte::Local(id, offset),
                    _ => return false,
                };
                self.occupants.get(&address).is_some_and(|v| v.contains(&(owner, Byte::Source(key))))
            }))
    }
}
