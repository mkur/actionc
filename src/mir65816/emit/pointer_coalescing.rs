//! Bounded stack affinities for dying, bit-preserving three-byte casts.
use super::*;
use std::collections::BTreeSet;

impl AllocatedFrame {
    pub(super) fn coalesce_pointer_casts(
        &mut self,
        routine: &Mir65816Routine,
    ) -> Result<(), String> {
        let pairs: Vec<_> = routine
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .filter_map(liveness::pointer_copy)
            .collect();
        if pairs.is_empty() {
            return Ok(());
        }
        let graph = liveness::pointer_copy_interference(routine)?;
        // Keep edge destinations and sources fixed in this slice: parallel-copy
        // schedules, staging, and existing edge affinities cannot regress.
        let mut anchors: BTreeSet<_> = routine
            .blocks
            .iter()
            .flat_map(|b| b.params.iter().map(|p| p.0))
            .collect();
        for block in &routine.blocks {
            let edges = match &block.terminator {
                Mir65816Terminator::Goto(e) => vec![e],
                Mir65816Terminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } => vec![then_edge, else_edge],
                _ => vec![],
            };
            for edge in edges {
                for value in &edge.args {
                    if let Mir65816Value::Temp(id, _) = value {
                        anchors.insert(*id);
                    }
                }
            }
        }
        let identities = |frame: &Self| {
            pairs
                .iter()
                .filter(|(a, b)| frame.temps[a] == frame.temps[b])
                .count()
        };
        for &(source, dest) in &pairs {
            if graph[&source].contains(&dest) || self.temps[&source] == self.temps[&dest] {
                continue;
            }
            for (moving, fixed) in [(dest, source), (source, dest)] {
                if anchors.contains(&moving) {
                    continue;
                }
                let (Location::Stack(a), Location::Stack(b)) =
                    (self.temps[&moving], self.temps[&fixed])
                else {
                    continue;
                };
                if a.width != 3 || b.width != 3 {
                    continue;
                }
                let mut trial = self.clone();
                trial.temps.insert(moving, Location::Stack(b));
                // Full third-party liveness, exact whole-home overlap, retained
                // frame accounting and staging are rechecked before adoption.
                if identities(&trial) > identities(self) && trial.verify_stack(routine).is_ok() {
                    *self = trial;
                    break;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nir::BlockId;
    fn routine() -> Mir65816Routine {
        let source = "ADDRESS FUNC Work(ADDRESS p) RETURN(ADDRESS(BYTE POINTER(p)))";
        let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
        let model = crate::semantic::analyze_with_options(
            &ast,
            crate::semantic::SemanticOptions::modern()
                .with_target(crate::target::TargetId::Wdc65816Native),
        )
        .unwrap();
        let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
        crate::nir::verify_program(&nir).unwrap();
        crate::mir65816::lower_program(&nir)
            .unwrap()
            .routines
            .remove(0)
    }
    fn separate(r: &Mir65816Routine) -> AllocatedFrame {
        let start = (r.frame.extent.get() + 2) & !1;
        let temps = r
            .temps
            .iter()
            .enumerate()
            .map(|(i, (id, ty))| {
                assert_eq!(ty.width.unwrap().get(), 3);
                (
                    *id,
                    Location::Stack(Slot {
                        offset: (start + 4 * i as u32) as u16,
                        width: 3,
                    }),
                )
            })
            .collect();
        let extent =
            abi::stack::fixed_extent(ByteSize::new(start + 4 * (r.temps.len() as u32 - 1) + 2))
                .unwrap()
                .get() as u16;
        let f = AllocatedFrame {
            extent,
            spill_bytes: extent - r.frame.extent.get() as u16,
            peak_below_entry: extent,
            temps,
            edge_copies: vec![],
        };
        f.verify_stack(r).unwrap();
        f
    }
    fn pairs(r: &Mir65816Routine) -> Vec<(crate::nir::TempId, crate::nir::TempId)> {
        r.blocks
            .iter()
            .flat_map(|b| &b.ops)
            .filter_map(liveness::pointer_copy)
            .collect()
    }
    #[test]
    fn dying_casts_share_exact_homes_without_changing_frame_reservations() {
        let r = routine();
        let pairs = pairs(&r);
        assert!(pairs.len() >= 2);
        let mut f = separate(&r);
        let before = (f.extent, f.spill_bytes, f.peak_below_entry);
        for &(a, b) in &pairs {
            assert!(liveness::interference(&r).unwrap()[&a].contains(&b));
            assert!(!liveness::pointer_copy_interference(&r).unwrap()[&a].contains(&b));
        }
        f.coalesce_pointer_casts(&r).unwrap();
        f.verify_stack(&r).unwrap();
        assert!(pairs.iter().any(|(a, b)| f.temps[a] == f.temps[b]));
        assert_eq!(before, (f.extent, f.spill_bytes, f.peak_below_entry));
        let once = format!("{f:?}");
        f.coalesce_pointer_casts(&r).unwrap();
        assert_eq!(once, format!("{f:?}"));
    }
    #[test]
    fn later_and_backedge_uses_keep_closed_interference() {
        for backedge in [false, true] {
            let mut r = routine();
            let (a, b) = pairs(&r)[0];
            if backedge {
                let tail = r.blocks[0].ops.split_off(1);
                r.blocks[0].terminator = Mir65816Terminator::Goto(Mir65816Edge {
                    target: BlockId(99),
                    args: vec![],
                });
                r.blocks.push(Mir65816Block {
                    id: BlockId(99),
                    params: vec![],
                    ops: tail,
                    terminator: Mir65816Terminator::Goto(Mir65816Edge {
                        target: BlockId(99),
                        args: vec![],
                    }),
                });
            } else {
                let Mir65816Terminator::Return { value, .. } = &mut r.blocks[0].terminator else {
                    panic!()
                };
                *value = Some(Mir65816Value::Temp(a, ByteSize::new(3)));
            }
            assert!(liveness::pointer_copy_interference(&r).unwrap()[&a].contains(&b));
            let mut f = separate(&r);
            f.coalesce_pointer_casts(&r).unwrap();
            assert_ne!(f.temps[&a], f.temps[&b]);
            f.temps.insert(b, f.temps[&a]);
            assert!(f.verify_stack(&r).unwrap_err().contains("overlapping live"));
        }
    }
    #[test]
    fn partial_overlap_is_rejected_even_for_a_dying_identity_cast() {
        let r = routine();
        let (a, b) = pairs(&r)[0];
        let mut f = separate(&r);
        let offset = f.temps[&a].slot().offset + 2;
        f.temps
            .insert(b, Location::Stack(Slot { offset, width: 3 }));
        assert!(
            f.verify_stack(&r)
                .unwrap_err()
                .contains("partially overlapping")
        );
    }
    #[test]
    fn changing_widths_and_non_temp_inputs_never_gain_an_affinity() {
        let r = routine();
        let cast = r.blocks[0]
            .ops
            .iter()
            .find(|op| liveness::pointer_copy(op).is_some())
            .unwrap();
        for problem in 0..3 {
            let mut op = cast.clone();
            let Mir65816Op::Cast {
                from, to, value, ..
            } = &mut op
            else {
                panic!()
            };
            match problem {
                0 => *from = ByteSize::new(2),
                1 => *to = ByteSize::new(4),
                _ => *value = Mir65816Value::U24(0),
            }
            assert!(liveness::pointer_copy(&op).is_none());
        }
    }
}
