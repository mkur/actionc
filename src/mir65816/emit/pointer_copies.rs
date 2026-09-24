//! Checked whole-pointer edge scheduling and its exact staging requirement.
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
    moves: Vec<(PointerHome, PointerHome)>,
    order: Vec<usize>,
}
impl PointerCopies {
    fn plan(moves: Vec<(PointerHome, PointerHome)>) -> Option<Self> {
        if moves.is_empty()
            || moves.iter().enumerate().any(|(i, &(source, destination))| {
                moves[..i]
                    .iter()
                    .any(|&(_, d)| destination.distance(d).is_some_and(|n| n < 3))
                    || moves
                        .iter()
                        .any(|&(_, d)| source.distance(d).is_some_and(|n| n != 0 && n < 3))
            })
        {
            return None;
        }
        let mut pending: Vec<_> = (0..moves.len())
            .filter(|&i| moves[i].0 != moves[i].1)
            .collect();
        let mut order = Vec::new();
        while !pending.is_empty() {
            // Consume every complete source before overwriting its home. Never
            // schedule the two overlapping word pieces as independent moves.
            let at = pending
                .iter()
                .position(|&i| pending.iter().all(|&j| i == j || moves[j].0 != moves[i].1))?;
            order.push(pending.remove(at));
        }
        Some(Self { moves, order })
    }
    pub fn scheduled(&self) -> impl Iterator<Item = (PointerHome, PointerHome)> + '_ {
        self.order.iter().map(|&i| self.moves[i])
    }
    pub fn final_destination(&self) -> PointerHome {
        self.moves.last().unwrap().1
    }
    // Native word copies overwrite hidden B. Preserve the complete original A
    // in one private word, then restore B and establish the old final byte/NZ.
    // An identity only needs that final byte load and no staging storage.
    pub fn preserve_a(&self) -> bool {
        !self.order.is_empty()
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
        let widths = self.edge_widths(routine, edge)?;
        if widths.is_empty() || widths.iter().any(|&w| w != 3) {
            return Ok(None);
        }
        let block = routine.blocks.iter().find(|b| b.id == edge.target).unwrap();
        let mut moves = Vec::new();
        for (value, &(dest, _)) in edge.args.iter().zip(&block.params) {
            let destination = home(self.temps[&dest], delta)?;
            let source = match value {
                Mir65816Value::Temp(id, _) => home(
                    *self.temps.get(id).ok_or("missing pointer edge source")?,
                    delta,
                )?,
                Mir65816Value::Param(id) => stack(self.parameter_home(routine, *id)?.0, delta)?,
                _ => return Ok(None),
            };
            moves.push((source, destination));
        }
        Ok(PointerCopies::plan(moves))
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
        for home in plan.moves.iter().flat_map(|&(s, d)| [s, d]) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_match_exhaustive_simultaneous_three_pointer_assignments() {
        let orders = [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ];
        for a in 0..5 {
            for b in 0..5 {
                for c in 0..5 {
                    let sources = [a, b, c];
                    let initial = [0x123456u32, 0x80ff00, 0xffffff, 0, 0x010203];
                    let mut expected = initial;
                    for i in 0..3 {
                        expected[i] = initial[sources[i]];
                    }
                    let possible = orders.iter().any(|order| {
                        let mut actual = initial;
                        for &i in order {
                            actual[i] = actual[sources[i]];
                        }
                        actual == expected
                    });
                    let plan = PointerCopies::plan(
                        (0..3)
                            .map(|i| {
                                (
                                    PointerHome::Stack(4 + 3 * sources[i] as u8),
                                    PointerHome::Stack(4 + 3 * i as u8),
                                )
                            })
                            .collect(),
                    );
                    assert_eq!(plan.is_some(), possible, "{sources:?}");
                    if let Some(plan) = plan {
                        let mut memory = [0u8; 32];
                        for (i, value) in initial.iter().enumerate() {
                            memory[4 + 3 * i..7 + 3 * i].copy_from_slice(&value.to_le_bytes()[..3]);
                        }
                        for (s, d) in plan.scheduled() {
                            let (PointerHome::Stack(s), PointerHome::Stack(d)) = (s, d) else {
                                panic!()
                            };
                            // Execute the actual overlapping words, not atomic pointer moves.
                            for byte in [0, 1] {
                                let word =
                                    [memory[s as usize + byte], memory[s as usize + byte + 1]];
                                memory[d as usize + byte..d as usize + byte + 2]
                                    .copy_from_slice(&word);
                            }
                        }
                        for (i, value) in expected.iter().enumerate() {
                            assert_eq!(&memory[4 + 3 * i..7 + 3 * i], &value.to_le_bytes()[..3]);
                        }
                        assert_eq!(plan.preserve_a(), sources != [0, 1, 2]);
                        assert_eq!(plan.final_destination(), PointerHome::Stack(10));
                    }
                }
            }
        }
    }

    #[test]
    fn schedules_reject_partial_geometry_and_cycles_across_private_spaces() {
        use PointerHome::{DirectPage as D, Stack as S};
        for moves in [
            vec![(S(10), S(20)), (S(21), S(30))],
            vec![(S(10), S(20)), (S(30), S(22))],
            vec![(S(10), S(20)), (S(30), S(20))],
            vec![(S(10), S(20)), (S(20), S(10))],
            vec![(S(10), D(10)), (D(10), S(10))],
        ] {
            assert!(PointerCopies::plan(moves).is_none());
        }
        let plan = PointerCopies::plan(vec![(S(10), D(10)), (D(10), S(20))]).unwrap();
        assert_eq!(plan.order, [1, 0]);
    }
}
