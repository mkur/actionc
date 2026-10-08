//! One owner for current native value placement. Selector admission remains
//! conservative; neither a missing artifact home nor a resource bound is a proof.
use super::{home_demand, resources, selected::*, state::Width, *};
use crate::mir65816::analysis::{Definition, EdgeId, ProgramPoint, RoutineAnalysis, UseSite};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InputOwner {
    Parameter(ParamId),
    Frame(Mir65816FrameObjectId),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BorrowedInput {
    pub temp: TempId,
    pub definition: ProgramPoint,
    pub uses: Vec<ProgramPoint>,
    pub source: InputOwner,
    pub home: Slot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Lanes {
    A8,
    A16,
    A16X8,
    A16X16,
}
impl Lanes {
    fn for_width(bytes: u8) -> Result<Self, String> {
        Ok(match bytes {
            1 => Self::A8,
            2 => Self::A16,
            3 => Self::A16X8,
            4 => Self::A16X16,
            _ => return Err("invalid placement register width".into()),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Capture {
    Home(Location),
    NativeOutput {
        interval: home_demand::Accumulator,
        lanes: Lanes,
    },
    Borrowed(BorrowedInput),
    Registers {
        interval: home_demand::Accumulator,
        lanes: Lanes,
    },
    /// Address components are evaluated at their sole store; no A/X value is
    /// promised at the otherwise omitted source operation.
    Components(home_demand::Accumulator),
    Local {
        definition: ProgramPoint,
        destination: Slot,
    },
    Assignment {
        definition: ProgramPoint,
        consumer: ProgramPoint,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReadLocation {
    Home(Location),
    Borrowed(Slot),
    Registers(Lanes),
    NativeOutput(Lanes),
    Components,
    Local(Slot),
    Assignment,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Value {
    pub bytes: u8,
    /// Canonical allocation stays truthful even when a read borrows another home.
    pub materialized: Option<Location>,
    pub residence: Option<mixed::Residence>,
    pub capture: Capture,
    pub reads: Vec<(UseSite, ReadLocation)>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Edge {
    target: BlockId,
    /// Simultaneous logical bindings; parallel edges retain their ordinals.
    bindings: Vec<(TempId, Mir65816Value)>,
    transfers: Vec<Transfer>,
    mixed: Option<mixed_copies::Plan>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Transfer {
    value: Mir65816Value,
    source: Option<Location>,
    destination: Location,
    bytes: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct WindowId(usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DescriptionId(usize);
#[derive(Clone, Debug, PartialEq, Eq)]
struct Window {
    description: DescriptionId,
    /// Closed-operation inputs/outputs are the only additional resident bytes
    /// allowed beside ordinary workspace. Empty ranges remain explicit rows.
    dp_operands: resources::Scratch,
    protected_dp: resources::Scratch,
    access_owner: WindowId,
}

/// Owns the admission results consumed by selection, instead of asking each
/// selector to decide independently whether an omitted home exists.
pub(super) struct Plan<'a> {
    routine: &'a Mir65816Routine,
    data: &'a [Mir65816Data],
    logical: RoutineAnalysis<'a>,
    pub demand: home_demand::Plan,
    pub frame: AllocatedFrame,
    pub calls: call_flow::Plan,
    pub pointers: select::pointer_forwarding::Plan,
    pub scalars: select::scalar_forwarding::Plan,
    pub loop_x: Option<loop_x::LoopXPlan>,
    pub components: BTreeMap<BlockId, select::address_consumers::Plan>,
    pub assignments: BTreeMap<BlockId, select::direct_assignments::Plan>,
    contract: Contract,
}

#[derive(Clone, Debug)]
pub(super) struct Contract {
    owner: Option<super::analysis::sites::Identity>,
    routine: RoutineId,
    frame: AllocatedFrame,
    pub values: BTreeMap<TempId, Value>,
    calls: call_flow::Plan,
    windows: BTreeMap<ProgramPoint, WindowId>,
    window_rows: Vec<Window>,
    descriptions: Vec<resources::Requirements>,
    edges: BTreeMap<EdgeId, Edge>,
    entries: BTreeMap<BlockId, BTreeMap<TempId, Location>>,
    regions: BTreeSet<TempId>,
    loops: BTreeSet<TempId>,
    segments: BTreeMap<TempId, Vec<mixed::Residence>>,
    aggregates: BTreeMap<ProgramPoint, (u32, bool)>,
    entry: BlockId,
    reachable: BTreeSet<BlockId>,
    labels: BTreeMap<BlockId, Label>,
    pointer_sources: BTreeMap<(ProgramPoint, tracked::PointerOrigin), Slot>,
    x: Option<tracked::XContract>,
}

impl<'a> Plan<'a> {
    pub fn new(r: &'a Mir65816Routine, data: &'a [Mir65816Data]) -> Result<Self, String> {
        // Descriptions exist before allocation. Logical facts borrow the same
        // immutable input throughout selection and cannot silently become stale.
        let resources = resources::Plan::new(r);
        let logical = RoutineAnalysis::new(r)?;
        let demand = home_demand::Plan::new(r);
        let frame = AllocatedFrame::with_demand(r, &demand)?;
        let calls = call_flow::plan(r, &logical)?;
        let pointers = demand.pointers.resolve(r, &frame)?;
        let scalars = select::scalar_forwarding::Plan::new(r, &frame)?;
        let loop_x = loop_x::LoopXPlan::new(r, &frame)?;
        let components = r
            .blocks
            .iter()
            .map(|b| {
                Ok((
                    b.id,
                    select::address_consumers::Plan::new(r, &frame, b, &demand, &pointers)?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        let counts = liveness::input_counts(r);
        let assignments = r
            .blocks
            .iter()
            .map(|b| {
                (
                    b.id,
                    select::direct_assignments::Plan::new(r, &frame, b, &counts, data),
                )
            })
            .collect();
        let contract = Self::describe(
            r,
            &logical,
            &demand,
            &frame,
            &pointers,
            &scalars,
            &components,
            &assignments,
            resources,
        )?;
        let this = Self {
            routine: r,
            data,
            logical,
            demand,
            frame,
            calls,
            pointers,
            scalars,
            loop_x,
            components,
            assignments,
            contract,
        };
        this.verify()?;
        Ok(this)
    }

    fn describe(
        r: &Mir65816Routine,
        logical: &RoutineAnalysis<'_>,
        demand: &home_demand::Plan,
        frame: &AllocatedFrame,
        pointers: &select::pointer_forwarding::Plan,
        scalars: &select::scalar_forwarding::Plan,
        components: &BTreeMap<BlockId, select::address_consumers::Plan>,
        assignments: &BTreeMap<BlockId, select::direct_assignments::Plan>,
        resources: resources::Plan,
    ) -> Result<Contract, String> {
        let mut borrowed = BTreeMap::new();
        for binding in pointers
            .placement_bindings()
            .into_iter()
            .chain(scalars.placement_bindings())
        {
            if borrowed.insert(binding.temp, binding).is_some() {
                return Err("conflicting placement input owners".into());
            }
        }
        let locals: BTreeMap<_, _> = demand
            .locals
            .placement_captures()
            .into_iter()
            .map(|(id, definition, destination)| (id, (definition, destination)))
            .collect();
        let assignment_values: BTreeMap<_, _> = assignments
            .iter()
            .flat_map(|(&block, a)| {
                a.placement_groups()
                    .into_iter()
                    .map(move |(temp, producer, consumer)| {
                        (
                            temp,
                            (
                                ProgramPoint {
                                    block,
                                    index: producer,
                                },
                                ProgramPoint {
                                    block,
                                    index: consumer,
                                },
                            ),
                        )
                    })
            })
            .collect();
        let mut access_owner: BTreeMap<_, _> = resources.windows.keys().map(|&p| (p, p)).collect();
        let mut values = BTreeMap::new();
        for (id, ty) in &r.temps {
            let bytes = allocation::width(ty.width.ok_or("unsized placement value")?)?;
            let materialized = frame.temps.get(id).copied();
            let capture = if let Some(interval) = demand.native_output(*id) {
                Capture::NativeOutput {
                    interval,
                    lanes: Lanes::for_width(bytes)?,
                }
            } else if let Some(b) = borrowed.remove(id) {
                Capture::Borrowed(b)
            } else if let Some(&(definition, destination)) = locals.get(id) {
                Capture::Local {
                    definition,
                    destination,
                }
            } else if let Some(interval) = demand.accumulator(*id) {
                if components[&interval.block].defers(interval.producer) {
                    Capture::Components(interval)
                } else {
                    Capture::Registers {
                        interval,
                        lanes: Lanes::for_width(bytes)?,
                    }
                }
            } else if let Some(&(definition, consumer)) = assignment_values.get(id) {
                access_owner.insert(consumer, definition);
                Capture::Assignment {
                    definition,
                    consumer,
                }
            } else {
                Capture::Home(materialized.ok_or("missing authoritative placement home")?)
            };
            let facts = logical
                .value(logical.temp(*id).map_err(|e| format!("{e:?}"))?)
                .map_err(|e| format!("{e:?}"))?;
            let reads = facts
                .uses
                .iter()
                .map(|usage| {
                    let point = usage.point;
                    if let Some(slot) = demand.mixed.cache(*id, point) {
                        return Ok((*usage, ReadLocation::Home(Location::DirectPage(slot))));
                    }
                    let location = match &capture {
                        Capture::Home(home) => ReadLocation::Home(*home),
                        Capture::NativeOutput { interval, lanes }
                            if usage.point
                                == (ProgramPoint {
                                    block: interval.block,
                                    index: interval.consumer,
                                }) =>
                        {
                            ReadLocation::NativeOutput(*lanes)
                        }
                        Capture::Borrowed(b) if b.uses.contains(&usage.point) => {
                            ReadLocation::Borrowed(b.home)
                        }
                        Capture::Registers { interval, lanes }
                            if usage.point
                                == (ProgramPoint {
                                    block: interval.block,
                                    index: interval.consumer,
                                }) =>
                        {
                            ReadLocation::Registers(*lanes)
                        }
                        Capture::Components(interval)
                            if usage.point
                                == (ProgramPoint {
                                    block: interval.block,
                                    index: interval.consumer,
                                }) =>
                        {
                            ReadLocation::Components
                        }
                        Capture::Local {
                            definition,
                            destination,
                        } if usage.point
                            == (ProgramPoint {
                                block: definition.block,
                                index: definition.index + 1,
                            }) =>
                        {
                            ReadLocation::Local(*destination)
                        }
                        Capture::Assignment { consumer, .. } if usage.point == *consumer => {
                            ReadLocation::Assignment
                        }
                        _ => return Err("placement has an uncovered logical value use".into()),
                    };
                    Ok((*usage, location))
                })
                .collect::<Result<Vec<_>, String>>()?;
            values.insert(
                *id,
                Value {
                    bytes,
                    materialized,
                    residence: demand.mixed.values.get(id).copied(),
                    capture,
                    reads,
                },
            );
        }
        if !borrowed.is_empty() {
            return Err("unknown borrowed placement value".into());
        }
        let mut edges = BTreeMap::new();
        let mut reachable = BTreeSet::new();
        let mut dp_operands = BTreeMap::new();
        let mut protected_dp = BTreeMap::new();
        let mut pointer_sources = BTreeMap::new();
        for (block_index, block) in r.blocks.iter().enumerate() {
            let b = logical.block(block.id).map_err(|e| format!("{e:?}"))?;
            let is_reachable = logical.dominates(b, b).is_ok();
            if is_reachable {
                reachable.insert(block.id);
            }
            let successors = crate::mir65816::analysis::operands::successors(r, block_index)?;
            for (ordinal, target) in successors.into_iter().enumerate() {
                let bindings: Vec<(TempId, Mir65816Value)> = if let Some(e) =
                    crate::mir65816::analysis::operands::edges(&block.terminator).get(ordinal)
                {
                    let target_block = r
                        .blocks
                        .iter()
                        .find(|b| b.id == target)
                        .ok_or("unknown placement edge")?;
                    target_block
                        .params
                        .iter()
                        .zip(&e.args)
                        .map(|((id, _), v)| (*id, v.clone()))
                        .collect()
                } else {
                    vec![]
                };
                let transfers = bindings
                    .iter()
                    .map(|(id, value)| {
                        let destination = *frame
                            .temps
                            .get(id)
                            .ok_or("edge destination has no placement home")?;
                        let source = match value {
                            Mir65816Value::Temp(id, _) => Some(
                                *frame
                                    .temps
                                    .get(id)
                                    .ok_or("edge source has no placement home")?,
                            ),
                            Mir65816Value::Param(id) => {
                                let (offset, width) = frame.parameter_home(r, *id)?;
                                Some(Location::Stack(Slot {
                                    offset: offset as u16,
                                    width,
                                }))
                            }
                            _ => None,
                        };
                        if source.is_some_and(|h| h.slot().width != destination.slot().width) {
                            return Err("partial-width placement edge transfer".into());
                        }
                        Ok(Transfer {
                            value: value.clone(),
                            source,
                            destination,
                            bytes: destination.slot().width,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                edges.insert(
                    EdgeId {
                        source: block.id,
                        ordinal,
                    },
                    Edge {
                        target,
                        bindings,
                        transfers,
                        mixed: if let Some(e) =
                            crate::mir65816::analysis::operands::edges(&block.terminator)
                                .get(ordinal)
                        {
                            if frame.word_copies(r, e, 0)?.is_none()
                                && frame.pointer_copies(r, e, 0)?.is_none()
                            {
                                frame.mixed_copies(r, e)?
                            } else {
                                None
                            }
                        } else {
                            None
                        },
                    },
                );
            }
            // One reverse sweep, not a liveness query per operation.
            let mut live: BTreeSet<_> = if is_reachable {
                logical
                    .live_at(
                        logical
                            .point(b, block.ops.len())
                            .map_err(|e| format!("{e:?}"))?,
                    )
                    .map_err(|e| format!("{e:?}"))?
                    .into_iter()
                    .map(|t| t.id())
                    .collect()
            } else {
                BTreeSet::new()
            };
            let edge_outputs: BTreeSet<_> = edges
                .iter()
                .filter(|(id, _)| id.source == block.id)
                .flat_map(|(_, edge)| edge.bindings.iter().map(|(id, _)| *id))
                .collect();
            for index in (0..=block.ops.len()).rev() {
                let census = if let Some(op) = block.ops.get(index) {
                    crate::mir65816::analysis::operands::operation(op)
                } else {
                    crate::mir65816::analysis::operands::terminator(&block.terminator)
                };
                let mut operands: BTreeSet<_> = census
                    .inputs
                    .iter()
                    .filter_map(|v| match v {
                        Mir65816Value::Temp(id, _) => Some(*id),
                        _ => None,
                    })
                    .collect();
                let mut outputs: BTreeSet<_> =
                    census.definition.map(|(id, _)| id).into_iter().collect();
                if let Some((id, _)) = census.definition {
                    live.remove(&id);
                    operands.insert(id);
                }
                let fused = index + 1 == block.ops.len()
                    && matches!(block.ops.get(index), Some(Mir65816Op::Compare { .. }))
                    && matches!(block.terminator, Mir65816Terminator::Branch { .. });
                if index == block.ops.len() || fused {
                    outputs.extend(&edge_outputs);
                    operands.extend(&edge_outputs);
                    operands.extend(
                        crate::mir65816::analysis::operands::terminator(&block.terminator)
                            .inputs
                            .iter()
                            .filter_map(|v| match v {
                                Mir65816Value::Temp(id, _) => Some(*id),
                                _ => None,
                            }),
                    );
                }
                let point = ProgramPoint {
                    block: block.id,
                    index,
                };
                for (&id, list) in &demand.mixed.segments {
                    if list.iter().any(|v| v.block == block.id && v.first == index) {
                        outputs.insert(id);
                        operands.insert(id);
                    }
                }
                let bytes = |ids: &BTreeSet<TempId>| {
                    ids.iter()
                        .filter_map(|id| {
                            demand
                                .mixed
                                .values
                                .get(id)
                                .filter(|v| v.contains(point))
                                .map(|v| Location::DirectPage(v.slot))
                                .or_else(|| {
                                    demand.mixed.cache(*id, point).map(Location::DirectPage)
                                })
                                .or_else(|| frame.temps.get(id).copied())
                        })
                        .filter_map(|home| match home {
                            Location::DirectPage(s) => {
                                resources::Scratch::range(s.offset, u16::from(s.width))
                            }
                            _ => None,
                        })
                        .fold(resources::Scratch::default(), resources::Scratch::union)
                };
                let point = ProgramPoint {
                    block: block.id,
                    index,
                };
                for input in &census.inputs {
                    let source = match input {
                        Mir65816Value::Temp(id, bytes) if bytes.get() == 3 => {
                            let v = &values[id];
                            match &v.capture {
                                Capture::Borrowed(b) => Some((
                                    match b.source {
                                        InputOwner::Parameter(id) => {
                                            tracked::PointerOrigin::Parameter(id)
                                        }
                                        InputOwner::Frame(id) => tracked::PointerOrigin::Frame(id),
                                    },
                                    b.home,
                                )),
                                _ => v
                                    .materialized
                                    .and_then(|h| h.stack().ok())
                                    .map(|s| (tracked::PointerOrigin::Temporary(*id), s)),
                            }
                        }
                        Mir65816Value::Param(id) => {
                            let (offset, width) = frame.parameter_home(r, *id)?;
                            (width == 3).then_some((
                                tracked::PointerOrigin::Parameter(*id),
                                Slot {
                                    offset: offset as u16,
                                    width,
                                },
                            ))
                        }
                        _ => None,
                    };
                    if let Some((origin, slot)) = source {
                        pointer_sources.insert((point, origin), slot);
                    }
                }
                dp_operands.insert(point, bytes(&operands));
                let resident: BTreeSet<_> = live
                    .union(&operands)
                    .copied()
                    .filter(|id| !outputs.contains(id))
                    .collect();
                let writable = bytes(&outputs);
                // Pointer identity/reload aliases can share complete homes;
                // their existing dedicated verifier checks the phased handoff.
                let protection = bytes(&resident);
                protected_dp.insert(point, protection.without(writable));
                live.extend(census.inputs.iter().filter_map(|v| match v {
                    Mir65816Value::Temp(id, _) => Some(*id),
                    _ => None,
                }));
            }
        }
        // One dense row per complete operation, including empty scratch masks.
        // Intern identical immutable descriptions; do not duplicate several
        // full point-keyed maps in every retained selected generation.
        let windows: BTreeMap<_, _> = resources
            .windows
            .keys()
            .enumerate()
            .map(|(index, &point)| (point, WindowId(index)))
            .collect();
        let mut descriptions = Vec::new();
        let mut window_rows = Vec::with_capacity(windows.len());
        for (point, requirements) in resources.windows {
            let description =
                if let Some(index) = descriptions.iter().position(|r| r == &requirements) {
                    DescriptionId(index)
                } else {
                    let id = DescriptionId(descriptions.len());
                    descriptions.push(requirements);
                    id
                };
            window_rows.push(Window {
                description,
                dp_operands: dp_operands
                    .remove(&point)
                    .ok_or("missing operation DP resources")?,
                protected_dp: protected_dp
                    .remove(&point)
                    .ok_or("missing operation DP protection")?,
                access_owner: *windows
                    .get(&access_owner[&point])
                    .ok_or("unknown compound access owner")?,
            });
        }
        Ok(Contract {
            owner: None,
            routine: r.id,
            frame: frame.clone(),
            values,
            calls: call_flow::plan(r, logical)?,
            windows,
            window_rows,
            descriptions,
            edges,
            entries: demand.mixed.entries.clone(),
            regions: demand.mixed.regions.keys().copied().collect(),
            loops: demand.mixed.loops.clone(),
            segments: demand.mixed.segments.clone(),
            aggregates: r
                .blocks
                .iter()
                .flat_map(|b| {
                    b.ops.iter().enumerate().filter_map(|(index, op)| {
                        if let Mir65816Op::Copy {
                            bytes,
                            overlap_safe,
                            source_volatile: false,
                            destination_volatile: false,
                            ..
                        } = op
                        {
                            (!bytes.is_zero()).then_some((
                                ProgramPoint { block: b.id, index },
                                (bytes.get(), *overlap_safe),
                            ))
                        } else {
                            None
                        }
                    })
                })
                .collect(),
            entry: r.blocks[0].id,
            reachable,
            labels: BTreeMap::new(),
            pointer_sources,
            x: None,
        })
    }

    /// Recompute ownership and logical consumption from the immutable MIR and
    /// final allocation. A malformed candidate is an error, never fallback.
    pub fn verify(&self) -> Result<(), String> {
        let r = self.routine;
        let demand = home_demand::Plan::new(r);
        if demand.decisions != self.demand.decisions || demand.mixed != self.demand.mixed {
            return Err("placement home demand differs from MIR admission".into());
        }
        if demand.locals != self.demand.locals || demand.pointers != self.demand.pointers {
            return Err("placement demand consumers differ from MIR admission".into());
        }
        if !demand.mixed.values.is_empty() {
            self.frame.verify_stack_with_demand(r, &demand)?;
        } else if self
            .frame
            .temps
            .values()
            .any(|h| matches!(h, Location::DirectPage(s) if s.width == 3))
        {
            self.frame.verify_pointer_leaf(r)?;
        } else if self
            .frame
            .temps
            .values()
            .any(|h| matches!(h, Location::DirectPage(_)))
        {
            self.frame.verify_scalar_dp(r)?;
        } else {
            self.frame.verify_stack_with_demand(r, &demand)?;
        }
        let pointers = demand.pointers.resolve(r, &self.frame)?;
        let scalars = select::scalar_forwarding::Plan::new(r, &self.frame)?;
        let components = r
            .blocks
            .iter()
            .map(|b| {
                Ok((
                    b.id,
                    select::address_consumers::Plan::new(r, &self.frame, b, &demand, &pointers)?,
                ))
            })
            .collect::<Result<_, String>>()?;
        let counts = liveness::input_counts(r);
        let assignments = r
            .blocks
            .iter()
            .map(|b| {
                (
                    b.id,
                    select::direct_assignments::Plan::new(r, &self.frame, b, &counts, self.data),
                )
            })
            .collect();
        if self.pointers != pointers
            || self.scalars != scalars
            || self.components != components
            || self.assignments != assignments
        {
            return Err("placement selector owners differ from recomputed admission".into());
        }
        let expected = Self::describe(
            r,
            &self.logical,
            &demand,
            &self.frame,
            &pointers,
            &scalars,
            &components,
            &assignments,
            resources::Plan::new(r),
        )?;
        if self.contract.values != expected.values
            || self.calls != expected.calls
            || self.contract.calls != expected.calls
            || self.contract.routine != expected.routine
            || self.contract.entry != expected.entry
            || self.contract.reachable != expected.reachable
            || !self.contract.labels.is_empty()
            || self.contract.x.is_some()
            || self.contract.owner.is_some()
            || self.contract.windows != expected.windows
            || self.contract.edges != expected.edges
            || self.contract.entries != expected.entries
            || self.contract.regions != expected.regions
            || self.contract.loops != expected.loops
            || self.contract.segments != expected.segments
            || self.contract.aggregates != expected.aggregates
            || self.contract.window_rows != expected.window_rows
            || self.contract.descriptions != expected.descriptions
            || self.contract.pointer_sources != expected.pointer_sources
            || self.contract.frame != self.frame
        {
            return Err("placement differs from recomputed value/resource obligations".into());
        }
        for (id, edge) in &self.contract.edges {
            for (&temp, &home) in self
                .contract
                .entries
                .get(&edge.target)
                .into_iter()
                .flatten()
            {
                if let Some(ordinal) = edge.bindings.iter().position(|(t, _)| *t == temp) {
                    if edge.transfers[ordinal].destination != home
                        || edge.transfers[ordinal].bytes != home.slot().width
                    {
                        return Err("incoming edge fails resident parameter obligation".into());
                    }
                } else if self.frame.temps.get(&temp) != Some(&home)
                    || !demand.mixed.regions.get(&temp).is_some_and(|points| {
                        points.contains(&ProgramPoint {
                            block: id.source,
                            index: r
                                .blocks
                                .iter()
                                .find(|b| b.id == id.source)
                                .unwrap()
                                .ops
                                .len(),
                        })
                    })
                {
                    return Err("incoming edge fails surviving residence obligation".into());
                }
            }
        }
        for (&id, value) in &self.contract.values {
            let facts = self
                .logical
                .value(self.logical.temp(id).map_err(|e| format!("{e:?}"))?)
                .map_err(|e| format!("{e:?}"))?;
            if value
                .materialized
                .is_some_and(|h| h.slot().width != value.bytes)
            {
                return Err("partial-width placement home".into());
            }
            match &value.capture {
                Capture::Home(home) if Some(*home) == value.materialized => (),
                Capture::NativeOutput { interval, lanes } => {
                    if value.materialized.is_some()
                        || !self.demand.omits(id)
                        || facts.definition
                            != Definition::Operation(ProgramPoint {
                                block: interval.block,
                                index: interval.producer,
                            })
                        || facts.uses.len() != 1
                        || facts.uses[0].point
                            != (ProgramPoint {
                                block: interval.block,
                                index: interval.consumer,
                            })
                        || interval.consumer != interval.producer + 1
                        || *lanes != Lanes::for_width(value.bytes)?
                    {
                        return Err("invalid native output ownership".into());
                    }
                }
                Capture::Borrowed(b) => {
                    if b.temp != id
                        || b.home.width != value.bytes
                        || facts.definition != Definition::Operation(b.definition)
                        || b.uses.is_empty()
                        || self.demand.omits(id) != value.materialized.is_none()
                    {
                        return Err("invalid borrowed placement lifetime".into());
                    }
                }
                Capture::Registers { interval, lanes } => {
                    self.check_interval(id, *interval)?;
                    if *lanes != Lanes::for_width(value.bytes)? {
                        return Err("partial-width placement register lanes".into());
                    }
                }
                Capture::Components(interval) => self.check_interval(id, *interval)?,
                Capture::Local {
                    definition,
                    destination,
                } => {
                    if value.materialized.is_some()
                        || destination.width != value.bytes
                        || facts.definition != Definition::Operation(*definition)
                        || facts.uses.len() != 1
                    {
                        return Err("invalid redirected placement capture".into());
                    }
                }
                Capture::Assignment {
                    definition,
                    consumer,
                } => {
                    if facts.definition != Definition::Operation(*definition)
                        || facts.uses.len() != 1
                        || consumer.block != definition.block
                        || consumer.index != definition.index + 1
                    {
                        return Err("invalid deferred assignment placement".into());
                    }
                }
                _ => return Err("missing or conflicting placement home".into()),
            }
        }
        if self.loop_x != loop_x::LoopXPlan::new(r, &self.frame)? {
            return Err("inconsistent placement X boundary".into());
        }
        Ok(())
    }
    fn check_interval(&self, id: TempId, interval: home_demand::Accumulator) -> Result<(), String> {
        let v = &self.contract.values[&id];
        let facts = self
            .logical
            .value(self.logical.temp(id).map_err(|e| format!("{e:?}"))?)
            .map_err(|e| format!("{e:?}"))?;
        if v.materialized.is_some()
            || interval.bytes != v.bytes
            || interval.producer >= interval.consumer
            || facts.definition
                != Definition::Operation(ProgramPoint {
                    block: interval.block,
                    index: interval.producer,
                })
            || facts.uses.len() != 1
            || !self.demand.omits(id)
        {
            return Err("invalid placement register/deferred interval".into());
        }
        Ok(())
    }
    pub fn seal(&self, labels: &BTreeMap<BlockId, Label>) -> Result<Contract, String> {
        if labels.keys().copied().collect::<BTreeSet<_>>()
            != self.routine.blocks.iter().map(|b| b.id).collect()
            || labels.values().copied().collect::<BTreeSet<_>>().len() != labels.len()
        {
            return Err("invalid placement block labels".into());
        }
        let mut contract = self.contract.clone();
        contract.labels = labels.clone();
        contract.x = self.loop_x.as_ref().map(|x| tracked::XContract {
            param: x.param,
            increment: x.increment.map(|id| (id, self.frame.temps[&id])),
            home: x.home,
            header: labels[&x.header],
            body: labels[&x.body],
            predecessors: [labels[&x.preheader], labels[&x.body]].into(),
        });
        Ok(contract)
    }
}

impl Contract {
    fn requirements(&self, point: ProgramPoint) -> Option<&resources::Requirements> {
        let id = self.windows.get(&point)?;
        self.descriptions
            .get(self.window_rows.get(id.0)?.description.0)
    }
    #[cfg(test)]
    fn requirements_mut(&mut self, point: ProgramPoint) -> &mut resources::Requirements {
        let id = self.windows[&point];
        &mut self.descriptions[self.window_rows[id.0].description.0]
    }
    pub fn bind_owner(&mut self, selected: &SelectedRoutine) -> Result<(), String> {
        if self.owner.is_some() {
            return Err("placement contract already belongs to a selected allocation".into());
        }
        self.owner = Some(selected.identity());
        Ok(())
    }
    #[cfg(feature = "native65816-state-proof")]
    pub fn summary(&self) -> super::proof::PlacementSummary {
        let count =
            |f: fn(&Capture) -> bool| self.values.values().filter(|v| f(&v.capture)).count();
        let form = |f| {
            self.window_rows
                .iter()
                .filter(|r| self.descriptions[r.description.0].form == f)
                .count()
        };
        let dp_uses: BTreeSet<_> = self
            .values
            .values()
            .flat_map(|v| &v.reads)
            .filter_map(|(usage, location)| {
                matches!(location, ReadLocation::Home(Location::DirectPage(_)))
                    .then_some(usage.point)
            })
            .collect();
        let resident_inputs = |f| {
            self.windows
                .iter()
                .filter(|(_, window)| {
                    self.descriptions[self.window_rows[window.0].description.0].form == f
                })
                .filter(|(point, _)| dp_uses.contains(point))
                .count()
        };
        super::proof::PlacementSummary {
            values: self.values.len(),
            materialized: self.frame.temps.len(),
            mixed_homes: self
                .values
                .values()
                .filter(|v| v.residence.is_some_and(|r| !r.backed))
                .count(),
            backed_residences: self
                .values
                .values()
                .filter(|v| v.residence.is_some_and(|r| r.backed))
                .count(),
            branch_homes: self.regions.len(),
            resident_entries: self.entries.len(),
            mixed_edges: self.edges.values().filter(|e| e.mixed.is_some()).count(),
            loop_homes: self.loops.len(),
            call_segments: self.segments.values().map(Vec::len).sum(),
            borrowed: count(|c| matches!(c, Capture::Borrowed(_))),
            register_intervals: count(|c| matches!(c, Capture::Registers { .. })),
            native_output_intervals: count(|c| matches!(c, Capture::NativeOutput { .. })),
            component_intervals: count(|c| matches!(c, Capture::Components(_))),
            redirected_locals: count(|c| matches!(c, Capture::Local { .. })),
            deferred_assignments: count(|c| matches!(c, Capture::Assignment { .. })),
            windows: self.windows.len(),
            record_windows: form(resources::Form::RecordMemory),
            indexed_windows: form(resources::Form::IndexedMemory),
            aggregate_windows: form(resources::Form::Aggregate),
            resident_indexed_windows: resident_inputs(resources::Form::IndexedMemory),
            resident_aggregate_windows: resident_inputs(resources::Form::Aggregate),
            scalar_windows: form(resources::Form::Scalar),
            address_windows: form(resources::Form::Address),
            barriers: form(resources::Form::Barrier),
            boundaries: form(resources::Form::Boundary),
            edges: self.edges.len(),
            transfers: self.edges.values().map(|e| e.transfers.len()).sum(),
            staging_bytes: self
                .frame
                .edge_copies
                .iter()
                .map(|s| usize::from(s.width))
                .sum(),
            x_mirror: self.x.is_some(),
        }
    }

    pub fn verify_selected(&self, selected: &SelectedRoutine) -> Result<(), String> {
        if self
            .owner
            .is_some_and(|owner| !owner.same_allocation(selected.identity()))
        {
            return Err("foreign or stale placement allocation".into());
        }
        if selected.routine_id() != self.routine || selected.allocation != self.frame {
            return Err("selected allocation differs from placement".into());
        }
        if !self.labels.contains_key(&self.entry)
            || !self
                .reachable
                .is_subset(&self.labels.keys().copied().collect())
            || self.edges.iter().any(|(id, e)| {
                !self.labels.contains_key(&id.source) || !self.labels.contains_key(&e.target)
            })
        {
            return Err("unknown placement boundary identity".into());
        }
        if self.windows.len() != self.window_rows.len()
            || self.windows.values().copied().collect::<BTreeSet<_>>()
                != (0..self.window_rows.len()).map(WindowId).collect()
            || self.window_rows.iter().any(|row| {
                row.description.0 >= self.descriptions.len()
                    || row.access_owner.0 >= self.window_rows.len()
            })
        {
            return Err("incomplete placement resource locations".into());
        }
        let mut expected_predecessors: BTreeMap<_, BTreeMap<_, usize>> = self
            .labels
            .values()
            .map(|&l| (l, BTreeMap::new()))
            .collect();
        expected_predecessors
            .get_mut(&self.labels[&self.entry])
            .ok_or("missing placement entry label")?
            .insert(None, 1);
        for (id, edge) in &self.edges {
            *expected_predecessors
                .get_mut(&self.labels[&edge.target])
                .ok_or("missing placement successor label")?
                .entry(Some(self.labels[&id.source]))
                .or_default() += 1;
        }
        let reachable: BTreeSet<_> = self.reachable.iter().map(|id| self.labels[id]).collect();
        let mut entries = 0;
        let mut x_contracts = 0;
        let mut covered = BTreeSet::new();
        let mut calls = BTreeSet::new();
        let mut outputs = BTreeSet::new();
        let mut output_reads = BTreeSet::new();
        let mut captures = BTreeSet::new();
        let mut reloads = BTreeSet::new();
        let mut aggregates = BTreeSet::new();
        let mut mixed_edges = BTreeSet::new();
        struct Active<'a> {
            point: ProgramPoint,
            window: &'a Window,
            requirements: &'a resources::Requirements,
            external: (u32, u32),
        }
        let mut active: Option<Active<'_>> = None;
        let mut external = vec![(0u32, 0u32); self.window_rows.len()];
        for record in selected.records() {
            match &record.action {
                Action::Request(Request::ProveEntries {
                    predecessors,
                    reachable: actual,
                }) => {
                    if predecessors != &expected_predecessors || actual != &reachable {
                        return Err("inconsistent placement boundary requirements".into());
                    }
                    entries += 1;
                }
                Action::Request(Request::PublishNative(temp, lanes))
                | Action::Request(Request::ConsumeNative(temp, lanes)) => {
                    let point = active
                        .as_ref()
                        .ok_or("native output outside source span")?
                        .point;
                    let value = self.values.get(temp).ok_or("unknown native output")?;
                    let Capture::NativeOutput { interval, .. } = value.capture else {
                        return Err("unplanned native output".into());
                    };
                    let call = self
                        .calls
                        .get(&ProgramPoint {
                            block: interval.block,
                            index: interval.producer,
                        })
                        .ok_or("missing native producer")?;
                    let declaration = call.result.ok_or("missing native declaration")?;
                    let publish =
                        matches!(record.action, Action::Request(Request::PublishNative(..)));
                    if declaration.temp != *temp
                        || declaration.lanes != *lanes
                        || point.block != interval.block
                        || point.index
                            != if publish {
                                interval.producer
                            } else {
                                interval.consumer
                            }
                        || !(if publish {
                            outputs.insert(*temp)
                        } else {
                            output_reads.insert(*temp)
                        })
                    {
                        return Err("native output differs from placement owner".into());
                    }
                }
                Action::Request(Request::ProveX(actual)) => {
                    if self.x.as_ref() != Some(actual) {
                        return Err("selected X mirror differs from placement boundary".into());
                    }
                    x_contracts += 1;
                }
                Action::Request(Request::CaptureResident(id, source, destination)) => {
                    let w = active
                        .as_ref()
                        .ok_or("residence capture outside operation")?;
                    let value = self.values.get(id).ok_or("unknown residence capture")?;
                    let residence = value.residence.ok_or("unplanned residence capture")?;
                    if !residence.backed
                        || residence.block != w.point.block
                        || residence.first != w.point.index
                        || value.materialized != Some(Location::Stack(*source))
                        || residence.slot != *destination
                        || !captures.insert(*id)
                    {
                        return Err("residence capture differs from placement transfer".into());
                    }
                }
                Action::Request(Request::MixedEdge(plan, staging)) => {
                    let point = active
                        .as_ref()
                        .ok_or("mixed edge outside source boundary")?
                        .point;
                    let (id, _) = self
                        .edges
                        .iter()
                        .find(|(id, e)| {
                            id.source == point.block
                                && !mixed_edges.contains(*id)
                                && e.mixed.as_ref() == Some(plan)
                        })
                        .ok_or("mixed transfer differs from simultaneous edge obligations")?;
                    if self.frame.mixed_staging(plan)? != *staging {
                        return Err("mixed transfer staging differs from allocation".into());
                    }
                    mixed_edges.insert(*id);
                }
                Action::Request(Request::ReloadResident(id, source, destination)) => {
                    let point = active
                        .as_ref()
                        .ok_or("resident reload outside operation")?
                        .point;
                    if self.frame.temps.get(id) != Some(&Location::Stack(*source))
                        || !self.segments.get(id).is_some_and(|list| {
                            list.iter().any(|v| {
                                v.block == point.block
                                    && v.first == point.index
                                    && v.slot == *destination
                            })
                        })
                        || !reloads.insert((*id, point))
                    {
                        return Err("resident reload differs from invocation-backed segment".into());
                    }
                }
                Action::Request(Request::AggregateCopy {
                    bytes,
                    overlap_safe,
                }) => {
                    let point = active
                        .as_ref()
                        .ok_or("aggregate transfer outside operation")?
                        .point;
                    if record.parent.is_some()
                        || self.aggregates.get(&point) != Some(&(*bytes, *overlap_safe))
                        || !aggregates.insert(point)
                    {
                        return Err(
                            "aggregate transfer differs from MIR extent or overlap protocol".into(),
                        );
                    }
                }
                Action::Request(Request::StagePointer(origin, source, scratch)) => {
                    let w = active
                        .as_ref()
                        .ok_or("pointer residence outside placement operation")?;
                    if self.pointer_sources.get(&(w.point, *origin)) != Some(source)
                        || !w.requirements.scratch.contains(
                            resources::Scratch::range(u16::from(*scratch), 3)
                                .ok_or("unowned pointer residence scratch")?,
                        )
                    {
                        return Err(
                            "selected pointer residence differs from placement input".into()
                        );
                    }
                }
                Action::SourceStart(s) => {
                    let p = ProgramPoint {
                        block: s.block,
                        index: s.index,
                    };
                    if active.is_some() || !covered.insert(p) {
                        return Err("missing, duplicate or unknown placement operation".into());
                    }
                    let id = self.windows.get(&p).ok_or("unknown placement operation")?;
                    let window = &self.window_rows[id.0];
                    active = Some(Active {
                        point: p,
                        window,
                        requirements: &self.descriptions[window.description.0],
                        external: (0, 0),
                    });
                    let env = record.before.env;
                    if env.depth != i64::from(self.frame.extent)
                        || env.anchor != Some(env.depth)
                        || env.pushes != 0
                        || !env.native
                        || !env.current_domain
                        || env.index != Width::Word
                        || env.decimal != Some(false)
                        || (p.index == 0 && env.m != Width::Word)
                    {
                        return Err("inconsistent placement operation entry state".into());
                    }
                }
                Action::Instruction { form, effects, .. } if active.is_some() => {
                    let w = active.as_mut().unwrap();
                    let p = w.point;
                    if matches!(
                        form,
                        Instruction::NativeCall(..) | Instruction::IndirectTransfer(Some(_))
                    ) {
                        let call = self.calls.get(&p).ok_or("unplanned native call transfer")?;
                        if !call.check_transfer(form)? || !calls.insert(p) {
                            return Err("duplicate or invalid native call transfer".into());
                        }
                    }
                    for effect in &effects.memory {
                        if let super::effects::Memory::DirectPage { offset, bytes } = effect.memory
                        {
                            if effect.access == super::effects::Access::Read
                                && self.segments.iter().any(|(&id, list)| {
                                    list.iter().any(|v| {
                                        v.block == p.block
                                            && v.first == p.index
                                            && resources::Scratch::range(offset, bytes).is_some_and(
                                                |range| {
                                                    resources::Scratch::range(
                                                        v.slot.offset,
                                                        v.slot.width.into(),
                                                    )
                                                    .unwrap()
                                                    .overlaps(range)
                                                },
                                            )
                                            && !reloads.contains(&(id, p))
                                    })
                                })
                            {
                                return Err(
                                    "resident consumer precedes its invocation reload".into()
                                );
                            }
                            if effect.access != super::effects::Access::Read
                                && resources::Scratch::range(offset, bytes)
                                    .is_some_and(|range| w.window.protected_dp.overlaps(range))
                            {
                                return Err(
                                    "operation scratch conflicts with a live placement value"
                                        .into(),
                                );
                            }
                        }
                    }
                    let used = resources::check_instruction(
                        w.requirements,
                        form,
                        record.before.env,
                        effects,
                        w.window.dp_operands,
                    )
                    .map_err(|e| format!("b{}:{}: {e}", p.block.0, p.index))?;
                    w.external.0 += used.0;
                    w.external.1 += used.1;
                    let e = record.after.env;
                    if e.depth > i64::from(self.frame.peak_below_entry)
                        || e.depth
                            > i64::from(self.frame.extent) + i64::from(w.requirements.extra_stack)
                    {
                        return Err("selected operation exceeds placement stack peak".into());
                    }
                }
                Action::SourceEnd {
                    source,
                    fused_terminator,
                } => {
                    let w = active.take().ok_or("missing placement source window")?;
                    let p = w.point;
                    if p != (ProgramPoint {
                        block: source.block,
                        index: source.index,
                    }) {
                        return Err("mismatched placement source window".into());
                    }
                    let total = &mut external[w.window.access_owner.0];
                    total.0 += w.external.0;
                    total.1 += w.external.1;
                    if let Some(index) = fused_terminator {
                        let t = ProgramPoint {
                            block: p.block,
                            index: *index,
                        };
                        if self
                            .requirements(t)
                            .is_none_or(|r| r.form != resources::Form::Boundary)
                            || !covered.insert(t)
                        {
                            return Err("invalid fused placement boundary".into());
                        }
                    }
                }
                _ => (),
            }
        }
        let expected_outputs: BTreeSet<_> = self
            .values
            .iter()
            .filter_map(|(id, v)| matches!(v.capture, Capture::NativeOutput { .. }).then_some(*id))
            .collect();
        if outputs != expected_outputs || output_reads != expected_outputs {
            return Err("incomplete native output ownership coverage".into());
        }
        if calls != self.calls.keys().copied().collect() {
            return Err("missing native call transfer".into());
        }
        if captures
            != self
                .values
                .iter()
                .filter_map(|(&id, v)| v.residence.filter(|r| r.backed).map(|_| id))
                .collect()
            || active.is_some()
            || entries != 1
            || x_contracts != usize::from(self.x.is_some())
            || covered != self.windows.keys().copied().collect()
        {
            return Err("incomplete selected placement/resource coverage".into());
        }
        if mixed_edges
            != self
                .edges
                .iter()
                .filter(|(_, e)| e.mixed.is_some())
                .map(|(id, _)| *id)
                .collect()
        {
            return Err("incomplete simultaneous mixed transfer coverage".into());
        }
        if reloads
            != self
                .segments
                .iter()
                .flat_map(|(&id, list)| {
                    list.iter().map(move |v| {
                        (
                            id,
                            ProgramPoint {
                                block: v.block,
                                index: v.first,
                            },
                        )
                    })
                })
                .collect()
        {
            return Err("incomplete invocation-backed segment coverage".into());
        }
        if aggregates != self.aggregates.keys().copied().collect() {
            return Err("incomplete aggregate transfer protocol coverage".into());
        }
        let mut expected = vec![(Some(0u32), Some(0u32)); self.window_rows.len()];
        for window in &self.window_rows {
            let requirements = &self.descriptions[window.description.0];
            let total = &mut expected[window.access_owner.0];
            total.0 = total.0.zip(requirements.external_reads).map(|(a, b)| a + b);
            total.1 = total
                .1
                .zip(requirements.external_writes)
                .map(|(a, b)| a + b);
        }
        for (index, ((reads, writes), actual)) in expected.into_iter().zip(external).enumerate() {
            if reads.is_some_and(|n| n != actual.0) || writes.is_some_and(|n| n != actual.1) {
                let p = self.windows.iter().find(|(_, id)| id.0 == index).unwrap().0;
                return Err(format!(
                    "b{}:{}: external access extent differs from resource contract",
                    p.block.0, p.index
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
