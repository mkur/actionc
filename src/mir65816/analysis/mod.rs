//! Immutable logical MIR facts, before resource selection or physical placement.
//! No query establishes pointee disjointness or permission to reorder memory.
mod cfg;
pub(crate) mod operands;
mod storage;
#[cfg(test)]
mod tests;

use crate::analysis::{
    dataflow::{DataflowDirection, DataflowProblem, DataflowResult, solve_dataflow},
    dominance::Dominance,
    graph::DataflowGraph,
};
use crate::mir65816::*;
use crate::nir::NirTypeKind;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
pub use storage::{
    ByteContents, Ownership, StorageContents, StorageFacts, StorageId, StorageVersion,
};

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

/// Only the owning analysis can create a handle. Stable IDs remain readable,
/// but do not alone identify a proof or an immutable input generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Handle<T> {
    generation: u64,
    routine: RoutineId,
    key: T,
}
impl<T: Copy> Handle<T> {
    pub fn id(self) -> T {
        self.key
    }
}
pub type Block = Handle<BlockId>;
pub type Temp = Handle<TempId>;
pub type Point = Handle<ProgramPoint>;
pub type Edge = Handle<EdgeId>;
pub type Storage = Handle<StorageId>;

/// Before operation `index`, or before the terminator when index == ops.len().
/// Block parameters are already defined at index zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProgramPoint {
    pub block: BlockId,
    pub index: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EdgeId {
    pub source: BlockId,
    pub ordinal: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Definition {
    BlockParameter { block: BlockId, ordinal: usize },
    Operation(ProgramPoint),
}
impl Definition {
    fn block(self) -> BlockId {
        match self {
            Self::BlockParameter { block, .. } => block,
            Self::Operation(p) => p.block,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UseSite {
    pub point: ProgramPoint,
    pub ordinal: usize,
}
/// Target value representation, without source/display names or pointee
/// ownership claims. Width is retained separately in `ValueFacts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Representation {
    Boolean,
    Integer(crate::nir::NirIntegerType),
    DataPointer(crate::target::AddressSpaceId),
    CodePointer {
        address_space: crate::target::AddressSpaceId,
        signature: SignatureId,
        kind: crate::nir::NirCallableKind,
        convention: NirCallConvention,
    },
    Record {
        definition: Option<crate::semantic::SymbolId>,
        size: Option<ByteSize>,
    },
    Real,
    Unknown,
}
impl Representation {
    fn from_kind(kind: &NirTypeKind) -> Self {
        match kind {
            NirTypeKind::Bool => Self::Boolean,
            NirTypeKind::Integer(i) => Self::Integer(*i),
            NirTypeKind::Pointer { address_space, .. } => Self::DataPointer(*address_space),
            NirTypeKind::Callable {
                kind,
                signature,
                convention,
                address_space,
            } => Self::CodePointer {
                address_space: *address_space,
                signature: *signature,
                kind: *kind,
                convention: *convention,
            },
            NirTypeKind::Record {
                definition, size, ..
            } => Self::Record {
                definition: *definition,
                size: *size,
            },
            NirTypeKind::Real => Self::Real,
            NirTypeKind::Error | NirTypeKind::Void => Self::Unknown,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueFacts {
    pub width: ByteSize,
    /// Structured type only; printed type summaries never establish equality.
    pub representation: Representation,
    pub definition: Definition,
    pub uses: Vec<UseSite>,
    /// Equal only for complete, representation-preserving captured copies.
    pub identity: Temp,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeMapping {
    pub target: Block,
    /// Simultaneous incoming assignments, preserving this logical edge's ordinal.
    pub bindings: Vec<(Temp, Mir65816Value)>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryError {
    ForeignRoutine,
    StaleGeneration,
    UnknownBlock,
    UnknownTemp,
    UnknownStorage,
    UnknownEdge,
    InvalidPoint,
    InvalidRange,
    Unreachable,
}

/// Facts borrow one immutable logical routine, including its ownership plans.
/// Rebuilding always creates a new generation, even for an identical clone.
/// Edits cannot occur while this borrow or a query result borrowing it is live.
///
/// ```compile_fail
/// use actionc::mir65816::{Mir65816Routine, analysis::RoutineAnalysis};
/// fn edit(r: &mut Mir65816Routine) {
///     let facts = RoutineAnalysis::new(r).unwrap();
///     r.blocks.clear();
///     let _ = facts.block(actionc::nir::BlockId(0));
/// }
/// ```
#[derive(Debug)]
pub struct RoutineAnalysis<'a> {
    routine: &'a Mir65816Routine,
    generation: u64,
    cfg: cfg::Cfg,
    dominance: Dominance<BlockId>,
    values: BTreeMap<TempId, ValueFacts>,
    edges: BTreeMap<EdgeId, EdgeMapping>,
    liveness: DataflowResult<BlockId, BTreeSet<TempId>>,
    storage: storage::StorageAnalysis,
}

impl<'a> RoutineAnalysis<'a> {
    pub fn new(routine: &'a Mir65816Routine) -> Result<Self, String> {
        if routine.helper.is_some() || routine.entry.external {
            return Err("opaque routines have no logical body to analyze".into());
        }
        let generation = NEXT_GENERATION
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| "logical analysis generation exhausted")?;
        let handle = |key| Handle {
            generation,
            routine: routine.id,
            key,
        };
        let cfg = cfg::Cfg::build(routine)?;
        let dominance = Dominance::from_graph(&cfg);
        let types: BTreeMap<_, _> = routine.temps.iter().map(|(id, ty)| (*id, ty)).collect();
        if types.len() != routine.temps.len() {
            return Err("duplicate temporary identity".into());
        }
        let mut definitions = BTreeMap::new();
        let mut define = |id, width, site| -> Result<(), String> {
            let ty = types.get(&id).ok_or("definition of unknown temporary")?;
            if width == ByteSize::ZERO
                || ty.width != Some(width)
                || definitions.insert(id, site).is_some()
            {
                return Err("duplicate or incorrectly sized temporary definition".into());
            }
            Ok(())
        };
        for block in &routine.blocks {
            for (ordinal, &(id, width)) in block.params.iter().enumerate() {
                define(
                    id,
                    width,
                    Definition::BlockParameter {
                        block: block.id,
                        ordinal,
                    },
                )?;
            }
            for (index, op) in block.ops.iter().enumerate() {
                if let Some((id, width)) = operands::operation(op).definition {
                    define(
                        id,
                        width,
                        Definition::Operation(ProgramPoint {
                            block: block.id,
                            index,
                        }),
                    )?;
                }
            }
        }
        if definitions.len() != types.len() {
            return Err("missing temporary definition".into());
        }
        let mut values: BTreeMap<_, _> = types
            .iter()
            .map(|(&id, ty)| {
                (
                    id,
                    ValueFacts {
                        width: ty.width.unwrap(),
                        representation: Representation::from_kind(&ty.kind),
                        definition: definitions[&id],
                        uses: vec![],
                        identity: handle(id),
                    },
                )
            })
            .collect();
        let mut edges = BTreeMap::new();
        for block in &routine.blocks {
            for (index, census) in block
                .ops
                .iter()
                .map(operands::operation)
                .chain([operands::terminator(&block.terminator)])
                .enumerate()
            {
                let point = ProgramPoint {
                    block: block.id,
                    index,
                };
                for (ordinal, value) in census.inputs.into_iter().enumerate() {
                    checked_width(routine, &types, value)?;
                    if let Mir65816Value::Temp(id, _) = value {
                        let definition = definitions[id];
                        if cfg.reachable().contains(&block.id)
                            && !definition_available(&cfg, &dominance, definition, point)
                        {
                            return Err(format!(
                                "temporary t{} is not definitely defined at b{}:{index}",
                                id.0, block.id.0
                            ));
                        }
                        values
                            .get_mut(id)
                            .unwrap()
                            .uses
                            .push(UseSite { point, ordinal });
                    }
                }
            }
            for (ordinal, edge) in operands::edges(&block.terminator).into_iter().enumerate() {
                let target = &routine.blocks[cfg.indices[&edge.target]];
                if edge.args.len() != target.params.len() {
                    return Err("edge argument count mismatch".into());
                }
                let mut bindings = Vec::new();
                for (value, &(id, width)) in edge.args.iter().zip(&target.params) {
                    if checked_width(routine, &types, value)? != width {
                        return Err("edge argument width mismatch".into());
                    }
                    bindings.push((handle(id), value.clone()));
                }
                edges.insert(
                    EdgeId {
                        source: block.id,
                        ordinal,
                    },
                    EdgeMapping {
                        target: Handle {
                            generation,
                            routine: routine.id,
                            key: edge.target,
                        },
                        bindings,
                    },
                );
            }
            if matches!(block.terminator, Mir65816Terminator::Fallthrough) {
                edges.insert(
                    EdgeId {
                        source: block.id,
                        ordinal: 0,
                    },
                    EdgeMapping {
                        target: Handle {
                            generation,
                            routine: routine.id,
                            key: operands::successors(routine, cfg.indices[&block.id])?[0],
                        },
                        bindings: vec![],
                    },
                );
            }
        }
        // Identity casts may form chains independent of lexical block order.
        // Dominance verification prevents executable identity cycles.
        for _ in 0..values.len() {
            let mut changed = false;
            for block in &routine.blocks {
                if !cfg.reachable().contains(&block.id) {
                    continue;
                }
                for op in &block.ops {
                    if let Mir65816Op::Cast {
                        dest,
                        from,
                        to,
                        value: Mir65816Value::Temp(source, width),
                        ..
                    } = op
                    {
                        if from == to
                            && from == width
                            && values[source].width == *width
                            && values[source].representation != Representation::Unknown
                            && values[dest].representation == values[source].representation
                        {
                            let identity = values[source].identity;
                            let dest = values.get_mut(dest).unwrap();
                            changed |= dest.identity != identity;
                            dest.identity = identity;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let liveness = solve_dataflow(&cfg, &LiveProblem::new(routine));
        let storage = storage::StorageAnalysis::new(routine, &cfg)?;
        Ok(Self {
            routine,
            generation,
            cfg,
            dominance,
            values,
            edges,
            liveness,
            storage,
        })
    }

    fn handle<T>(&self, key: T) -> Handle<T> {
        Handle {
            generation: self.generation,
            routine: self.routine.id,
            key,
        }
    }
    fn check<T: Copy>(&self, handle: Handle<T>) -> Result<T, QueryError> {
        if handle.routine != self.routine.id {
            return Err(QueryError::ForeignRoutine);
        }
        if handle.generation != self.generation {
            return Err(QueryError::StaleGeneration);
        }
        Ok(handle.key)
    }
    fn reachable(&self, id: BlockId) -> Result<(), QueryError> {
        if self.cfg.reachable().contains(&id) {
            Ok(())
        } else {
            Err(QueryError::Unreachable)
        }
    }
    pub fn block(&self, id: BlockId) -> Result<Block, QueryError> {
        self.cfg
            .indices
            .contains_key(&id)
            .then(|| self.handle(id))
            .ok_or(QueryError::UnknownBlock)
    }
    pub fn temp(&self, id: TempId) -> Result<Temp, QueryError> {
        self.values
            .contains_key(&id)
            .then(|| self.handle(id))
            .ok_or(QueryError::UnknownTemp)
    }
    pub fn point(&self, block: Block, index: usize) -> Result<Point, QueryError> {
        let id = self.check(block)?;
        if index > self.routine.blocks[self.cfg.indices[&id]].ops.len() {
            return Err(QueryError::InvalidPoint);
        }
        Ok(self.handle(ProgramPoint { block: id, index }))
    }
    pub fn edge(&self, block: Block, ordinal: usize) -> Result<Edge, QueryError> {
        let key = EdgeId {
            source: self.check(block)?,
            ordinal,
        };
        self.edges
            .contains_key(&key)
            .then(|| self.handle(key))
            .ok_or(QueryError::UnknownEdge)
    }
    pub fn edge_mapping(&self, edge: Edge) -> Result<&EdgeMapping, QueryError> {
        let key = self.check(edge)?;
        self.reachable(key.source)?;
        Ok(&self.edges[&key])
    }
    /// Includes occurrences in unreachable blocks; this is a census, not a proof
    /// that their definitions are available on an executable path.
    pub fn value(&self, temp: Temp) -> Result<&ValueFacts, QueryError> {
        Ok(&self.values[&self.check(temp)?])
    }
    pub fn dominates(&self, a: Block, b: Block) -> Result<bool, QueryError> {
        let (a, b) = (self.check(a)?, self.check(b)?);
        self.reachable(a)?;
        self.reachable(b)?;
        Ok(self.dominance.dominates(a, b))
    }
    pub fn is_cyclic(&self, block: Block) -> Result<bool, QueryError> {
        let id = self.check(block)?;
        self.reachable(id)?;
        Ok(self.cfg.cyclic.contains(&id))
    }
    pub fn available(&self, point: Point, temp: Temp) -> Result<bool, QueryError> {
        let point = self.check(point)?;
        let temp = self.check(temp)?;
        self.reachable(point.block)?;
        Ok(definition_available(
            &self.cfg,
            &self.dominance,
            self.values[&temp].definition,
            point,
        ))
    }
    pub fn same_value(&self, point: Point, a: Temp, b: Temp) -> Result<bool, QueryError> {
        let a_available = self.available(point, a)?;
        let b_available = self.available(point, b)?;
        if !a_available || !b_available {
            return Ok(false);
        }
        let (a, b) = (self.value(a)?, self.value(b)?);
        Ok(a.representation != Representation::Unknown
            && b.representation != Representation::Unknown
            && a.identity == b.identity)
    }
    pub fn live_at(&self, point: Point) -> Result<Vec<Temp>, QueryError> {
        let point = self.check(point)?;
        self.reachable(point.block)?;
        let block = &self.routine.blocks[self.cfg.indices[&point.block]];
        let mut live = self.liveness.out_state(point.block).unwrap().clone();
        add_inputs(&mut live, operands::terminator(&block.terminator));
        for op in block.ops[point.index..].iter().rev() {
            let census = operands::operation(op);
            if let Some((id, _)) = census.definition {
                live.remove(&id);
            }
            add_inputs(&mut live, census);
        }
        Ok(live.into_iter().map(|id| self.handle(id)).collect())
    }
    pub fn storage(&self, id: StorageId) -> Result<Storage, QueryError> {
        self.storage
            .facts
            .contains_key(&id)
            .then(|| self.handle(id))
            .ok_or(QueryError::UnknownStorage)
    }
    pub fn parameter_storage(&self, param: ParamId) -> Result<Storage, QueryError> {
        self.storage
            .parameters
            .get(&param)
            .copied()
            .map(|id| self.handle(id))
            .ok_or(QueryError::UnknownStorage)
    }
    pub fn storage_facts(&self, storage: Storage) -> Result<&StorageFacts, QueryError> {
        Ok(&self.storage.facts[&self.check(storage)?])
    }
    pub fn storage_at(
        &self,
        point: Point,
        storage: Storage,
        offset: u32,
        bytes: u32,
    ) -> Result<StorageContents, QueryError> {
        let p = self.check(point)?;
        let s = self.check(storage)?;
        self.reachable(p.block)?;
        self.storage.contents(
            self.routine,
            &self.cfg,
            p,
            s,
            offset,
            bytes,
            |p| self.handle(p),
            |s| self.handle(s),
        )
    }
    pub fn solver_evaluations(&self) -> usize {
        self.liveness.evaluations() + self.storage.evaluations()
    }
    /// Exact bytes of the same logical home, within one invocation. Unknown
    /// versions, including cyclic writes, cannot satisfy this proof. This says
    /// nothing about memory reached through a pointer stored in that home.
    pub fn same_storage_version(
        &self,
        a: Point,
        b: Point,
        storage: Storage,
        offset: u32,
        bytes: u32,
    ) -> Result<bool, QueryError> {
        let left = self.storage_at(a, storage, offset, bytes)?;
        let right = self.storage_at(b, storage, offset, bytes)?;
        Ok(left
            .bytes
            .iter()
            .zip(&right.bytes)
            .all(|(a, b)| a.version.is_some() && a.version == b.version))
    }
}

fn checked_width(
    r: &Mir65816Routine,
    types: &BTreeMap<TempId, &crate::nir::NirType>,
    value: &Mir65816Value,
) -> Result<ByteSize, String> {
    let width = match value {
        Mir65816Value::Temp(id, w) => {
            if types.get(id).and_then(|ty| ty.width) != Some(*w) {
                return Err("unknown or incorrectly sized temporary use".into());
            }
            *w
        }
        Mir65816Value::U24(n) if *n > 0xffffff => return Err("out-of-range 24-bit constant".into()),
        _ => super::value_width(r, value).ok_or("unknown or unsized operand")?,
    };
    if width == ByteSize::ZERO {
        return Err("zero-width operand".into());
    }
    Ok(width)
}

fn definition_available(
    cfg: &cfg::Cfg,
    dom: &Dominance<BlockId>,
    def: Definition,
    p: ProgramPoint,
) -> bool {
    if !cfg.reachable().contains(&def.block()) {
        return false;
    }
    if def.block() != p.block {
        return dom.dominates(def.block(), p.block);
    }
    match def {
        Definition::BlockParameter { .. } => true,
        Definition::Operation(d) => d.index < p.index,
    }
}

fn add_inputs(set: &mut BTreeSet<TempId>, census: operands::Census<'_>) {
    set.extend(census.inputs.into_iter().filter_map(|v| {
        if let Mir65816Value::Temp(id, _) = v {
            Some(*id)
        } else {
            None
        }
    }));
}
struct LiveProblem {
    defs: BTreeMap<BlockId, BTreeSet<TempId>>,
    uses: BTreeMap<BlockId, BTreeSet<TempId>>,
}
impl LiveProblem {
    fn new(routine: &Mir65816Routine) -> Self {
        let mut defs = BTreeMap::new();
        let mut uses = BTreeMap::new();
        for b in &routine.blocks {
            let mut defined: BTreeSet<_> = b.params.iter().map(|(id, _)| *id).collect();
            let mut upward = BTreeSet::new();
            for c in b
                .ops
                .iter()
                .map(operands::operation)
                .chain([operands::terminator(&b.terminator)])
            {
                for v in &c.inputs {
                    if let Mir65816Value::Temp(id, _) = v {
                        if !defined.contains(id) {
                            upward.insert(*id);
                        }
                    }
                }
                defined.extend(c.definition.map(|(id, _)| id));
            }
            defs.insert(b.id, defined);
            uses.insert(b.id, upward);
        }
        Self { defs, uses }
    }
}
impl DataflowProblem<cfg::Cfg> for LiveProblem {
    type State = BTreeSet<TempId>;
    fn direction(&self) -> DataflowDirection {
        DataflowDirection::Backward
    }
    fn bottom(&self) -> Self::State {
        BTreeSet::new()
    }
    fn boundary(&self, _: BlockId) -> Option<Self::State> {
        None
    }
    fn join(&self, into: &mut Self::State, other: &Self::State) {
        into.extend(other);
    }
    fn transfer(&self, b: BlockId, out: &Self::State) -> Self::State {
        out.difference(&self.defs[&b])
            .copied()
            .chain(self.uses[&b].iter().copied())
            .collect()
    }
}
