//! Complete mixed-width parallel transfers, including DP/stack overlap and cycles.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Source {
    Home(Location),
    Immediate(u32),
    Staged(usize),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Copy {
    pub source: Source,
    pub destination: Location,
    pub bytes: u8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Step {
    Capture { source: Location, pool: usize },
    Move(Copy),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Plan {
    pub edge: Mir65816Edge,
    pub moves: Vec<Copy>,
    pub steps: Vec<Step>,
    pub widths: Vec<u8>,
}
impl Plan {
    fn new(edge: Mir65816Edge, moves: Vec<Copy>) -> Result<Self, String> {
        for (i, m) in moves.iter().enumerate() {
            if m.destination.slot().width != m.bytes
                || matches!(&m.source, Source::Home(h) if h.slot().width != m.bytes)
                || moves[..i]
                    .iter()
                    .any(|other| m.destination.overlaps(other.destination))
            {
                return Err("invalid complete mixed edge homes".into());
            }
        }
        let mut pending: Vec<_> = moves
            .iter()
            .filter(|m| m.source != Source::Home(m.destination))
            .cloned()
            .collect();
        let mut widths = Vec::new();
        let mut steps = Vec::new();
        while !pending.is_empty() {
            if let Some(i) = pending.iter().position(|m| {
                pending.iter().all(
                    |other| !matches!(&other.source, Source::Home(h) if m.destination.overlaps(*h)),
                )
            }) {
                steps.push(Step::Move(pending.remove(i)));
            } else {
                // Capture one complete source, including all reads of the same
                // range. Partial overlaps are blocked by byte extents too.
                let source = pending
                    .iter()
                    .find_map(|m| match m.source {
                        Source::Home(h) => Some(h),
                        _ => None,
                    })
                    .ok_or("mixed transfer cannot resolve cycle")?;
                let pool = widths.len();
                widths.push(source.slot().width);
                steps.push(Step::Capture { source, pool });
                for m in &mut pending {
                    if m.source == Source::Home(source) {
                        m.source = Source::Staged(pool);
                    }
                }
            }
        }
        Ok(Self {
            edge,
            moves,
            steps,
            widths,
        })
    }
}
impl AllocatedFrame {
    pub(super) fn mixed_copies(
        &self,
        r: &Mir65816Routine,
        edge: &Mir65816Edge,
    ) -> Result<Option<Plan>, String> {
        let widths = self.edge_widths(r, edge)?;
        let block = r.blocks.iter().find(|b| b.id == edge.target).unwrap();
        let mut moves = Vec::new();
        for ((value, &(id, _)), bytes) in edge.args.iter().zip(&block.params).zip(widths) {
            let source = match value {
                Mir65816Value::Temp(id, _) => {
                    Source::Home(*self.temps.get(id).ok_or("missing mixed source")?)
                }
                Mir65816Value::Param(id) => {
                    let (offset, width) = self.parameter_home(r, *id)?;
                    Source::Home(Location::Stack(Slot {
                        offset: offset as u16,
                        width,
                    }))
                }
                Mir65816Value::U8(n) => Source::Immediate(u32::from(*n)),
                Mir65816Value::U16(n) => Source::Immediate(u32::from(*n)),
                Mir65816Value::U24(n) | Mir65816Value::U32(n) => Source::Immediate(*n),
                _ => return Ok(None),
            };
            moves.push(Copy {
                source,
                destination: self.temps[&id],
                bytes,
            });
        }
        // Existing all-word/pointer selectors and stack-only fallback keep their
        // own contracts. Only mixed edges using the residence pool enter here.
        if !moves.iter().any(|m| {
            matches!(m.destination, Location::DirectPage(_))
                || matches!(m.source, Source::Home(Location::DirectPage(_)))
        }) {
            return Ok(None);
        }
        Ok(Some(Plan::new(edge.clone(), moves)?))
    }
    pub(super) fn mixed_staging(&self, plan: &Plan) -> Result<Vec<Slot>, String> {
        plan.widths.iter().enumerate().map(|(i, &bytes)| {
            let slot = *self.edge_copies.get(i).ok_or("missing mixed capture slot")?;
            if slot.width < bytes || !(1..=4).contains(&slot.width) ||
                plan.moves.iter().any(|m| Location::Stack(slot).overlaps(m.destination) ||
                    matches!(m.source, Source::Home(h) if Location::Stack(slot).overlaps(h))) ||
                self.edge_copies[..i].iter().any(|&s| Location::Stack(slot).overlaps(Location::Stack(s))) {
                return Err("mixed capture overlaps a live home or has incomplete width".into());
            }
            abi::stack::access_displacement(ByteOffset::new(slot.offset.into()), ByteSize::new(slot.width.into()), ByteSize::ZERO).map_err(|e| e.to_string())?;
            Ok(slot)
        }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn home(offset: u16, width: u8, dp: bool) -> Location {
        let s = Slot { offset, width };
        if dp {
            Location::DirectPage(s)
        } else {
            Location::Stack(s)
        }
    }
    fn key(h: Location, byte: u8) -> (bool, u16) {
        (
            matches!(h, Location::DirectPage(_)),
            h.slot().offset + u16::from(byte),
        )
    }
    fn check(moves: Vec<Copy>) -> Plan {
        let plan = Plan::new(
            Mir65816Edge {
                target: BlockId(1),
                args: vec![],
            },
            moves,
        )
        .unwrap();
        let mut memory = BTreeMap::new();
        for dp in [false, true] {
            for n in 0..256 {
                memory.insert(
                    (dp, n),
                    (n as u8).wrapping_mul(13).wrapping_add(u8::from(dp)),
                );
            }
        }
        let original = memory.clone();
        let read = |h: Location, m: &BTreeMap<_, _>| {
            (0..h.slot().width)
                .map(|byte| m[&key(h, byte)])
                .collect::<Vec<_>>()
        };
        let expected: Vec<_> = plan
            .moves
            .iter()
            .map(|m| match m.source {
                Source::Home(h) => read(h, &original),
                Source::Immediate(n) => n.to_le_bytes()[..usize::from(m.bytes)].to_vec(),
                Source::Staged(_) => panic!(),
            })
            .collect();
        let mut captures = BTreeMap::new();
        for step in &plan.steps {
            match step {
                Step::Capture { source, pool } => {
                    captures.insert(*pool, read(*source, &memory));
                }
                Step::Move(m) => {
                    let value = match m.source {
                        Source::Home(h) => read(h, &memory),
                        Source::Immediate(n) => n.to_le_bytes()[..usize::from(m.bytes)].to_vec(),
                        Source::Staged(i) => captures[&i].clone(),
                    };
                    for (byte, value) in value.into_iter().enumerate() {
                        memory.insert(key(m.destination, byte as u8), value);
                    }
                }
            }
        }
        for (m, expected) in plan.moves.iter().zip(expected) {
            assert_eq!(read(m.destination, &memory), expected);
        }
        for (&at, &before) in &original {
            if !plan
                .moves
                .iter()
                .any(|m| (0..m.bytes).any(|byte| key(m.destination, byte) == at))
            {
                assert_eq!(memory[&at], before);
            }
        }
        plan
    }

    #[test]
    fn mixed_schedules_match_simultaneous_byte_oracles_with_partial_overlap() {
        for dp in [false, true] {
            for a in [2, 160] {
                for b in [6, 164] {
                    let plan = check(vec![
                        Copy {
                            source: Source::Home(home(a, 3, dp)),
                            destination: home(b, 3, dp),
                            bytes: 3,
                        },
                        Copy {
                            source: Source::Home(home(b, 2, dp)),
                            destination: home(a, 2, dp),
                            bytes: 2,
                        },
                        Copy {
                            source: Source::Immediate(0xa5),
                            destination: home(200, 1, !dp),
                            bytes: 1,
                        },
                    ]);
                    assert_eq!(plan.widths, [3]);
                }
            }
            for width in 1..=4 {
                let plan = check(vec![Copy {
                    source: Source::Home(home(160, width, dp)),
                    destination: home(162, width, dp),
                    bytes: width,
                }]);
                assert_eq!(plan.widths.len(), usize::from(width > 2));
            }
        }
    }

    #[test]
    fn identities_namespaces_repeated_sources_and_independent_cycles_need_only_actual_captures() {
        let plan = check(vec![
            Copy {
                source: Source::Home(home(160, 3, true)),
                destination: home(164, 3, true),
                bytes: 3,
            },
            Copy {
                source: Source::Home(home(164, 3, true)),
                destination: home(160, 3, true),
                bytes: 3,
            },
            Copy {
                source: Source::Home(home(160, 3, true)),
                destination: home(168, 3, true),
                bytes: 3,
            },
            Copy {
                source: Source::Home(home(172, 2, true)),
                destination: home(174, 2, true),
                bytes: 2,
            },
            Copy {
                source: Source::Home(home(174, 2, true)),
                destination: home(172, 2, true),
                bytes: 2,
            },
            Copy {
                source: Source::Home(home(180, 1, true)),
                destination: home(180, 1, true),
                bytes: 1,
            },
            Copy {
                source: Source::Home(home(200, 4, false)),
                destination: home(200, 4, true),
                bytes: 4,
            },
        ]);
        assert_eq!(plan.widths, [3, 2]);
        assert_eq!(
            plan.steps
                .iter()
                .filter(|s| matches!(s, Step::Move(_)))
                .count(),
            6
        );
        assert!(
            Plan::new(
                plan.edge,
                vec![
                    Copy {
                        source: Source::Immediate(1),
                        destination: home(160, 3, true),
                        bytes: 3
                    },
                    Copy {
                        source: Source::Immediate(2),
                        destination: home(162, 2, true),
                        bytes: 2
                    },
                ]
            )
            .is_err()
        );
    }
}
