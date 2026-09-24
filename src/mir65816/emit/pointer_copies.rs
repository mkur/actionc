//! One checked captured-pointer edge move and its exact staging requirement.
use super::*;
use crate::target::{ByteOffset, ByteSize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PointerHome {
    Stack(u8),
    DirectPage(u8),
}
impl PointerHome {
    pub fn distance(self, other: Self) -> Option<u8> {
        match (self, other) {
            (Self::Stack(a), Self::Stack(b)) | (Self::DirectPage(a), Self::DirectPage(b)) => {
                Some(a.abs_diff(b))
            }
            _ => None,
        }
    }
}
pub(super) struct PointerCopies {
    pub source: PointerHome,
    pub destination: PointerHome,
}
impl PointerCopies {
    // Native word copies overwrite hidden B. Preserve the complete original A
    // in one private word, then restore B and establish the old final byte/NZ.
    // An identity only needs that final byte load and no staging storage.
    pub fn preserve_a(&self) -> bool {
        self.source != self.destination
    }
}
fn stack(offset: u32, delta: u32) -> Result<PointerHome, String> {
    abi::stack::access_displacement(
        ByteOffset::new(offset),
        ByteSize::new(3),
        ByteSize::new(delta),
    )
    .map(|n| PointerHome::Stack(n.get() as u8))
    .map_err(|e| e.to_string())
}
fn home(location: Location, delta: u32) -> Result<PointerHome, String> {
    if location.slot().width != 3 {
        return Err("pointer edge home width mismatch".into());
    }
    match location {
        Location::Stack(s) => stack(s.offset.into(), delta),
        Location::DirectPage(s) if u32::from(s.offset) + 3 <= abi::generated::DP_SCRATCH_SIZE => {
            Ok(PointerHome::DirectPage(s.offset as u8))
        }
        _ => Err("pointer edge exceeds private DP extent".into()),
    }
}
impl AllocatedFrame {
    pub(super) fn pointer_copies(
        &self,
        routine: &Mir65816Routine,
        edge: &Mir65816Edge,
        delta: u32,
    ) -> Result<Option<PointerCopies>, String> {
        if self.edge_widths(routine, edge)? != [3] {
            return Ok(None);
        }
        let block = routine.blocks.iter().find(|b| b.id == edge.target).unwrap();
        let destination = home(self.temps[&block.params[0].0], delta)?;
        let source = match &edge.args[0] {
            Mir65816Value::Temp(id, _) => home(
                *self.temps.get(id).ok_or("missing pointer edge source")?,
                delta,
            )?,
            Mir65816Value::Param(id) => stack(self.parameter_home(routine, *id)?.0, delta)?,
            _ => return Ok(None),
        };
        if source
            .distance(destination)
            .is_some_and(|d| d != 0 && d < 3)
        {
            return Ok(None);
        }
        Ok(Some(PointerCopies {
            source,
            destination,
        }))
    }
    pub(super) fn pointer_staging(
        &self,
        plan: &PointerCopies,
        delta: u32,
    ) -> Result<Option<u8>, String> {
        if !plan.preserve_a() {
            return Ok(None);
        }
        let slot = self
            .edge_copies
            .first()
            .ok_or("missing pointer edge A staging word")?;
        if !(2..=4).contains(&slot.width) {
            return Err("invalid pointer edge A staging width".into());
        }
        let at = abi::stack::access_displacement(
            ByteOffset::new(slot.offset.into()),
            ByteSize::new(2),
            ByteSize::new(delta),
        )
        .map_err(|e| e.to_string())?
        .get() as u8;
        for home in [plan.source, plan.destination] {
            if let PointerHome::Stack(n) = home
                && u16::from(at) < u16::from(n) + 3
                && u16::from(n) < u16::from(at) + 2
            {
                return Err("pointer edge A staging overlaps a live home".into());
            }
        }
        Ok(Some(at))
    }
}
