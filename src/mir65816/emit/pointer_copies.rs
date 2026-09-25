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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PointerCopy {
    Move(PointerHome, PointerHome),
    Capture(PointerHome),
    Restore(PointerHome),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PointerStaging {
    pub a: u8,
    pub capture: Option<u8>,
}
pub(super) struct PointerCopies {
    moves: Vec<(PointerHome, PointerHome)>,
    steps: Vec<PointerCopy>,
    cyclic: bool,
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
        let mut pending: Vec<_> = moves
            .iter()
            .copied()
            .filter(|&(s, d)| s != d)
            .map(|(s, d)| (Some(s), d))
            .collect();
        let mut steps = Vec::new();
        let mut cyclic = false;
        while !pending.is_empty() {
            // Consume every complete source before overwriting its home. Never
            // schedule the two overlapping word pieces as independent moves.
            if let Some(at) = pending.iter().position(|&(_, destination)| {
                pending
                    .iter()
                    .all(|&(source, _)| source != Some(destination))
            }) {
                let (source, destination) = pending.remove(at);
                steps.push(match source {
                    Some(source) => PointerCopy::Move(source, destination),
                    None => PointerCopy::Restore(destination),
                });
            } else {
                // Only cycles remain. Save one destination, replacing every
                // use of its old value. This opens a chain ending at Restore;
                // the capture is consumed before another cycle can be blocked.
                debug_assert!(pending.iter().all(|&(s, _)| s.is_some()));
                let saved = pending[0].1;
                steps.push(PointerCopy::Capture(saved));
                for (source, _) in &mut pending {
                    if *source == Some(saved) {
                        *source = None;
                    }
                }
                cyclic = true;
            }
        }
        Some(Self {
            moves,
            steps,
            cyclic,
        })
    }
    pub fn scheduled(
        &self,
        staging: PointerStaging,
    ) -> impl Iterator<Item = (PointerHome, PointerHome)> + '_ {
        self.steps.iter().map(move |&step| match step {
            PointerCopy::Move(s, d) => (s, d),
            PointerCopy::Capture(s) => (
                s,
                PointerHome::Stack(staging.capture.expect("checked capture slot")),
            ),
            PointerCopy::Restore(d) => (
                PointerHome::Stack(staging.capture.expect("checked capture slot")),
                d,
            ),
        })
    }
    pub fn staging_widths(&self) -> Vec<u8> {
        if self.cyclic {
            vec![2, 3]
        } else {
            vec![2; usize::from(self.preserve_a())]
        }
    }
    pub fn final_destination(&self) -> PointerHome {
        self.moves.last().unwrap().1
    }
    // Native word copies overwrite hidden B. Preserve the complete original A
    // in one private word, then restore B and establish the old final byte/NZ.
    // An identity only needs that final byte load and no staging storage.
    pub fn preserve_a(&self) -> bool {
        !self.steps.is_empty()
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
    ) -> Result<Option<PointerStaging>, String> {
        let widths = plan.staging_widths();
        let mut slots = Vec::new();
        for (index, &width) in widths.iter().enumerate() {
            let slot = self
                .edge_copies
                .get(index)
                .ok_or("missing pointer edge staging slot")?;
            if !(width..=4).contains(&slot.width) {
                return Err("invalid pointer edge staging width".into());
            }
            let at = abi::stack::access_displacement(
                ByteOffset::new(slot.offset.into()),
                ByteSize::new(width.into()),
                ByteSize::new(delta),
            )
            .map_err(|e| e.to_string())?
            .get() as u8;
            let overlap = |n: u8, w: u8| {
                u16::from(at) < u16::from(n) + u16::from(w)
                    && u16::from(n) < u16::from(at) + u16::from(width)
            };
            if plan
                .moves
                .iter()
                .flat_map(|&(s, d)| [s, d])
                .any(|home| matches!(home, PointerHome::Stack(n) if overlap(n, 3)))
                || slots.iter().zip(&widths).any(|(&n, &w)| overlap(n, w))
            {
                return Err("pointer edge staging overlaps a live home or staging slot".into());
            }
            slots.push(at);
        }
        Ok(slots.first().map(|&a| PointerStaging {
            a,
            capture: slots.get(1).copied(),
        }))
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
                    assert!(plan.is_some(), "{sources:?}");
                    if let Some(plan) = plan {
                        assert_eq!(plan.cyclic, !possible, "{sources:?}");
                        let mut memory = [0u8; 32];
                        for (i, value) in initial.iter().enumerate() {
                            memory[4 + 3 * i..7 + 3 * i].copy_from_slice(&value.to_le_bytes()[..3]);
                        }
                        for (s, d) in plan.scheduled(PointerStaging {
                            a: 24,
                            capture: Some(26),
                        }) {
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
    fn schedules_reject_partial_geometry_and_support_cycles_across_private_spaces() {
        use PointerHome::{DirectPage as D, Stack as S};
        for moves in [
            vec![(S(10), S(20)), (S(21), S(30))],
            vec![(S(10), S(20)), (S(30), S(22))],
            vec![(S(10), S(20)), (S(30), S(20))],
        ] {
            assert!(PointerCopies::plan(moves).is_none());
        }
        let plan = PointerCopies::plan(vec![(S(10), D(10)), (D(10), S(20))]).unwrap();
        assert_eq!(
            plan.steps,
            [
                PointerCopy::Move(D(10), S(20)),
                PointerCopy::Move(S(10), D(10))
            ]
        );
        let plan = PointerCopies::plan(vec![
            (S(10), D(10)),
            (D(10), S(10)),
            (S(20), S(30)),
            (S(30), S(20)),
        ])
        .unwrap();
        assert_eq!(plan.staging_widths(), [2, 3]);
        assert_eq!(
            plan.steps,
            [
                PointerCopy::Capture(D(10)),
                PointerCopy::Move(S(10), D(10)),
                PointerCopy::Restore(S(10)),
                PointerCopy::Capture(S(30)),
                PointerCopy::Move(S(20), S(30)),
                PointerCopy::Restore(S(20)),
            ]
        );
    }
}
