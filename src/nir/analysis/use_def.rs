use std::collections::BTreeMap;

use crate::nir::{
    BlockId, NirCallee, NirEdge, NirOp, NirPlace, NirPlaceKind, NirRoutine, NirTerminator,
    NirValue, TempId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::nir) struct NirDefSite {
    pub(in crate::nir) block: BlockId,
    pub(in crate::nir) op_index: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::nir) enum NirUseKind {
    LoadPlace,
    AddressPlace,
    StorePlace,
    StoreSource,
    UnarySource,
    CastSource,
    BinaryLeft,
    BinaryRight,
    CompareLeft,
    CompareRight,
    IndirectCallee,
    CallArgument(usize),
    BranchCondition,
    EdgeArgument { target: BlockId, index: usize },
    ReturnValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::nir) enum NirUseSite {
    Op {
        block: BlockId,
        op_index: usize,
        kind: NirUseKind,
    },
    Terminator {
        block: BlockId,
        kind: NirUseKind,
    },
}

impl NirUseSite {
    pub(in crate::nir) fn block(self) -> BlockId {
        match self {
            Self::Op { block, .. } | Self::Terminator { block, .. } => block,
        }
    }

    pub(in crate::nir) fn op_index(self) -> Option<usize> {
        match self {
            Self::Op { op_index, .. } => Some(op_index),
            Self::Terminator { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::nir) struct NirUseDef {
    definitions: BTreeMap<TempId, Vec<NirDefSite>>,
    uses: BTreeMap<TempId, Vec<NirUseSite>>,
}

impl NirUseDef {
    pub(in crate::nir) fn from_routine(routine: &NirRoutine) -> Self {
        let mut facts = Self {
            definitions: BTreeMap::new(),
            uses: BTreeMap::new(),
        };
        for block in &routine.blocks {
            for param in &block.params {
                facts
                    .definitions
                    .entry(param.dest)
                    .or_default()
                    .push(NirDefSite {
                        block: block.id,
                        op_index: None,
                    });
            }
            for (op_index, op) in block.ops.iter().enumerate() {
                if let Some(temp) = op_definition(op) {
                    facts.definitions.entry(temp).or_default().push(NirDefSite {
                        block: block.id,
                        op_index: Some(op_index),
                    });
                }
                record_op_uses(&mut facts.uses, block.id, op_index, op);
            }
            record_terminator_uses(&mut facts.uses, block.id, &block.terminator);
        }
        facts
    }

    pub(in crate::nir) fn definitions(&self, temp: TempId) -> &[NirDefSite] {
        self.definitions.get(&temp).map_or(&[], Vec::as_slice)
    }

    pub(in crate::nir) fn unique_definition(&self, temp: TempId) -> Option<NirDefSite> {
        let [definition] = self.definitions(temp) else {
            return None;
        };
        Some(*definition)
    }

    pub(in crate::nir) fn uses(&self, temp: TempId) -> &[NirUseSite] {
        self.uses.get(&temp).map_or(&[], Vec::as_slice)
    }

    #[allow(dead_code)] // Used by routine-wide propagation and DCE slices.
    pub(in crate::nir) fn use_count(&self, temp: TempId) -> usize {
        self.uses(temp).len()
    }

    pub(in crate::nir) fn has_use_at(
        &self,
        temp: TempId,
        block: BlockId,
        op_index: Option<usize>,
    ) -> bool {
        self.uses(temp)
            .iter()
            .any(|site| site.block() == block && site.op_index() == op_index)
    }

    #[allow(dead_code)] // Used by liveness and routine-wide optimizer slices.
    pub(in crate::nir) fn all_definitions(&self) -> &BTreeMap<TempId, Vec<NirDefSite>> {
        &self.definitions
    }

    #[allow(dead_code)] // Used by liveness and routine-wide optimizer slices.
    pub(in crate::nir) fn all_uses(&self) -> &BTreeMap<TempId, Vec<NirUseSite>> {
        &self.uses
    }
}

fn op_definition(op: &NirOp) -> Option<TempId> {
    match op {
        NirOp::Load { dest, .. }
        | NirOp::AddrOf { dest, .. }
        | NirOp::Unary { dest, .. }
        | NirOp::Cast { dest, .. }
        | NirOp::Binary { dest, .. }
        | NirOp::Compare { dest, .. } => Some(*dest),
        NirOp::Call {
            result: Some(result),
            ..
        } => Some(result.dest),
        NirOp::Define { .. }
        | NirOp::Set { .. }
        | NirOp::Declare { .. }
        | NirOp::Assign { .. }
        | NirOp::CompoundAssign { .. }
        | NirOp::Store { .. }
        | NirOp::Call { result: None, .. }
        | NirOp::MachineBlock { .. }
        | NirOp::Unsupported { .. }
        | NirOp::Note { .. } => None,
    }
}

fn record_op_uses(
    uses: &mut BTreeMap<TempId, Vec<NirUseSite>>,
    block: BlockId,
    op_index: usize,
    op: &NirOp,
) {
    let site = |kind| NirUseSite::Op {
        block,
        op_index,
        kind,
    };
    match op {
        NirOp::Load { place, .. } => record_place(uses, place, site(NirUseKind::LoadPlace)),
        NirOp::AddrOf { place, .. } => {
            record_place(uses, place, site(NirUseKind::AddressPlace));
        }
        NirOp::Store { place, src, .. } => {
            record_place(uses, place, site(NirUseKind::StorePlace));
            record_value(uses, src, site(NirUseKind::StoreSource));
        }
        NirOp::Unary { src, .. } => record_value(uses, src, site(NirUseKind::UnarySource)),
        NirOp::Cast { src, .. } => record_value(uses, src, site(NirUseKind::CastSource)),
        NirOp::Binary { left, right, .. } => {
            record_value(uses, left, site(NirUseKind::BinaryLeft));
            record_value(uses, right, site(NirUseKind::BinaryRight));
        }
        NirOp::Compare { left, right, .. } => {
            record_value(uses, left, site(NirUseKind::CompareLeft));
            record_value(uses, right, site(NirUseKind::CompareRight));
        }
        NirOp::Call {
            callee,
            args,
            result: _,
            signature: _,
            effects: _,
        } => {
            if let NirCallee::Indirect { target, .. } = callee {
                record_value(uses, target, site(NirUseKind::IndirectCallee));
            }
            for (argument, value) in args.iter().enumerate() {
                record_value(uses, value, site(NirUseKind::CallArgument(argument)));
            }
        }
        NirOp::Define { .. }
        | NirOp::Set { .. }
        | NirOp::Declare { .. }
        | NirOp::Assign { .. }
        | NirOp::CompoundAssign { .. }
        | NirOp::MachineBlock { .. }
        | NirOp::Unsupported { .. }
        | NirOp::Note { .. } => {}
    }
}

fn record_terminator_uses(
    uses: &mut BTreeMap<TempId, Vec<NirUseSite>>,
    block: BlockId,
    terminator: &NirTerminator,
) {
    match terminator {
        NirTerminator::Goto(edge) => record_edge_uses(uses, block, edge),
        NirTerminator::Branch {
            condition,
            then_edge,
            else_edge,
        } => {
            record_value(
                uses,
                condition,
                NirUseSite::Terminator {
                    block,
                    kind: NirUseKind::BranchCondition,
                },
            );
            record_edge_uses(uses, block, then_edge);
            record_edge_uses(uses, block, else_edge);
        }
        NirTerminator::Return(Some(value)) => record_value(
            uses,
            value,
            NirUseSite::Terminator {
                block,
                kind: NirUseKind::ReturnValue,
            },
        ),
        NirTerminator::Open
        | NirTerminator::Fallthrough
        | NirTerminator::Return(None)
        | NirTerminator::Exit
        | NirTerminator::Unknown(_) => {}
    }
}

fn record_edge_uses(uses: &mut BTreeMap<TempId, Vec<NirUseSite>>, block: BlockId, edge: &NirEdge) {
    for (index, value) in edge.args.iter().enumerate() {
        record_value(
            uses,
            value,
            NirUseSite::Terminator {
                block,
                kind: NirUseKind::EdgeArgument {
                    target: edge.target,
                    index,
                },
            },
        );
    }
}

fn record_place(uses: &mut BTreeMap<TempId, Vec<NirUseSite>>, place: &NirPlace, site: NirUseSite) {
    match &place.kind {
        NirPlaceKind::Deref { addr } => record_value(uses, addr, site),
        NirPlaceKind::Index {
            base_addr, index, ..
        } => {
            record_value(uses, base_addr, site);
            record_value(uses, index, site);
        }
        NirPlaceKind::Field { base, .. } => record_place(uses, base, site),
        NirPlaceKind::Symbol(_)
        | NirPlaceKind::Param { .. }
        | NirPlaceKind::Local { .. }
        | NirPlaceKind::Global { .. }
        | NirPlaceKind::Absolute(_)
        | NirPlaceKind::UnresolvedName(_) => {}
    }
}

fn record_value(uses: &mut BTreeMap<TempId, Vec<NirUseSite>>, value: &NirValue, site: NirUseSite) {
    if let NirValue::Temp { id, .. } = value {
        uses.entry(*id).or_default().push(site);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nir::{
        NirBinaryOp, NirBlock, NirBlockParam, NirCallEffects, NirCallResult, NirCompareOp,
        NirMemoryAccess, NirMemoryEffects, NirPlace, NirPlaceKind, NirType, NirTypeKind,
        NirUnaryOp,
    };

    fn byte_type() -> NirType {
        NirType {
            kind: NirTypeKind::U8,
            summary: "BYTE".to_string(),
            width: Some(1),
            pointer: false,
        }
    }

    fn temp(id: u32) -> NirValue {
        NirValue::Temp {
            id: TempId(id),
            ty: byte_type(),
        }
    }

    #[test]
    fn indexes_definitions_and_distinct_operand_positions() {
        let routine = NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "entry".to_string(),
                params: Vec::new(),
                ops: vec![
                    NirOp::Binary {
                        dest: TempId(1),
                        ty: byte_type(),
                        op: NirBinaryOp::Add,
                        left: temp(10),
                        right: temp(11),
                    },
                    NirOp::Compare {
                        dest: TempId(2),
                        ty: byte_type(),
                        op: NirCompareOp::Eq,
                        left: temp(1),
                        right: temp(12),
                    },
                    NirOp::Call {
                        callee: NirCallee::Indirect {
                            target: temp(13),
                            ty: byte_type(),
                        },
                        args: vec![temp(1), temp(2)],
                        result: Some(NirCallResult {
                            dest: TempId(3),
                            ty: byte_type(),
                        }),
                        signature: None,
                        effects: NirCallEffects {
                            memory: NirMemoryEffects {
                                reads: NirMemoryAccess::None,
                                writes: NirMemoryAccess::None,
                            },
                            may_call_os: false,
                            opaque: false,
                        },
                    },
                ],
                terminator: NirTerminator::Return(Some(temp(3))),
            }],
        };

        let use_def = NirUseDef::from_routine(&routine);

        assert_eq!(
            use_def.unique_definition(TempId(1)),
            Some(NirDefSite {
                block: BlockId(0),
                op_index: Some(0)
            })
        );
        assert_eq!(
            use_def.unique_definition(TempId(3)).unwrap().op_index,
            Some(2)
        );
        assert_eq!(
            use_def.uses(TempId(1)),
            &[
                NirUseSite::Op {
                    block: BlockId(0),
                    op_index: 1,
                    kind: NirUseKind::CompareLeft,
                },
                NirUseSite::Op {
                    block: BlockId(0),
                    op_index: 2,
                    kind: NirUseKind::CallArgument(0),
                },
            ]
        );
        assert_eq!(
            use_def.uses(TempId(3)),
            &[NirUseSite::Terminator {
                block: BlockId(0),
                kind: NirUseKind::ReturnValue,
            }]
        );
        assert!(matches!(
            use_def.uses(TempId(13)),
            [NirUseSite::Op {
                kind: NirUseKind::IndirectCallee,
                ..
            }]
        ));
    }

    #[test]
    fn indexes_place_scalar_and_branch_uses() {
        let indexed_place = NirPlace {
            kind: NirPlaceKind::Index {
                base_addr: temp(20),
                index: temp(21),
                elem_ty: byte_type(),
                elem_size: 1,
            },
            ty: Some(byte_type()),
        };
        let deref_place = |id| NirPlace {
            kind: NirPlaceKind::Deref { addr: temp(id) },
            ty: Some(byte_type()),
        };
        let routine = NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![NirBlock {
                id: BlockId(0),
                label: "entry".to_string(),
                params: Vec::new(),
                ops: vec![
                    NirOp::Load {
                        dest: TempId(1),
                        ty: byte_type(),
                        place: indexed_place,
                    },
                    NirOp::AddrOf {
                        dest: TempId(2),
                        ty: byte_type(),
                        place: deref_place(22),
                    },
                    NirOp::Store {
                        place: deref_place(23),
                        src: temp(24),
                        ty: byte_type(),
                    },
                    NirOp::Unary {
                        dest: TempId(3),
                        ty: byte_type(),
                        op: NirUnaryOp::Neg,
                        src: temp(25),
                    },
                    NirOp::Cast {
                        dest: TempId(4),
                        src: temp(26),
                        from: byte_type(),
                        to: byte_type(),
                    },
                ],
                terminator: NirTerminator::Branch {
                    condition: temp(27),
                    then_edge: NirEdge {
                        target: BlockId(0),
                        args: Vec::new(),
                    },
                    else_edge: NirEdge {
                        target: BlockId(0),
                        args: Vec::new(),
                    },
                },
            }],
        };

        let use_def = NirUseDef::from_routine(&routine);

        for (temp, kind) in [
            (20, NirUseKind::LoadPlace),
            (21, NirUseKind::LoadPlace),
            (22, NirUseKind::AddressPlace),
            (23, NirUseKind::StorePlace),
            (24, NirUseKind::StoreSource),
            (25, NirUseKind::UnarySource),
            (26, NirUseKind::CastSource),
        ] {
            assert!(matches!(
                use_def.uses(TempId(temp)),
                [NirUseSite::Op {
                    kind: actual,
                    ..
                }] if *actual == kind
            ));
        }
        assert!(matches!(
            use_def.uses(TempId(27)),
            [NirUseSite::Terminator {
                kind: NirUseKind::BranchCondition,
                ..
            }]
        ));
        for temp in 1..=4 {
            assert!(use_def.unique_definition(TempId(temp)).is_some());
        }
    }

    #[test]
    fn indexes_edge_arguments_as_uses_and_block_parameters_as_entry_definitions() {
        let routine = NirRoutine {
            name: "Main".to_string(),
            params: Vec::new(),
            locals: Vec::new(),
            temps: Vec::new(),
            notes: Vec::new(),
            blocks: vec![
                NirBlock {
                    id: BlockId(0),
                    label: "define".to_string(),
                    params: Vec::new(),
                    ops: vec![NirOp::Binary {
                        dest: TempId(0),
                        ty: byte_type(),
                        op: NirBinaryOp::Add,
                        left: NirValue::ConstU8(1),
                        right: NirValue::ConstU8(2),
                    }],
                    terminator: NirTerminator::Goto(NirEdge {
                        target: BlockId(1),
                        args: Vec::new(),
                    }),
                },
                NirBlock {
                    id: BlockId(1),
                    label: "pass".to_string(),
                    params: Vec::new(),
                    ops: Vec::new(),
                    terminator: NirTerminator::Goto(NirEdge {
                        target: BlockId(2),
                        args: vec![temp(0)],
                    }),
                },
                NirBlock {
                    id: BlockId(2),
                    label: "join".to_string(),
                    params: vec![NirBlockParam {
                        dest: TempId(1),
                        ty: byte_type(),
                    }],
                    ops: Vec::new(),
                    terminator: NirTerminator::Return(Some(temp(1))),
                },
            ],
        };

        let use_def = NirUseDef::from_routine(&routine);

        assert_eq!(
            use_def.unique_definition(TempId(1)),
            Some(NirDefSite {
                block: BlockId(2),
                op_index: None,
            })
        );
        assert_eq!(
            use_def.uses(TempId(0)),
            &[NirUseSite::Terminator {
                block: BlockId(1),
                kind: NirUseKind::EdgeArgument {
                    target: BlockId(2),
                    index: 0,
                },
            }]
        );
        assert_eq!(
            use_def.uses(TempId(1)),
            &[NirUseSite::Terminator {
                block: BlockId(2),
                kind: NirUseKind::ReturnValue,
            }]
        );
    }
}
