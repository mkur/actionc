//! One-pass edge affinities with fixed parameter anchors and verified rollback.
use super::{allocation::*, copies::WordStrategy, liveness::Interference, *};
use std::collections::{BTreeMap, BTreeSet};

impl AllocatedFrame {
    pub(super) fn coalesce_edges(
        &mut self,
        routine: &Mir65816Routine,
        graph: &Interference,
    ) -> Result<(), String> {
        let anchors: BTreeSet<_> = routine
            .blocks
            .iter()
            .flat_map(|b| b.params.iter().map(|p| p.0))
            .collect();
        let edges: Vec<_> = routine
            .blocks
            .iter()
            .flat_map(|b| match &b.terminator {
                Mir65816Terminator::Goto(e) => vec![e],
                Mir65816Terminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } => vec![then_edge, else_edge],
                _ => vec![],
            })
            .collect();
        for edge in &edges {
            let Some(plan) = self.word_copies(routine, edge, 0)? else {
                continue;
            };
            if !matches!(plan.strategy, WordStrategy::Direct(_)) {
                continue;
            }
            let target = routine.blocks.iter().find(|b| b.id == edge.target).unwrap();
            let mut changes = BTreeMap::new();
            let mut consistent = true;
            for (v, (dest, _)) in edge.args.iter().zip(&target.params) {
                let Mir65816Value::Temp(source, w) = v else {
                    continue;
                };
                if w.get() != 2
                    || anchors.contains(source)
                    || source == dest
                    || graph[source].contains(dest)
                {
                    continue;
                }
                let (Location::Stack(s), Location::Stack(d)) =
                    (self.temps[source], self.temps[dest])
                else {
                    continue;
                };
                if s.width != 2 || d.width != 2 {
                    continue;
                }
                // Include existing equalities: a repeated source cannot silently
                // abandon one anchored destination to satisfy another.
                if let Some(old) = changes.insert(*source, Location::Stack(d)) {
                    consistent &= old == Location::Stack(d);
                }
            }
            if !consistent || changes.iter().all(|(id, home)| self.temps[id] == *home) {
                continue;
            }
            let mut trial = self.clone();
            trial.temps.extend(changes);
            // Full verification includes every third-party interference, exact
            // frame accounting and all staging requirements, not just this edge.
            if trial.verify_stack(routine).is_err() {
                continue;
            }
            let mut improves = false;
            let mut no_regression = true;
            for e in &edges {
                let before = self.word_copies(routine, e, 0)?.map(|p| p.cost());
                let after = trial.word_copies(routine, e, 0)?.map(|p| p.cost());
                match (before, after) {
                    (Some((bb, bc)), Some((ab, ac))) => {
                        no_regression &= ab <= bb && ac <= bc;
                        improves |= ab < bb || ac < bc;
                    }
                    (None, None) => (), // Same bytewise widths and reservations.
                    _ => no_regression = false,
                }
            }
            if no_regression && improves {
                *self = trial;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn example() -> (Mir65816Routine, AllocatedFrame) {
        let ast = crate::parser::parse(
            &crate::lexer::tokenize("CARD FUNC Work(CARD a,b) RETURN(a+b)").unwrap(),
        )
        .unwrap();
        let model = crate::semantic::analyze_with_options(
            &ast,
            crate::semantic::SemanticOptions::modern()
                .with_target(crate::target::TargetId::Wdc65816Native),
        )
        .unwrap();
        let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
        let mut r = crate::mir65816::lower_program(&nir)
            .unwrap()
            .routines
            .remove(0);
        let ty = r.temps[0].1.clone();
        let w = ByteSize::new(2);
        r.temps = (0..4).map(|i| (TempId(i), ty.clone())).collect();
        let ops = r.blocks[0].ops[..2].to_vec();
        let mut ret = r.blocks[0].terminator.clone();
        let Mir65816Terminator::Return { value, .. } = &mut ret else {
            panic!()
        };
        *value = Some(Mir65816Value::Temp(TempId(2), w));
        r.blocks = vec![
            Mir65816Block {
                id: BlockId(0),
                params: vec![],
                ops,
                terminator: Mir65816Terminator::Goto(Mir65816Edge {
                    target: BlockId(1),
                    args: vec![
                        Mir65816Value::Temp(TempId(0), w),
                        Mir65816Value::Temp(TempId(1), w),
                    ],
                }),
            },
            Mir65816Block {
                id: BlockId(1),
                params: vec![(TempId(2), w), (TempId(3), w)],
                ops: vec![],
                terminator: ret,
            },
        ];
        let f = AllocatedFrame {
            extent: 8,
            spill_bytes: 8,
            peak_below_entry: 8,
            temps: [(0, 2), (1, 4), (2, 6), (3, 2)]
                .into_iter()
                .map(|(id, offset)| (TempId(id), Location::Stack(Slot { offset, width: 2 })))
                .collect(),
            edge_copies: vec![],
        };
        f.verify_stack(&r).unwrap();
        (r, f)
    }
    #[test]
    fn simultaneous_sources_move_but_anchors_and_frame_accounting_remain() {
        let (r, mut f) = example();
        let original = f.clone();
        let mut alone = f.clone();
        alone.temps.insert(TempId(1), f.temps[&TempId(3)]);
        assert!(alone.verify_stack(&r).is_err());
        f.coalesce_edges(&r, &super::super::liveness::interference(&r).unwrap())
            .unwrap();
        f.verify_stack(&r).unwrap();
        for (a, d) in [(0, 2), (1, 3)] {
            assert_eq!(f.temps[&TempId(a)], original.temps[&TempId(d)]);
            assert_eq!(f.temps[&TempId(d)], original.temps[&TempId(d)]);
        }
        assert_eq!((f.extent, f.spill_bytes, f.peak_below_entry), (8, 8, 8));
        let once = format!("{f:?}");
        f.coalesce_edges(&r, &super::super::liveness::interference(&r).unwrap())
            .unwrap();
        assert_eq!(format!("{f:?}"), once);
    }
    #[test]
    fn anchored_sources_and_conflicting_repeated_sources_roll_back_completely() {
        for repeated in [false, true] {
            let (mut r, mut f) = example();
            if repeated {
                let Mir65816Terminator::Goto(e) = &mut r.blocks[0].terminator else {
                    panic!()
                };
                e.args[1] = e.args[0].clone();
            } else {
                r.blocks[0].params.push((TempId(0), ByteSize::new(2)));
                r.blocks[0].ops.remove(0);
            }
            f.verify_stack(&r).unwrap();
            let before = format!("{f:?}");
            f.coalesce_edges(&r, &super::super::liveness::interference(&r).unwrap())
                .unwrap();
            assert_eq!(format!("{f:?}"), before);
        }
    }
    #[test]
    fn compatible_pairs_cannot_shrink_the_accounted_extent_in_this_slice() {
        let (r, mut f) = example();
        f.temps.insert(
            TempId(0),
            Location::Stack(Slot {
                offset: 8,
                width: 2,
            }),
        );
        f.extent = 10;
        f.spill_bytes = 10;
        f.peak_below_entry = 10;
        f.verify_stack(&r).unwrap();
        let before = format!("{f:?}");
        f.coalesce_edges(&r, &super::super::liveness::interference(&r).unwrap())
            .unwrap();
        assert_eq!(format!("{f:?}"), before);
    }
    #[test]
    fn a_profitable_arm_cannot_regress_the_other_arm() {
        let (mut r, mut f) = example();
        let ty = r.temps[0].1.clone();
        let w = ByteSize::new(2);
        r.temps.extend([(TempId(4), ty.clone()), (TempId(5), ty)]);
        let Mir65816Terminator::Goto(first) = r.blocks[0].terminator.clone() else {
            panic!()
        };
        let mut second = first.clone();
        second.target = BlockId(2);
        r.blocks[0].terminator = Mir65816Terminator::Branch {
            condition: Mir65816Value::U8(1),
            then_edge: first,
            else_edge: second,
        };
        let mut ret = r.blocks[1].terminator.clone();
        let Mir65816Terminator::Return { value, .. } = &mut ret else {
            panic!()
        };
        *value = Some(Mir65816Value::Temp(TempId(4), w));
        r.blocks.push(Mir65816Block {
            id: BlockId(2),
            params: vec![(TempId(4), w), (TempId(5), w)],
            ops: vec![],
            terminator: ret,
        });
        f.temps.insert(TempId(4), f.temps[&TempId(0)]);
        f.temps.insert(TempId(5), f.temps[&TempId(1)]);
        f.verify_stack(&r).unwrap();
        let before = format!("{f:?}");
        let mut legal = f.clone();
        legal.temps.insert(TempId(0), f.temps[&TempId(2)]);
        legal.temps.insert(TempId(1), f.temps[&TempId(3)]);
        legal.verify_stack(&r).unwrap();
        f.coalesce_edges(&r, &super::super::liveness::interference(&r).unwrap())
            .unwrap();
        assert_eq!(format!("{f:?}"), before);
    }
}
