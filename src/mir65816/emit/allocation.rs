use super::super::*;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Slot {
    pub offset: u16,
    pub width: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocatedFrame {
    pub extent: u16,
    pub spill_bytes: u16,
    /// Exact local reservation/transfer peak, excluding callees' reservations
    /// and the platform's separately reserved interrupt headroom.
    pub peak_below_entry: u16,
    /// Materialized temporaries only. Storage-demand-verified accumulator
    /// lifetimes have no fictitious stack/DP entry.
    pub temps: BTreeMap<TempId, Location>,
    /// Scratch pool in capture order; selective move indices need not be dense.
    pub edge_copies: Vec<Slot>,
}

pub(super) fn width(width: ByteSize) -> Result<u8, String> {
    match width.get() {
        1..=4 => Ok(width.get() as u8),
        _ => Err(format!("unsupported native scalar width {}", width.get())),
    }
}

impl AllocatedFrame {
    pub(super) fn forwarding(
        program: &Mir65816Program,
        routine: &Mir65816Routine,
        plan: &super::forwarding::Plan,
    ) -> Result<Self, String> {
        let frame = Self {
            extent: 0,
            spill_bytes: 0,
            peak_below_entry: 0,
            temps: BTreeMap::new(),
            edge_copies: Vec::new(),
        };
        frame.verify_forwarding(program, routine, plan)?;
        Ok(frame)
    }

    pub(super) fn verify_forwarding(
        &self,
        program: &Mir65816Program,
        routine: &Mir65816Routine,
        plan: &super::forwarding::Plan,
    ) -> Result<(), String> {
        // Recompute the complete correspondence and cycle proof. Empty homes
        // alone never authorize removing frame allocation or entry checks.
        plan.verify(program, routine)?;
        if self.extent != 0
            || self.spill_bytes != 0
            || self.peak_below_entry != 0
            || !self.temps.is_empty()
            || !self.edge_copies.is_empty()
        {
            return Err("forwarding wrapper must have zero local storage and peak".into());
        }
        Ok(())
    }

    /// Unit selectors may deliberately test materialized fallback forms without
    /// running the whole-routine demand selector. Give those fixtures real,
    /// disjoint homes; production allocation must never use this path.
    #[cfg(test)]
    pub(super) fn materialized_fixture(routine: &Mir65816Routine) -> Result<Self, String> {
        let mut frame = Self::stack(routine)?;
        let mut cursor = u32::from(frame.extent) + 1;
        for (id, ty) in &routine.temps {
            if frame.temps.contains_key(id) {
                continue;
            }
            let bytes = width(ty.width.ok_or("missing fixture width")?)?;
            if bytes > 1 {
                cursor = (cursor + 1) & !1;
            }
            frame.temps.insert(
                *id,
                Location::Stack(Slot {
                    offset: cursor as u16,
                    width: bytes,
                }),
            );
            cursor += u32::from(bytes);
        }
        frame.extent = abi::stack::fixed_extent(ByteSize::new(cursor - 1))
            .map_err(|e| e.to_string())?
            .get() as u16;
        frame.spill_bytes = frame.extent - routine.frame.extent.get() as u16;
        frame.peak_below_entry = local_peak(routine, frame.extent)?;
        Ok(frame)
    }

    #[cfg(any(test, feature = "native65816-state-proof"))]
    pub(super) fn new(routine: &Mir65816Routine) -> Result<Self, String> {
        Self::with_demand(routine, &super::home_demand::Plan::new(routine))
    }

    pub(super) fn with_demand(
        routine: &Mir65816Routine,
        demand: &super::home_demand::Plan,
    ) -> Result<Self, String> {
        if let Some(frame) = Self::pointer_leaf(routine)? {
            return Ok(frame);
        }
        let mut frame = Self::stack_with_demand(routine, demand)?;
        frame.promote_scalar(routine)?;
        Ok(frame)
    }

    #[cfg(test)]
    pub(super) fn stack(routine: &Mir65816Routine) -> Result<Self, String> {
        let demand = super::home_demand::Plan::new(routine);
        Self::stack_with_demand(routine, &demand)
    }

    fn stack_with_demand(
        routine: &Mir65816Routine,
        demand: &super::home_demand::Plan,
    ) -> Result<Self, String> {
        let mut frame = Self::layout(routine, &demand)?;
        frame.verify_stack_with_demand(routine, demand)?;
        let interference = super::liveness::interference(routine)?;
        frame.coalesce_edges(routine, &interference)?;
        frame.coalesce_pointer_casts(routine)?;
        Ok(frame)
    }

    // A bounded layout preview lets demand planning retain the existing exact
    // call/stack geometry checks. It never recursively plans or verifies demand.
    pub(super) fn layout(
        routine: &Mir65816Routine,
        demand: &super::home_demand::Plan,
    ) -> Result<Self, String> {
        Self::layout_for_residence(routine, demand, &demand.mixed)
    }

    pub(super) fn layout_for_residence(
        routine: &Mir65816Routine,
        demand: &super::home_demand::Plan,
        residence: &super::mixed::Plan,
    ) -> Result<Self, String> {
        let interference = super::liveness::interference(routine)?;
        let mut cursor = routine.frame.extent.get() + 1;
        let mut ordered = routine
            .temps
            .iter()
            .filter(|(id, _)| !demand.omits(*id) && residence.home(*id).is_none())
            .map(|(id, ty)| {
                Ok((
                    *id,
                    width(ty.width.ok_or("temporary has no scalar width")?)?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        // Large, highly constrained values first; stable IDs break ties. Keep
        // actual widths in maps even when differently sized values share bytes.
        ordered.sort_by_key(|(id, width)| {
            (
                std::cmp::Reverse(*width),
                std::cmp::Reverse(interference[id].len()),
                *id,
            )
        });
        let mut temps = residence
            .values
            .iter()
            .filter_map(|(&id, _)| residence.home(id).map(|home| (id, home)))
            .collect::<BTreeMap<TempId, Location>>();
        for (id, width) in ordered {
            let mut offset = routine.frame.extent.get() + 1;
            loop {
                if width > 1 {
                    offset = (offset + 1) & !1;
                }
                abi::stack::access_displacement(
                    ByteOffset::new(offset),
                    ByteSize::new(width.into()),
                    ByteSize::ZERO,
                )
                .map_err(|e| e.to_string())?;
                let slot = Slot {
                    offset: offset as u16,
                    width,
                };
                if interference[&id].iter().all(|other| {
                    temps
                        .get(other)
                        .is_none_or(|home| !Location::Stack(slot).overlaps(*home))
                }) {
                    temps.insert(id, Location::Stack(slot));
                    cursor = cursor.max(offset + u32::from(width));
                    break;
                }
                offset += 1;
            }
        }
        // Plan with the smallest frame containing all private homes. Incoming
        // arguments are above this extent and cannot alias a destination. Adding
        // staging moves them farther away, so eligibility/order cannot change;
        // the final frame independently rechecks all argument bounds and plans.
        let mut frame = Self {
            extent: abi::stack::fixed_extent(ByteSize::new(cursor - 1))
                .map_err(|e| e.to_string())?
                .get() as u16,
            spill_bytes: 0,
            peak_below_entry: 0,
            temps,
            edge_copies: vec![],
        };
        let required = frame.staging_widths(routine)?;
        let reserve = |width: u8| -> Result<Slot, String> {
            if width > 1 {
                cursor = (cursor + 1) & !1;
            }
            let offset = cursor;
            cursor += u32::from(width);
            abi::stack::access_displacement(
                ByteOffset::new(offset),
                ByteSize::new(width.into()),
                ByteSize::ZERO,
            )
            .map_err(|e| e.to_string())?;
            Ok(Slot {
                offset: offset as u16,
                width,
            })
        };
        let edge_copies = required
            .into_iter()
            .map(reserve)
            .collect::<Result<Vec<_>, _>>()?;
        let extent = abi::stack::fixed_extent(ByteSize::new(cursor - 1))
            .map_err(|e| e.to_string())?
            .get() as u16;
        for parameter in &routine.frame.parameters {
            let Mir65816AbiHome::StackArgument { offset, size, .. } = parameter.incoming else {
                return Err("parameter has no stack home".into());
            };
            abi::stack::incoming_displacement(ByteSize::new(extent.into()), offset, size)
                .map_err(|e| e.to_string())?;
        }
        frame.extent = extent;
        frame.spill_bytes = extent - routine.frame.extent.get() as u16;
        frame.peak_below_entry = local_peak(routine, extent)?;
        frame.edge_copies = edge_copies;
        Ok(frame)
    }

    /// Recheck physical byte overlap against closed-operation CFG liveness.
    /// Image maps alone cannot establish the lifetime proof for shared homes.
    pub fn verify_stack(&self, routine: &Mir65816Routine) -> Result<(), String> {
        // Recompute register admission from typed MIR; a sparse map alone is
        // never permission to omit a value's capture or reserved storage.
        let demand = super::home_demand::Plan::new(routine);
        self.verify_stack_with_demand(routine, &demand)
    }

    pub(super) fn verify_stack_with_demand(
        &self,
        routine: &Mir65816Routine,
        demand: &super::home_demand::Plan,
    ) -> Result<(), String> {
        let graph = super::liveness::pointer_copy_interference(routine)?;
        if self.temps.len() + demand.count() != routine.temps.len() {
            return Err("invalid stack temporary count".into());
        }
        let mut end = routine.frame.extent.get();
        let mut check_slot = |slot: Slot| -> Result<(), String> {
            if u32::from(slot.offset) <= routine.frame.extent.get()
                || !(1..=4).contains(&slot.width)
                || (slot.width > 1 && slot.offset % 2 != 0)
            {
                return Err(
                    "stack temporary overlaps frame objects or has invalid alignment/width".into(),
                );
            }
            abi::stack::access_displacement(
                ByteOffset::new(slot.offset.into()),
                ByteSize::new(slot.width.into()),
                ByteSize::ZERO,
            )
            .map_err(|e| e.to_string())?;
            end = end.max(u32::from(slot.offset) + u32::from(slot.width) - 1);
            Ok(())
        };
        for (id, ty) in &routine.temps {
            if demand.omits(*id) {
                if self.temps.contains_key(id) {
                    return Err("register-only temporary has a memory home".into());
                }
                continue;
            }
            let home = *self.temps.get(id).ok_or("missing stack temporary")?;
            if let Some(expected) = demand.mixed.home(*id) {
                if home != expected {
                    return Err("mixed residence home mismatch".into());
                }
                continue;
            }
            let slot = home.stack()?;
            if Some(ByteSize::new(slot.width.into())) != ty.width {
                return Err("stack temporary width mismatch".into());
            }
            check_slot(slot)?;
            for other in &graph[id] {
                if demand.omits(*other) {
                    continue;
                }
                let other_slot = self.temps.get(other).ok_or("missing stack temporary")?;
                if Location::Stack(slot).overlaps(*other_slot) {
                    return Err("overlapping live stack temporaries".into());
                }
            }
        }
        for (source, dest) in routine
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .filter_map(super::liveness::pointer_copy)
        {
            if demand.omits(source) || demand.omits(dest) {
                continue;
            }
            let source = self.temps[&source];
            let dest = self.temps[&dest];
            if source.overlaps(dest) && source != dest {
                return Err("partially overlapping pointer cast homes".into());
            }
        }
        let required = self.staging_widths(routine)?;
        if self.edge_copies.len() != required.len() {
            return Err("invalid edge-copy slot count".into());
        }
        for (index, &slot) in self.edge_copies.iter().enumerate() {
            check_slot(slot)?;
            if slot.width != required[index]
                || self
                    .temps
                    .values()
                    .any(|home| Location::Stack(slot).overlaps(*home))
                || self.edge_copies[..index]
                    .iter()
                    .any(|&other| overlap(slot, other))
            {
                return Err("invalid or overlapping edge-copy staging slot".into());
            }
        }
        let extent = abi::stack::fixed_extent(ByteSize::new(end))
            .map_err(|e| e.to_string())?
            .get();
        if extent != u32::from(self.extent)
            || u32::from(self.spill_bytes) != extent - routine.frame.extent.get()
            || self.peak_below_entry != local_peak(routine, self.extent)?
        {
            return Err("invalid stack frame accounting".into());
        }
        for parameter in &routine.frame.parameters {
            let Mir65816AbiHome::StackArgument { offset, size, .. } = parameter.incoming else {
                return Err("parameter has no stack home".into());
            };
            abi::stack::incoming_displacement(ByteSize::new(extent), offset, size)
                .map_err(|e| e.to_string())?;
        }
        // Resolve final capture mappings independently of the minimum-frame
        // sizing pass, after proving every pool slot disjoint from live homes.
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
                if let Some(plan) = self.word_copies(routine, edge, 0)? {
                    self.word_staging(&plan, 0)?;
                } else if let Some(plan) = self.pointer_copies(routine, edge, 0)? {
                    self.pointer_staging(&plan, 0)?;
                }
            }
        }
        Ok(())
    }
}

fn overlap(a: Slot, b: Slot) -> bool {
    u32::from(a.offset) < u32::from(b.offset) + u32::from(b.width)
        && u32::from(b.offset) < u32::from(a.offset) + u32::from(a.width)
}

fn local_peak(routine: &Mir65816Routine, extent: u16) -> Result<u16, String> {
    let mut peak = u32::from(extent);
    for op in routine.blocks.iter().flat_map(|b| &b.ops) {
        if let Mir65816Op::Call { plan, .. } = op {
            let transfer = plan
                .native
                .ok_or("missing native call contract")?
                .transfer
                .peak_bytes()
                .get();
            peak = peak.max(u32::from(extent) + plan.outgoing_bytes.get() + transfer);
        }
    }
    u16::try_from(peak).map_err(|_| "local stack peak overflow".into())
}

/// A physical home for a typed MIR value. Offsets are relative to S or D,
/// respectively; they must never be interpreted interchangeably.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Location {
    Stack(Slot),
    DirectPage(Slot),
}

impl From<Slot> for Location {
    fn from(slot: Slot) -> Self {
        Self::Stack(slot)
    }
}
impl Location {
    pub fn overlaps(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Stack(_), Self::Stack(_)) | (Self::DirectPage(_), Self::DirectPage(_))
        ) && overlap(self.slot(), other.slot())
    }

    pub fn slot(self) -> Slot {
        match self {
            Self::Stack(slot) | Self::DirectPage(slot) => slot,
        }
    }
    pub fn stack(self) -> Result<Slot, String> {
        match self {
            Self::Stack(slot) => Ok(slot),
            Self::DirectPage(_) => {
                Err("direct-page location requires resident pointer selection".into())
            }
        }
    }
}

const POINTER_SLOTS: [u16; 3] = [
    abi::generated::DP_POINTER0_OFFSET as u16,
    abi::generated::DP_POINTER1_OFFSET as u16,
    abi::generated::DP_POINTER2_OFFSET as u16,
];

/// Closed operation intervals: the destination and all inputs coexist for the
/// entire operation, including the bank-byte access after the low-word access.
#[derive(Clone, Copy)]
struct Interval {
    first: usize,
    last: usize,
}

/// Each immutable identity owns a closed lifetime. Alias temporaries retain
/// their IDs and maps, but share their root's complete physical value.
struct PointerRanges {
    roots: BTreeMap<TempId, TempId>,
    ranges: BTreeMap<TempId, Interval>,
}

fn pointer_type(ty: &crate::nir::NirType) -> bool {
    ty.width == Some(ByteSize::new(3))
        && matches!(ty.kind, crate::nir::NirTypeKind::Pointer { address_space, .. }
            if address_space == crate::target::TargetLayout::DATA_ADDRESS_SPACE)
}

fn address_type(ty: &crate::nir::NirType) -> bool {
    ty.width == Some(ByteSize::new(3))
        && ty.kind.integer().is_some_and(|i| {
            i.bits == 24 && !i.signed && i.role == crate::nir::NirIntegerRole::Address
        })
}

/// Exact representation identities only: no arithmetic, dereference, storage
/// escape, callable conversion or arbitrary same-width integer equivalence.
fn pointer_identity(routine: &Mir65816Routine, op: &Mir65816Op) -> Option<(TempId, TempId)> {
    let (dest, source, kind) = match op {
        Mir65816Op::Cast {
            dest,
            from,
            from_signed: false,
            to,
            kind,
            value: Mir65816Value::Temp(source, width),
        } if from.get() == 3 && to.get() == 3 && width.get() == 3 => (*dest, *source, Some(*kind)),
        Mir65816Op::AddressOf {
            dest,
            width,
            address,
        } if width.get() == 3 && address.index.is_none() && address.displacement.get() == 0 => {
            let Mir65816AddressBase::Indirect(Mir65816Value::Temp(source, width)) = &address.base
            else {
                return None;
            };
            if width.get() != 3 {
                return None;
            }
            (*dest, *source, None)
        }
        _ => return None,
    };
    let ty = |id| {
        routine
            .temps
            .iter()
            .find(|(t, _)| *t == id)
            .map(|(_, ty)| ty)
    };
    let (input, output) = (ty(source)?, ty(dest)?);
    let allowed = match kind {
        Some(NirCastKind::Pointer) => pointer_type(input) && pointer_type(output),
        Some(NirCastKind::PointerToInteger) => pointer_type(input) && address_type(output),
        Some(NirCastKind::IntegerToPointer) => address_type(input) && pointer_type(output),
        None => pointer_type(input) && (pointer_type(output) || address_type(output)),
        _ => false,
    };
    allowed.then_some((dest, source))
}

/// The whitelist is also the scratch contract: these operations use A/Y/flags,
/// their declared homes and the declared memory address only. Identities emit
/// nothing. No PTR/RESULT/arithmetic scratch is available while residents exist.
fn leaf_intervals(routine: &Mir65816Routine) -> Option<PointerRanges> {
    let [block] = routine.blocks.as_slice() else {
        return None;
    };
    if !block.params.is_empty()
        || block.ops.len() > 64
        || routine.temps.is_empty()
        || !matches!(
            block.terminator,
            Mir65816Terminator::Return { value: None, .. }
        )
        || routine
            .temps
            .iter()
            .any(|(_, ty)| !pointer_type(ty) && !address_type(ty))
    {
        return None;
    }
    let mut plan = PointerRanges {
        roots: BTreeMap::new(),
        ranges: BTreeMap::new(),
    };
    fn use_value(value: &Mir65816Value, at: usize, plan: &mut PointerRanges) -> Option<()> {
        let Mir65816Value::Temp(id, width) = value else {
            return None;
        };
        if width.get() != 3 {
            return None;
        }
        let root = plan.roots.get(id)?;
        plan.ranges.get_mut(root)?.last = at;
        Some(())
    }
    for (at, op) in block.ops.iter().enumerate() {
        if let Some((dest, source)) = pointer_identity(routine, op) {
            // Identity extension is profitable after private storage promotion.
            // Retain the stack path's borrowed/direct-local transfers while
            // invocation objects remain; the original load/store profile keeps
            // its existing admission and schedules.
            if routine.frame.extent.get() != 0 {
                return None;
            }
            use_value(
                &Mir65816Value::Temp(source, ByteSize::new(3)),
                at,
                &mut plan,
            )?;
            if plan.roots.insert(dest, plan.roots[&source]).is_some() {
                return None;
            }
            continue;
        }
        let (address, dest) = match op {
            Mir65816Op::Load {
                dest,
                width,
                address,
                volatile: false,
            } if width.get() == 3
                && routine
                    .temps
                    .iter()
                    .any(|(id, ty)| id == dest && pointer_type(ty)) =>
            {
                (address, Some(*dest))
            }
            Mir65816Op::Store {
                address,
                value,
                width,
                volatile: false,
            } if width.get() == 3 => {
                use_value(value, at, &mut plan)?;
                (address, None)
            }
            _ => return None,
        };
        if address.index.is_some() || address.displacement.get() > u32::from(u16::MAX) - 3 {
            return None;
        }
        match &address.base {
            Mir65816AddressBase::Indirect(value) => use_value(value, at, &mut plan)?,
            Mir65816AddressBase::AutomaticFrame(_)
            | Mir65816AddressBase::Parameter(_)
            | Mir65816AddressBase::External(_)
            | Mir65816AddressBase::Static(NirStorageId::Global(_)) => {}
            _ => return None,
        }
        if let Some(id) = dest {
            if plan.roots.insert(id, id).is_some() {
                return None;
            }
            plan.ranges.insert(
                id,
                Interval {
                    first: at,
                    last: at,
                },
            );
        }
    }
    if plan.roots.len() != routine.temps.len()
        || routine
            .temps
            .iter()
            .any(|(id, _)| !plan.roots.contains_key(id))
    {
        return None;
    }
    Some(plan)
}

/// The only whole-operation lifetime exception: an indirect three-byte load
/// consumes its dying base completely before writing the replacement home.
/// leaf_intervals already excludes calls, volatile accesses, indexes,
/// unrelated scalar values, CFG joins and results. In that closed whitelist no value lives in X.
fn reload_bases(routine: &Mir65816Routine, plan: &PointerRanges) -> BTreeMap<TempId, TempId> {
    routine.blocks[0]
        .ops
        .iter()
        .enumerate()
        .filter_map(|(at, op)| match op {
            Mir65816Op::Load {
                dest,
                address:
                    Mir65816Address {
                        base: Mir65816AddressBase::Indirect(Mir65816Value::Temp(base, _)),
                        ..
                    },
                ..
            } if plan.ranges[&plan.roots[base]].last == at && base != dest => {
                Some((*dest, plan.roots[base]))
            }
            _ => None,
        })
        .collect()
}

fn pointer_homes(
    ranges: &BTreeMap<TempId, Interval>,
    reloads: &BTreeMap<TempId, TempId>,
) -> Option<BTreeMap<TempId, Location>> {
    let mut ordered = ranges.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|(id, range)| (range.first, **id));
    let mut occupied = [None; 3];
    let mut temps = BTreeMap::new();
    for (&id, range) in ordered {
        let index = occupied
            .iter()
            .position(|entry: &Option<(TempId, usize)>| {
                entry.is_none_or(|(_, last)| last < range.first)
            })
            .or_else(|| {
                let base = reloads.get(&id)?;
                occupied
                    .iter()
                    .position(|entry| *entry == Some((*base, range.first)))
            })?;
        occupied[index] = Some((id, range.last));
        temps.insert(
            id,
            Location::DirectPage(Slot {
                offset: POINTER_SLOTS[index],
                width: 3,
            }),
        );
    }
    Some(temps)
}

impl AllocatedFrame {
    /// Check the exact selected load and the entire bounded allocation before
    /// allowing the low-word X handoff. Numeric coincident homes are insufficient.
    pub(super) fn pointer_reload(
        &self,
        routine: &Mir65816Routine,
        op: &Mir65816Op,
    ) -> Result<bool, String> {
        let Mir65816Op::Load {
            dest,
            address:
                Mir65816Address {
                    base: Mir65816AddressBase::Indirect(Mir65816Value::Temp(base, _)),
                    ..
                },
            ..
        } = op
        else {
            return Ok(false);
        };
        let Some(home @ Location::DirectPage(_)) = self.temps.get(dest) else {
            return Ok(false);
        };
        if self.temps.get(base) != Some(home) {
            return Ok(false);
        }
        self.verify_pointer_leaf(routine)?;
        let ranges = leaf_intervals(routine).ok_or("pointer reload outside bounded leaf")?;
        if reload_bases(routine, &ranges).get(dest) != ranges.roots.get(base)
            || routine.blocks[0].ops.get(ranges.ranges[dest].first) != Some(op)
        {
            return Err("pointer reload does not consume its dying base".into());
        }
        Ok(true)
    }
}

impl AllocatedFrame {
    /// Plan a bounded pointer leaf from verified native MIR. None selects the
    /// complete stack strategy, including when pressure exceeds three slots.
    pub fn pointer_leaf(routine: &Mir65816Routine) -> Result<Option<Self>, String> {
        let Some(intervals) = leaf_intervals(routine) else {
            return Ok(None);
        };
        // Preserve the smaller existing sequence whenever three closed slots
        // suffice. Pay for X capture only to avoid a whole-routine stack fallback.
        let Some(temps) = pointer_homes(&intervals.ranges, &BTreeMap::new())
            .or_else(|| pointer_homes(&intervals.ranges, &reload_bases(routine, &intervals)))
        else {
            return Ok(None);
        };
        let temps = intervals
            .roots
            .iter()
            .map(|(&id, root)| (id, temps[root]))
            .collect();
        let extent = abi::stack::fixed_extent(routine.frame.extent)
            .map_err(|e| e.to_string())?
            .get() as u16;
        let frame = Self {
            extent,
            spill_bytes: 0,
            peak_below_entry: extent,
            temps,
            edge_copies: vec![],
        };
        frame.verify_pointer_leaf(routine)?;
        Ok(Some(frame))
    }

    /// Recheck all identities, types, scratch ownership and closed live ranges
    /// before selection. A final image map cannot establish these MIR proofs.
    pub fn verify_pointer_leaf(&self, routine: &Mir65816Routine) -> Result<(), String> {
        let ranges =
            leaf_intervals(routine).ok_or("unsupported operation in direct-page allocation")?;
        if self.temps.len() != ranges.roots.len()
            || !self.edge_copies.is_empty()
            || u32::from(self.extent) != routine.frame.extent.get()
            || self.spill_bytes != 0
            || self.peak_below_entry != self.extent
        {
            return Err("invalid direct-page frame accounting".into());
        }
        let reloads = reload_bases(routine, &ranges);
        for (&id, &root) in &ranges.roots {
            if self.temps.get(&id) != self.temps.get(&root) {
                return Err("pointer identity has a different physical home".into());
            }
        }
        for (&id, range) in &ranges.ranges {
            let Some(Location::DirectPage(slot)) = self.temps.get(&id) else {
                return Err("missing direct-page temporary location".into());
            };
            if slot.width != 3 || !POINTER_SLOTS.contains(&slot.offset) {
                return Err("direct-page temporary exceeds owned pointer scratch".into());
            }
            for (&other, other_range) in ranges.ranges.range(..id) {
                if self.temps[&other] == self.temps[&id]
                    && range.first <= other_range.last
                    && other_range.first <= range.last
                {
                    let reload = (range.first == other_range.last
                        && reloads.get(&id) == Some(&other))
                        || (other_range.first == range.last && reloads.get(&other) == Some(&id));
                    if !reload {
                        return Err("overlapping direct-page temporary lifetimes".into());
                    }
                }
            }
        }
        for parameter in &routine.frame.parameters {
            let Mir65816AbiHome::StackArgument { offset, size, .. } = parameter.incoming else {
                return Err("parameter has no stack home".into());
            };
            abi::stack::incoming_displacement(ByteSize::new(self.extent.into()), offset, size)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
