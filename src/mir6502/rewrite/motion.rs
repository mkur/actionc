//! Generation-checked transactions that relocate unchanged pre-home operations.
use super::context::PreHomeRewriteContext;
use super::driver::MirRewriteError;
use crate::mir6502::analysis::{
    cfg::MirCfg,
    loop_motion::{self, MirLoopMotionPlan},
    prehome::PreHomeAnalysisSnapshot,
    sites::MirRoutineGeneration,
};
use crate::mir6502::ir::MirRoutine;

#[derive(Default)]
pub(in crate::mir6502) struct MirPreHomeMotionDriver {
    generation: MirRoutineGeneration,
}
impl MirPreHomeMotionDriver {
    pub(in crate::mir6502) fn apply(
        &mut self,
        r: &mut MirRoutine,
        plan: &MirLoopMotionPlan,
    ) -> Result<bool, MirRewriteError> {
        if plan.generation != self.generation {
            return Err(MirRewriteError::StalePlan {
                expected: self.generation,
                actual: plan.generation,
            });
        }
        let snapshot = PreHomeAnalysisSnapshot::new(r, self.generation)
            .map_err(|_| MirRewriteError::InvalidCfg)?;
        let c = PreHomeRewriteContext::new(&snapshot);
        let Some(&root) = plan.operations.last() else {
            return Ok(false);
        };
        // Rebuild the full proof against the current generation before creating
        // a candidate. A failed declaration never performs a partial mutation.
        if loop_motion::prove(r, &c, plan.region.header, root).as_ref() != Some(plan) {
            return Ok(false);
        }
        let mut candidate = r.clone();
        let header = candidate
            .blocks
            .iter_mut()
            .find(|b| b.id == plan.region.header)
            .unwrap();
        let moving = plan
            .operations
            .iter()
            .map(|&i| header.ops[i].clone())
            .collect::<Vec<_>>();
        header.ops = header
            .ops
            .iter()
            .enumerate()
            .filter(|(i, _)| !plan.operations.contains(i))
            .map(|(_, op)| op.clone())
            .collect();
        candidate
            .blocks
            .iter_mut()
            .find(|b| b.id == plan.region.preheader)
            .unwrap()
            .ops
            .extend(moving);
        let cfg = MirCfg::from_routine(&candidate).map_err(|_| MirRewriteError::InvalidCfg)?;
        if &cfg != c.cfg() {
            return Ok(false);
        }
        *r = candidate;
        self.generation = self.generation.next();
        Ok(true)
    }

    pub(in crate::mir6502) fn run(&mut self, r: &mut MirRoutine) -> Result<usize, MirRewriteError> {
        let mut applied = 0;
        // Each unchanged operation moves to a strict dominating preheader.
        // Nested loops can move it again, at most once per dominator block.
        let budget = r
            .blocks
            .iter()
            .map(|b| b.ops.len())
            .sum::<usize>()
            .saturating_mul(r.blocks.len());
        for _ in 0..budget {
            if !loop_motion::might_match(r) {
                return Ok(applied);
            }
            let snapshot = PreHomeAnalysisSnapshot::new(r, self.generation)
                .map_err(|_| MirRewriteError::InvalidCfg)?;
            let c = PreHomeRewriteContext::new(&snapshot);
            let plan = loop_motion::discover_captured_bound_motion(r, &c)
                .into_iter()
                .next();
            let Some(plan) = plan else {
                return Ok(applied);
            };
            if !self.apply(r, &plan)? {
                return Ok(applied);
            }
            applied += 1;
        }
        Ok(applied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::analysis::loop_motion::tests::{captured, plans};
    #[test]
    fn motion_is_atomic_rechecks_declarations_and_rejects_stale_generations() {
        let mut p = captured();
        let r = &mut p.routines[0];
        let plan = plans(r).remove(0);
        let original = r.clone();
        let mut driver = MirPreHomeMotionDriver::default();
        let mut malformed = plan.clone();
        let first = *malformed.operations.first().unwrap();
        malformed.operations.remove(&first);
        assert!(!driver.apply(r, &malformed).unwrap());
        assert_eq!(*r, original);
        assert!(driver.apply(r, &plan).unwrap());
        let pre = r
            .blocks
            .iter()
            .find(|b| b.id == plan.region.preheader)
            .unwrap();
        let source = original
            .blocks
            .iter()
            .find(|b| b.id == plan.region.header)
            .unwrap();
        assert_eq!(
            &pre.ops[pre.ops.len() - plan.operations.len()..],
            plan.operations
                .iter()
                .map(|&i| source.ops[i].clone())
                .collect::<Vec<_>>()
        );
        assert!(matches!(
            driver.apply(r, &plan),
            Err(MirRewriteError::StalePlan { .. })
        ));
        let after = r.clone();
        assert_eq!(driver.run(r).unwrap(), 0);
        assert_eq!(*r, after);
        crate::mir6502::verify_program(&p, crate::mir6502::MirPhase::PreMaterialization).unwrap();
    }
}
