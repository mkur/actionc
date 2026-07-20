#![allow(dead_code)] // Generation-scoped rewrite plans arrive in later workflow slices.

use crate::mir6502::analysis::cfg::{MirCfg, MirCfgError};
use crate::mir6502::ir::{MirBlockId, MirRoutine};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub(in crate::mir6502) struct MirRoutineGeneration(u64);

impl MirRoutineGeneration {
    pub(in crate::mir6502) fn initial() -> Self {
        Self(0)
    }

    pub(in crate::mir6502) fn next(self) -> Self {
        Self(
            self.0
                .checked_add(1)
                .expect("MIR routine generation overflow"),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::mir6502) enum MirSite {
    BlockEntry { block: MirBlockId },
    Op { block: MirBlockId, op_index: usize },
    Terminator { block: MirBlockId },
}

impl MirSite {
    pub(in crate::mir6502) fn block(self) -> MirBlockId {
        match self {
            Self::BlockEntry { block } | Self::Op { block, .. } | Self::Terminator { block } => {
                block
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::mir6502) struct MirProgramPoint {
    pub generation: MirRoutineGeneration,
    pub site: MirSite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::mir6502) enum MirProgramPointError {
    StaleGeneration {
        expected: MirRoutineGeneration,
        actual: MirRoutineGeneration,
    },
    UnknownBlock(MirBlockId),
    OpOutOfBounds {
        block: MirBlockId,
        op_index: usize,
        op_count: usize,
    },
}

/// Generation-scoped routine and CFG view used to validate stable sites.
#[derive(Debug)]
pub(in crate::mir6502) struct MirRoutineSnapshot<'a> {
    routine: &'a MirRoutine,
    generation: MirRoutineGeneration,
    cfg: MirCfg,
}

impl<'a> MirRoutineSnapshot<'a> {
    pub(in crate::mir6502) fn new(
        routine: &'a MirRoutine,
        generation: MirRoutineGeneration,
    ) -> Result<Self, Vec<MirCfgError>> {
        Ok(Self {
            routine,
            generation,
            cfg: MirCfg::from_routine(routine)?,
        })
    }

    pub(in crate::mir6502) fn generation(&self) -> MirRoutineGeneration {
        self.generation
    }

    pub(in crate::mir6502) fn cfg(&self) -> &MirCfg {
        &self.cfg
    }

    pub(in crate::mir6502) fn point(&self, site: MirSite) -> MirProgramPoint {
        MirProgramPoint {
            generation: self.generation,
            site,
        }
    }

    pub(in crate::mir6502) fn validate_point(
        &self,
        point: MirProgramPoint,
    ) -> Result<(), MirProgramPointError> {
        if point.generation != self.generation {
            return Err(MirProgramPointError::StaleGeneration {
                expected: self.generation,
                actual: point.generation,
            });
        }

        let block = point.site.block();
        let Some(block_index) = self.cfg.block_index(block) else {
            return Err(MirProgramPointError::UnknownBlock(block));
        };
        if let MirSite::Op { op_index, .. } = point.site {
            let op_count = self.routine.blocks[block_index].ops.len();
            if op_index >= op_count {
                return Err(MirProgramPointError::OpOutOfBounds {
                    block,
                    op_index,
                    op_count,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{
        MirBlock, MirDef, MirEffects, MirFrame, MirOp, MirReg, MirRoutineAbi, MirTerminator,
        MirWidth, RoutineId,
    };

    fn routine() -> MirRoutine {
        MirRoutine {
            id: RoutineId(0),
            name: "Main".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: Vec::new(),
            blocks: vec![MirBlock {
                id: MirBlockId(7),
                label: "entry".to_string(),
                params: Vec::new(),
                ops: vec![MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::A),
                    value: 7,
                    width: MirWidth::Byte,
                }],
                terminator: MirTerminator::Return,
            }],
            effects: MirEffects::default(),
        }
    }

    #[test]
    fn validates_operation_and_terminator_sites_in_current_generation() {
        let routine = routine();
        let snapshot = MirRoutineSnapshot::new(&routine, MirRoutineGeneration::initial()).unwrap();
        let op = snapshot.point(MirSite::Op {
            block: MirBlockId(7),
            op_index: 0,
        });
        let terminator = snapshot.point(MirSite::Terminator {
            block: MirBlockId(7),
        });

        assert_eq!(snapshot.generation(), MirRoutineGeneration::initial());
        assert_eq!(snapshot.cfg().entry(), Some(MirBlockId(7)));
        assert_eq!(snapshot.validate_point(op), Ok(()));
        assert_eq!(snapshot.validate_point(terminator), Ok(()));
    }

    #[test]
    fn rejects_stale_generation_before_reusing_an_operation_index() {
        let routine = routine();
        let old = MirRoutineSnapshot::new(&routine, MirRoutineGeneration::initial()).unwrap();
        let point = old.point(MirSite::Op {
            block: MirBlockId(7),
            op_index: 0,
        });
        let current =
            MirRoutineSnapshot::new(&routine, MirRoutineGeneration::initial().next()).unwrap();

        assert!(matches!(
            current.validate_point(point),
            Err(MirProgramPointError::StaleGeneration { .. })
        ));
    }

    #[test]
    fn rejects_unknown_blocks_and_out_of_bounds_operation_indices() {
        let routine = routine();
        let snapshot = MirRoutineSnapshot::new(&routine, MirRoutineGeneration::initial()).unwrap();

        let unknown = snapshot.point(MirSite::Terminator {
            block: MirBlockId(99),
        });
        let out_of_bounds = snapshot.point(MirSite::Op {
            block: MirBlockId(7),
            op_index: 1,
        });
        assert_eq!(
            snapshot.validate_point(unknown),
            Err(MirProgramPointError::UnknownBlock(MirBlockId(99)))
        );
        assert_eq!(
            snapshot.validate_point(out_of_bounds),
            Err(MirProgramPointError::OpOutOfBounds {
                block: MirBlockId(7),
                op_index: 1,
                op_count: 1,
            })
        );
    }

    #[test]
    fn generation_advance_is_monotone() {
        let first = MirRoutineGeneration::initial();
        let second = first.next();
        assert!(second > first);
    }
}
