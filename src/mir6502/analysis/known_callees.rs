use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::dataflow::{DataflowDirection, DataflowProblem, solve_dataflow};
use crate::codegen::AddressingMode;
use crate::mir6502::analysis::cfg::MirCfg;
use crate::mir6502::analysis::effects::{MirHomeByte, classify_op};
use crate::mir6502::analysis::machine_values::{MirMachineValue, MirMachineValueAvailability};
use crate::mir6502::analysis::sites::MirSite;
use crate::mir6502::ir::{
    MirCallTarget, MirFixedZpSlot, MirGlobalBacking, MirMachineAtom, MirMachineBlock,
    MirMachineByteSelector, MirMachineItem, MirMem, MirMemoryEffect, MirMemoryRegionKind, MirOp,
    MirProgram, MirReg, MirRegisterSet, MirRoutine, MirStorageBase, MirTerminator, RoutineId,
};

/// Conservative memory-write summary for a known direct callee.
///
/// `Exact` contains byte identities definitely covering every write made by
/// the routine and its summarized callees. Any indirect, opaque, recursive,
/// unresolved, or otherwise unsupported write makes the summary `Unknown`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) enum MirKnownMemoryWrites {
    Exact(Vec<MirMem>),
    #[default]
    Unknown,
}

impl MirKnownMemoryWrites {
    #[allow(dead_code)] // Retained as the pair-level query for future address consumers.
    pub(in crate::mir6502) fn preserves_fixed_pair(&self, lo: MirFixedZpSlot) -> bool {
        let hi = MirFixedZpSlot(lo.0.saturating_add(1));
        match self {
            Self::Exact(writes) => writes.iter().all(|write| {
                !mem_may_alias_fixed_slot(write, lo) && !mem_may_alias_fixed_slot(write, hi)
            }),
            Self::Unknown => false,
        }
    }

    pub(in crate::mir6502) fn may_write_mem(&self, mem: &MirMem) -> bool {
        match self {
            Self::Exact(writes) => writes.iter().any(|write| mems_may_alias(write, mem)),
            Self::Unknown => true,
        }
    }

    pub(in crate::mir6502) fn may_write_caller_private_byte(&self, address: u16) -> bool {
        match self {
            // Named callee storage cannot name a caller-private byte. Only a
            // raw absolute access can overlap the caller's resolved frame.
            Self::Exact(writes) => writes
                .iter()
                .any(|write| matches!(write, MirMem::Absolute(candidate) if *candidate == address)),
            Self::Unknown => true,
        }
    }

    fn exact() -> Self {
        Self::Exact(Vec::new())
    }

    fn record(&mut self, mem: MirMem) {
        let Self::Exact(writes) = self else {
            return;
        };
        if !writes.contains(&mem) {
            writes.push(mem);
        }
    }

    fn merge(&mut self, other: &Self) {
        let Self::Exact(other) = other else {
            *self = Self::Unknown;
            return;
        };
        for mem in other {
            self.record(mem.clone());
        }
    }

    fn make_unknown(&mut self) {
        *self = Self::Unknown;
    }
}

/// Conservative memory-read summary for a known direct callee.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) enum MirKnownMemoryReads {
    Exact(Vec<MirMem>),
    #[default]
    Unknown,
}

impl MirKnownMemoryReads {
    #[allow(dead_code)] // Retained for fixed-memory carrier selection.
    pub(in crate::mir6502) fn may_read_caller_private_ram(&self) -> bool {
        match self {
            // Callee-local identities, globals, statics, and fixed zero page
            // cannot name another routine's private RAM. A raw absolute read
            // can alias it after layout, so it remains conservative.
            Self::Exact(reads) => reads.iter().any(|read| matches!(read, MirMem::Absolute(_))),
            Self::Unknown => true,
        }
    }

    pub(in crate::mir6502) fn may_read_caller_private_byte(&self, address: u16) -> bool {
        match self {
            // Named callee storage cannot name a caller-private byte. Only a
            // raw absolute access can overlap the caller's resolved frame.
            Self::Exact(reads) => reads
                .iter()
                .any(|read| matches!(read, MirMem::Absolute(candidate) if *candidate == address)),
            Self::Unknown => true,
        }
    }

    fn exact() -> Self {
        Self::Exact(Vec::new())
    }

    fn record(&mut self, mem: MirMem) {
        let Self::Exact(reads) = self else {
            return;
        };
        if !reads.contains(&mem) {
            reads.push(mem);
        }
    }

    fn merge(&mut self, other: &Self) {
        let Self::Exact(other) = other else {
            *self = Self::Unknown;
            return;
        };
        for mem in other {
            self.record(mem.clone());
        }
    }

    fn make_unknown(&mut self) {
        *self = Self::Unknown;
    }
}

/// Machine state proven on every reachable return from a known routine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirKnownCalleeExitSummary {
    accumulator: Option<MirMachineValue>,
    /// Exact byte provenance that establishes both Z and N on every reachable
    /// return. `None` means either unknown or inconsistent across exits.
    zn: Option<MirMachineValue>,
    /// Registers whose incoming values survive every reachable return.
    ///
    /// The first consumers use X/Y. Keeping this as a machine register set
    /// leaves room for later A/flags/SP contracts without changing NIR.
    preserves: MirRegisterSet,
    reads: MirKnownMemoryReads,
    writes: MirKnownMemoryWrites,
}

impl MirKnownCalleeExitSummary {
    pub(in crate::mir6502) fn accumulator(&self) -> Option<&MirMachineValue> {
        self.accumulator.as_ref()
    }

    pub(in crate::mir6502) fn zn(&self) -> Option<&MirMachineValue> {
        self.zn.as_ref()
    }

    pub(in crate::mir6502) fn writes(&self) -> &MirKnownMemoryWrites {
        &self.writes
    }

    pub(in crate::mir6502) fn reads(&self) -> &MirKnownMemoryReads {
        &self.reads
    }

    pub(in crate::mir6502) fn preserves_register(&self, register: MirReg) -> bool {
        match register {
            MirReg::A => self.preserves.a,
            MirReg::X => self.preserves.x,
            MirReg::Y => self.preserves.y,
        }
    }

    #[allow(dead_code)] // Retained as the pair-level query for future address consumers.
    pub(in crate::mir6502) fn preserves_fixed_pair(&self, lo: MirFixedZpSlot) -> bool {
        self.writes.preserves_fixed_pair(lo)
    }
}

/// Program-wide summaries for direct MIR routine calls.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::mir6502) struct MirKnownCalleeSummaries {
    routines: BTreeMap<RoutineId, MirKnownCalleeExitSummary>,
    direct_call_targets: BTreeSet<RoutineId>,
}

impl MirKnownCalleeSummaries {
    pub(in crate::mir6502) fn analyze(program: &MirProgram) -> Self {
        let direct_call_targets = program
            .routines
            .iter()
            .flat_map(|routine| &routine.blocks)
            .flat_map(|block| &block.ops)
            .filter_map(|op| match op {
                MirOp::Call {
                    target: MirCallTarget::Routine(routine),
                    ..
                } => Some(*routine),
                _ => None,
            })
            .collect();
        let machine_summaries = program
            .routines
            .iter()
            .filter_map(|routine| {
                summarize_machine_routine(routine, program).map(|summary| (routine.id, summary))
            })
            .collect::<BTreeMap<_, _>>();
        let mut summaries = Self {
            routines: program
                .routines
                .iter()
                .map(|routine| {
                    (
                        routine.id,
                        machine_summaries
                            .get(&routine.id)
                            .cloned()
                            .unwrap_or_default(),
                    )
                })
                .collect(),
            direct_call_targets,
        };

        // Unknown is the conservative fixed point for recursion. Acyclic
        // callers become more precise as their callees acquire summaries.
        for _ in 0..=program.routines.len() {
            let mut changed = false;
            let previous = summaries.clone();
            for routine in &program.routines {
                if machine_summaries.contains_key(&routine.id) {
                    continue;
                }
                let summary = summarize_mir_routine(routine, &previous);
                if summaries.routines.get(&routine.id) != Some(&summary) {
                    summaries.routines.insert(routine.id, summary);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        summaries
    }

    pub(in crate::mir6502) fn get(&self, routine: RoutineId) -> Option<&MirKnownCalleeExitSummary> {
        self.routines.get(&routine)
    }

    pub(in crate::mir6502) fn for_target(
        &self,
        target: &MirCallTarget,
    ) -> Option<&MirKnownCalleeExitSummary> {
        let MirCallTarget::Routine(routine) = target else {
            return None;
        };
        self.get(*routine)
    }

    pub(in crate::mir6502) fn accumulator_summary_is_observable(&self, routine: RoutineId) -> bool {
        self.direct_call_targets.contains(&routine)
            && self
                .get(routine)
                .and_then(MirKnownCalleeExitSummary::accumulator)
                .is_some()
    }
}

fn summarize_mir_routine(
    routine: &MirRoutine,
    callees: &MirKnownCalleeSummaries,
) -> MirKnownCalleeExitSummary {
    let Ok(cfg) = MirCfg::from_routine(routine) else {
        return MirKnownCalleeExitSummary::default();
    };
    let values = MirMachineValueAvailability::analyze_with_known_callees(routine, &cfg, callees);
    let preservation = analyze_register_preservation(routine, &cfg, callees);
    let mut accumulator = None;
    let mut zn = None;
    let mut preserves = MirRegisterSet {
        x: true,
        y: true,
        ..MirRegisterSet::default()
    };
    let mut saw_return = false;
    for block in &routine.blocks {
        if !matches!(block.terminator, MirTerminator::Return)
            || !values
                .block_by_id(block.id)
                .is_some_and(|facts| facts.reachable)
        {
            continue;
        }
        let site = MirSite::Terminator { block: block.id };
        let (Ok(accumulator_value), Ok(zn_value)) =
            (values.accumulator_at(site), values.zn_value_at(site))
        else {
            return MirKnownCalleeExitSummary::default();
        };
        let Some(preserved_at_return) = preservation.get(&block.id) else {
            return MirKnownCalleeExitSummary::default();
        };
        preserves.x &= preserved_at_return.x;
        preserves.y &= preserved_at_return.y;
        if !saw_return {
            accumulator = accumulator_value;
            zn = zn_value;
            saw_return = true;
        } else {
            if accumulator != accumulator_value {
                accumulator = None;
            }
            if zn != zn_value {
                zn = None;
            }
        }
    }

    MirKnownCalleeExitSummary {
        accumulator: saw_return.then_some(accumulator).flatten(),
        zn: saw_return
            .then_some(zn)
            .flatten()
            .and_then(caller_visible_exit_value),
        preserves: if saw_return {
            preserves
        } else {
            MirRegisterSet::default()
        },
        reads: summarize_routine_reads(routine, callees),
        writes: summarize_routine_writes(routine, callees),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct MirRegisterPreservationState {
    reachable: bool,
    x: bool,
    y: bool,
}

impl MirRegisterPreservationState {
    fn entry() -> Self {
        Self {
            reachable: true,
            x: true,
            y: true,
        }
    }

    fn meet_with(&mut self, other: &Self) {
        if !other.reachable {
            return;
        }
        if !self.reachable {
            *self = *other;
            return;
        }
        self.x &= other.x;
        self.y &= other.y;
    }
}

struct MirRegisterPreservationProblem<'a> {
    routine: &'a MirRoutine,
    entry: Option<crate::mir6502::ir::MirBlockId>,
    callees: &'a MirKnownCalleeSummaries,
}

impl DataflowProblem<MirCfg> for MirRegisterPreservationProblem<'_> {
    type State = MirRegisterPreservationState;

    fn direction(&self) -> DataflowDirection {
        DataflowDirection::Forward
    }

    fn bottom(&self) -> Self::State {
        Self::State::default()
    }

    fn boundary(&self, node: crate::mir6502::ir::MirBlockId) -> Option<Self::State> {
        (Some(node) == self.entry).then(MirRegisterPreservationState::entry)
    }

    fn join(&self, into: &mut Self::State, other: &Self::State) {
        into.meet_with(other);
    }

    fn transfer(&self, node: crate::mir6502::ir::MirBlockId, input: &Self::State) -> Self::State {
        if !input.reachable {
            return *input;
        }
        let Some(block) = self.routine.blocks.iter().find(|block| block.id == node) else {
            return *input;
        };
        let mut output = *input;
        for op in &block.ops {
            let effects = classify_op(op);
            let known = match op {
                MirOp::Call { target, .. } => self.callees.for_target(target),
                _ => None,
            };
            if effects.may_clobber_reg_compat(MirReg::X)
                && !known.is_some_and(|summary| summary.preserves_register(MirReg::X))
            {
                output.x = false;
            }
            if effects.may_clobber_reg_compat(MirReg::Y)
                && !known.is_some_and(|summary| summary.preserves_register(MirReg::Y))
            {
                output.y = false;
            }
        }
        output
    }
}

fn analyze_register_preservation(
    routine: &MirRoutine,
    cfg: &MirCfg,
    callees: &MirKnownCalleeSummaries,
) -> BTreeMap<crate::mir6502::ir::MirBlockId, MirRegisterPreservationState> {
    let result = solve_dataflow(
        cfg,
        &MirRegisterPreservationProblem {
            routine,
            entry: cfg.entry(),
            callees,
        },
    );
    routine
        .blocks
        .iter()
        .filter_map(|block| {
            result
                .out_state(block.id)
                .copied()
                .map(|state| (block.id, state))
        })
        .collect()
}

fn caller_visible_exit_value(value: MirMachineValue) -> Option<MirMachineValue> {
    match value {
        value @ MirMachineValue::ConstU8(_) => Some(value),
        value @ MirMachineValue::DirectMem(
            MirMem::Absolute(_)
            | MirMem::Static { .. }
            | MirMem::Global { .. }
            | MirMem::FixedZeroPage(_),
        ) => Some(value),
        MirMachineValue::DirectMem(
            MirMem::Local { .. }
            | MirMem::Param { .. }
            | MirMem::Spill { .. }
            | MirMem::ZeroPage(_),
        ) => None,
    }
}

fn summarize_routine_reads(
    routine: &MirRoutine,
    callees: &MirKnownCalleeSummaries,
) -> MirKnownMemoryReads {
    let mut reads = MirKnownMemoryReads::exact();
    for block in &routine.blocks {
        for op in &block.ops {
            summarize_op_reads(op, callees, &mut reads);
            if matches!(reads, MirKnownMemoryReads::Unknown) {
                return reads;
            }
        }
    }
    reads
}

fn summarize_op_reads(
    op: &MirOp,
    callees: &MirKnownCalleeSummaries,
    reads: &mut MirKnownMemoryReads,
) {
    match op {
        MirOp::Call {
            target, effects, ..
        } => {
            if let Some(summary) = callees.for_target(target) {
                reads.merge(summary.reads());
            } else {
                summarize_structured_reads(&effects.memory_reads, effects.opaque, reads);
            }
            return;
        }
        MirOp::RuntimeHelper { effects, .. } => {
            summarize_structured_reads(&effects.memory_reads, effects.opaque, reads);
            return;
        }
        MirOp::MachineBlock { .. } | MirOp::Barrier { .. } => {
            reads.make_unknown();
            return;
        }
        _ => {}
    }

    let effects = classify_op(op);
    if effects.memory.indirect_reads
        || effects.memory.opaque
        || effects.memory.has_unknown_effects
        || effects.homes.unknown_reads
    {
        reads.make_unknown();
        return;
    }
    summarize_structured_reads(&effects.memory.structured_reads, false, reads);
    if matches!(reads, MirKnownMemoryReads::Unknown) {
        return;
    }
    for range in effects.memory.direct_reads {
        for offset in 0..range.bytes {
            reads.record(offset_mem(&range.base, offset));
        }
    }
    for home in effects.addresses.pair_reads {
        if let MirHomeByte::FixedZeroPage(slot) = home {
            reads.record(MirMem::FixedZeroPage(slot));
        }
    }
}

fn summarize_structured_reads(
    effect: &MirMemoryEffect,
    opaque: bool,
    reads: &mut MirKnownMemoryReads,
) {
    if opaque {
        reads.make_unknown();
        return;
    }
    match effect {
        MirMemoryEffect::None => {}
        MirMemoryEffect::Unknown | MirMemoryEffect::All => reads.make_unknown(),
        MirMemoryEffect::Regions(regions) => {
            for region in regions {
                let base = match region.kind {
                    MirMemoryRegionKind::Global(id) => MirMem::Global { id, offset: 0 },
                    MirMemoryRegionKind::Static(id) => MirMem::Static { id, offset: 0 },
                    MirMemoryRegionKind::Local(id) => MirMem::Local { id, offset: 0 },
                    MirMemoryRegionKind::Param(id) => MirMem::Param { id, offset: 0 },
                    MirMemoryRegionKind::AbsoluteRange => MirMem::Absolute(region.offset),
                    MirMemoryRegionKind::ZeroPage => {
                        let Ok(slot) = u8::try_from(region.offset) else {
                            reads.make_unknown();
                            return;
                        };
                        MirMem::FixedZeroPage(MirFixedZpSlot(slot))
                    }
                    MirMemoryRegionKind::Stack => {
                        reads.make_unknown();
                        return;
                    }
                };
                for offset in 0..region.size {
                    reads.record(offset_mem(&base, offset));
                }
            }
        }
    }
}

fn summarize_routine_writes(
    routine: &MirRoutine,
    callees: &MirKnownCalleeSummaries,
) -> MirKnownMemoryWrites {
    let mut writes = MirKnownMemoryWrites::exact();
    for block in &routine.blocks {
        for op in &block.ops {
            summarize_op_writes(op, routine, callees, &mut writes);
            if matches!(writes, MirKnownMemoryWrites::Unknown) {
                return writes;
            }
        }
    }
    writes
}

fn summarize_op_writes(
    op: &MirOp,
    routine: &MirRoutine,
    callees: &MirKnownCalleeSummaries,
    writes: &mut MirKnownMemoryWrites,
) {
    match op {
        MirOp::Call {
            target, effects, ..
        } => {
            if let Some(summary) = callees.for_target(target) {
                writes.merge(summary.writes());
            } else {
                summarize_structured_writes(&effects.memory_writes, effects.opaque, writes);
            }
            return;
        }
        MirOp::RuntimeHelper { effects, .. } => {
            summarize_structured_writes(&effects.memory_writes, effects.opaque, writes);
            return;
        }
        MirOp::MachineBlock { .. }
        | MirOp::Barrier { .. }
        | MirOp::StoreIndirect { .. }
        | MirOp::CopyIndirectWord { .. }
        | MirOp::CopyDirectWordToIndirect { .. }
        | MirOp::IndirectByteCompound { .. }
        | MirOp::IndirectWordCompound { .. }
        | MirOp::UpdateIndexedMem { .. } => {
            writes.make_unknown();
            return;
        }
        _ => {}
    }

    let effects = classify_op(op);
    if effects.memory.indirect_writes
        || effects.memory.opaque
        || effects.memory.may_write_any
        || effects.memory.has_unknown_effects
        || effects.homes.unknown_writes
    {
        writes.make_unknown();
        return;
    }
    for range in effects.memory.direct_writes {
        for offset in 0..range.bytes {
            writes.record(resolve_routine_private_zero_page(
                routine,
                offset_mem(&range.base, offset),
            ));
        }
    }
    for home in effects.addresses.pair_writes {
        match home {
            MirHomeByte::FixedZeroPage(slot) => {
                writes.record(MirMem::FixedZeroPage(slot));
            }
            MirHomeByte::VirtualZeroPage(slot) => {
                writes.record(resolve_routine_private_zero_page(
                    routine,
                    MirMem::ZeroPage(slot),
                ));
            }
            MirHomeByte::Spill { .. } => {}
        }
    }
}

fn resolve_routine_private_zero_page(routine: &MirRoutine, mem: MirMem) -> MirMem {
    let MirMem::ZeroPage(slot) = mem else {
        return mem;
    };
    routine
        .frame
        .zero_page_allocations
        .iter()
        .find(|allocation| allocation.slot == slot && allocation.size == 1)
        .map_or(MirMem::ZeroPage(slot), |allocation| {
            MirMem::FixedZeroPage(allocation.start)
        })
}

fn summarize_structured_writes(
    effect: &MirMemoryEffect,
    opaque: bool,
    writes: &mut MirKnownMemoryWrites,
) {
    if opaque {
        writes.make_unknown();
        return;
    }
    match effect {
        MirMemoryEffect::None => {}
        MirMemoryEffect::Unknown | MirMemoryEffect::All => writes.make_unknown(),
        MirMemoryEffect::Regions(regions) => {
            for region in regions {
                let base = match region.kind {
                    MirMemoryRegionKind::Global(id) => MirMem::Global { id, offset: 0 },
                    MirMemoryRegionKind::Static(id) => MirMem::Static { id, offset: 0 },
                    MirMemoryRegionKind::Local(id) => MirMem::Local { id, offset: 0 },
                    MirMemoryRegionKind::Param(id) => MirMem::Param { id, offset: 0 },
                    MirMemoryRegionKind::AbsoluteRange => MirMem::Absolute(region.offset),
                    MirMemoryRegionKind::ZeroPage => {
                        let Ok(slot) = u8::try_from(region.offset) else {
                            writes.make_unknown();
                            return;
                        };
                        MirMem::FixedZeroPage(MirFixedZpSlot(slot))
                    }
                    MirMemoryRegionKind::Stack => {
                        writes.make_unknown();
                        return;
                    }
                };
                for offset in 0..region.size {
                    writes.record(offset_mem(&base, offset));
                }
            }
        }
    }
}

fn summarize_machine_routine(
    routine: &MirRoutine,
    program: &MirProgram,
) -> Option<MirKnownCalleeExitSummary> {
    let [block] = routine.blocks.as_slice() else {
        return None;
    };
    let [MirOp::MachineBlock { id, .. }] = block.ops.as_slice() else {
        return None;
    };
    if !matches!(block.terminator, MirTerminator::Unreachable) {
        return None;
    }
    let machine = program
        .machine_blocks
        .iter()
        .find(|machine| machine.id == *id)?;
    summarize_bounded_machine_block(machine, routine, program)
}

#[derive(Debug, Clone)]
struct MachineByte {
    value: Option<u8>,
    memory: Option<MirMem>,
}

fn summarize_bounded_machine_block(
    machine: &MirMachineBlock,
    routine: &MirRoutine,
    program: &MirProgram,
) -> Option<MirKnownCalleeExitSummary> {
    let bytes = flatten_machine_items(&machine.items, routine, program)?;
    let instruction_starts = machine_instruction_starts(&bytes)?;
    let mut index = 0usize;
    let mut accumulator = None;
    let mut zn = None;
    let mut zn_register = None;
    let mut has_forward_branch = false;
    let mut preserves = MirRegisterSet {
        x: true,
        y: true,
        ..MirRegisterSet::default()
    };
    let mut reads = MirKnownMemoryReads::exact();
    let mut writes = MirKnownMemoryWrites::exact();
    while index < bytes.len() {
        let opcode = bytes[index].value?;
        let (mnemonic, mode, len) = crate::codegen::decode_6502_opcode(opcode)?;
        if index + len > bytes.len() {
            return None;
        }
        let operands = &bytes[index + 1..index + len];
        summarize_machine_instruction_reads(mnemonic, mode, operands, &mut reads);
        match opcode {
            0x60 if index + len == bytes.len() => {
                return Some(MirKnownCalleeExitSummary {
                    // The effect walk deliberately visits both sides of a
                    // forward branch by linearizing their union. That is exact
                    // for reads, writes, and register preservation, but not
                    // for a path-sensitive final A or Z/N value.
                    accumulator: if has_forward_branch {
                        None
                    } else {
                        accumulator
                    },
                    zn: if has_forward_branch { None } else { zn },
                    preserves,
                    reads,
                    writes,
                });
            }
            0x10 | 0x30 | 0x50 | 0x70 | 0x90 | 0xB0 | 0xD0 | 0xF0 => {
                let displacement = i8::from_ne_bytes([operands.first()?.value?]);
                let next = index.saturating_add(len);
                let target = isize::try_from(next)
                    .ok()?
                    .checked_add(isize::from(displacement))
                    .and_then(|target| usize::try_from(target).ok())?;
                // This bounded summary supports only structured forward
                // branches which rejoin before the single terminal RTS.
                // Linearizing both arms then safely over-approximates their
                // memory effects and index-register writes.
                if target <= next || target >= bytes.len() || !instruction_starts.contains(&target)
                {
                    return None;
                }
                has_forward_branch = true;
                accumulator = None;
                zn = None;
                zn_register = None;
            }
            0x00 | 0x60 | 0x40 | 0x4C | 0x6C => {
                return None;
            }
            0x20 => {
                accumulator = None;
                zn = None;
                zn_register = None;
                preserves.x = false;
                preserves.y = false;
                reads.make_unknown();
                let target = known_u16(operands);
                match target {
                    // Existing trusted OS-effect table: neither call writes
                    // zero page. CIOV writes only $0340-$03BF.
                    Some(0xF2F8) => {}
                    Some(0xE456) => {
                        for address in 0x0340..0x03C0 {
                            writes.record(MirMem::Absolute(address));
                        }
                    }
                    _ => writes.make_unknown(),
                }
            }
            0x85 => {
                let mem = machine_operand_mem(operands)?;
                record_machine_register_store(
                    &mut writes,
                    &mut accumulator,
                    &mut zn,
                    &mut zn_register,
                    MirReg::A,
                    mem,
                );
            }
            0x8D => {
                let mem = machine_operand_mem(operands)?;
                record_machine_register_store(
                    &mut writes,
                    &mut accumulator,
                    &mut zn,
                    &mut zn_register,
                    MirReg::A,
                    mem,
                );
            }
            0x81 | 0x91 | 0x95 | 0x99 | 0x9D => {
                invalidate_accumulator_after_unknown_write(&mut accumulator);
                invalidate_zn_after_unknown_write(&mut zn);
                writes.make_unknown();
            }
            0x84 | 0x86 => {
                record_machine_register_store(
                    &mut writes,
                    &mut accumulator,
                    &mut zn,
                    &mut zn_register,
                    if opcode == 0x84 { MirReg::Y } else { MirReg::X },
                    machine_operand_mem(operands)?,
                );
            }
            0x8C | 0x8E => {
                record_machine_register_store(
                    &mut writes,
                    &mut accumulator,
                    &mut zn,
                    &mut zn_register,
                    if opcode == 0x8C { MirReg::Y } else { MirReg::X },
                    machine_operand_mem(operands)?,
                );
            }
            0x94 | 0x96 => {
                invalidate_accumulator_after_unknown_write(&mut accumulator);
                invalidate_zn_after_unknown_write(&mut zn);
                writes.make_unknown();
            }
            0x06 | 0x26 | 0x46 | 0x66 | 0xC6 | 0xE6 => {
                let mem = machine_operand_mem(operands)?;
                record_machine_write(&mut writes, &mut accumulator, &mut zn, mem.clone());
                zn = Some(MirMachineValue::DirectMem(mem));
                zn_register = None;
            }
            0x0E | 0x2E | 0x4E | 0x6E | 0xCE | 0xEE => {
                let mem = machine_operand_mem(operands)?;
                record_machine_write(&mut writes, &mut accumulator, &mut zn, mem.clone());
                zn = Some(MirMachineValue::DirectMem(mem));
                zn_register = None;
            }
            0x16 | 0x1E | 0x36 | 0x3E | 0x56 | 0x5E | 0x76 | 0x7E | 0xD6 | 0xDE | 0xF6 | 0xFE => {
                invalidate_accumulator_after_unknown_write(&mut accumulator);
                invalidate_zn_after_unknown_write(&mut zn);
                zn = None;
                zn_register = None;
                writes.make_unknown();
            }
            0xA9 => {
                let value = MirMachineValue::ConstU8(operands.first()?.value?);
                accumulator = Some(value.clone());
                zn = Some(value);
                zn_register = Some(MirReg::A);
            }
            0xA5 => {
                let value = MirMachineValue::DirectMem(machine_operand_mem(operands)?);
                accumulator = Some(value.clone());
                zn = Some(value);
                zn_register = Some(MirReg::A);
            }
            0xAD => {
                let value = MirMachineValue::DirectMem(machine_operand_mem(operands)?);
                accumulator = Some(value.clone());
                zn = Some(value);
                zn_register = Some(MirReg::A);
            }
            0xA1 | 0xB1 | 0xB5 | 0xB9 | 0xBD => {
                accumulator = None;
                zn = None;
                zn_register = Some(MirReg::A);
            }
            0xA2 | 0xA0 => {
                if opcode == 0xA2 {
                    preserves.x = false;
                } else {
                    preserves.y = false;
                }
                zn = Some(MirMachineValue::ConstU8(operands.first()?.value?));
                zn_register = Some(if opcode == 0xA2 { MirReg::X } else { MirReg::Y });
            }
            0xA6 | 0xA4 => {
                if opcode == 0xA6 {
                    preserves.x = false;
                } else {
                    preserves.y = false;
                }
                zn = Some(MirMachineValue::DirectMem(machine_operand_mem(operands)?));
                zn_register = Some(if opcode == 0xA6 { MirReg::X } else { MirReg::Y });
            }
            0xAE | 0xAC => {
                if opcode == 0xAE {
                    preserves.x = false;
                } else {
                    preserves.y = false;
                }
                zn = Some(MirMachineValue::DirectMem(machine_operand_mem(operands)?));
                zn_register = Some(if opcode == 0xAE { MirReg::X } else { MirReg::Y });
            }
            0xB6 | 0xBE | 0xB4 | 0xBC => {
                if matches!(opcode, 0xB6 | 0xBE) {
                    preserves.x = false;
                } else {
                    preserves.y = false;
                }
                zn = None;
                zn_register = Some(if matches!(opcode, 0xB6 | 0xBE) {
                    MirReg::X
                } else {
                    MirReg::Y
                });
            }
            0xAA | 0xA8 => {
                if opcode == 0xAA {
                    preserves.x = false;
                } else {
                    preserves.y = false;
                }
                zn = accumulator.clone();
                zn_register = Some(if opcode == 0xAA { MirReg::X } else { MirReg::Y });
            }
            0x88 | 0xC8 => {
                preserves.y = false;
                zn = None;
                zn_register = Some(MirReg::Y);
            }
            0xCA | 0xE8 | 0xBA => {
                preserves.x = false;
                zn = None;
                zn_register = Some(MirReg::X);
            }
            opcode if opcode_changes_accumulator(opcode) => {
                accumulator = None;
                zn = None;
                zn_register = Some(MirReg::A);
            }
            opcode if opcode_changes_zn(opcode) => {
                zn = None;
                zn_register = None;
            }
            _ => {}
        }
        index += len;
    }
    None
}

fn machine_instruction_starts(bytes: &[MachineByte]) -> Option<BTreeSet<usize>> {
    let mut starts = BTreeSet::new();
    let mut index = 0usize;
    while index < bytes.len() {
        starts.insert(index);
        let opcode = bytes[index].value?;
        let (_, _, len) = crate::codegen::decode_6502_opcode(opcode)?;
        if len == 0 || index.saturating_add(len) > bytes.len() {
            return None;
        }
        index = index.saturating_add(len);
    }
    (index == bytes.len()).then_some(starts)
}

fn flatten_machine_items(
    items: &[MirMachineItem],
    routine: &MirRoutine,
    program: &MirProgram,
) -> Option<Vec<MachineByte>> {
    let mut bytes = Vec::new();
    let mut item_index = 0usize;
    while item_index < items.len() {
        let MirMachineItem::Byte(opcode) = &items[item_index] else {
            return None;
        };
        let (_, _, instruction_len) = crate::codegen::decode_6502_opcode(*opcode)?;
        bytes.push(MachineByte {
            value: Some(*opcode),
            memory: None,
        });
        item_index += 1;

        let mut remaining = instruction_len.saturating_sub(1);
        while remaining > 0 {
            let item = items.get(item_index)?;
            let expanded = expand_machine_operand(item, remaining, routine, program)?;
            if expanded.is_empty() || expanded.len() > remaining {
                return None;
            }
            remaining -= expanded.len();
            bytes.extend(expanded);
            item_index += 1;
        }
    }
    Some(bytes)
}

/// Return whether execution can continue into the bytes immediately following
/// a structured machine block. Unknown encodings are conservative: retaining
/// the following runtime routine is safer than truncating a legacy fallthrough
/// implementation.
pub(in crate::mir6502) fn machine_block_falls_through(machine: &MirMachineBlock) -> bool {
    let Some(last) = machine.items.last() else {
        return true;
    };
    if matches!(machine_item_last_byte(last), Some(0x60 | 0x40)) {
        return false;
    }
    let trailing_address = matches!(
        last,
        MirMachineItem::Name(_)
            | MirMachineItem::AddressExpr { selector: None, .. }
            | MirMachineItem::Relocation {
                kind: crate::asm6502::InlineAsmRelocationKind::Absolute16,
                ..
            }
    );
    if trailing_address
        && machine
            .items
            .get(machine.items.len().saturating_sub(2))
            .and_then(machine_item_last_byte)
            .is_some_and(|opcode| matches!(opcode, 0x4C | 0x6C))
    {
        return false;
    }
    true
}

fn machine_item_last_byte(item: &MirMachineItem) -> Option<u8> {
    match item {
        MirMachineItem::Byte(value) => Some(*value),
        MirMachineItem::Word(value) => Some((value >> 8) as u8),
        MirMachineItem::CharLiteral(value) => u8::try_from(*value as u32).ok(),
        MirMachineItem::StringLiteral(_)
        | MirMachineItem::Name(_)
        | MirMachineItem::AddressExpr { .. }
        | MirMachineItem::AddressByte { .. }
        | MirMachineItem::Relocation { .. } => None,
    }
}

fn summarize_machine_instruction_reads(
    mnemonic: &str,
    mode: AddressingMode,
    operands: &[MachineByte],
    reads: &mut MirKnownMemoryReads,
) {
    let stores_only = matches!(mnemonic, "STA" | "STX" | "STY");
    let control_address = matches!(mnemonic, "JMP" | "JSR");
    match mode {
        AddressingMode::ZeroPage | AddressingMode::Absolute if !stores_only && !control_address => {
            if let Some(mem) = machine_operand_mem(operands) {
                reads.record(mem);
            } else {
                reads.make_unknown();
            }
        }
        AddressingMode::AbsoluteX | AddressingMode::AbsoluteY
            if !stores_only && !control_address =>
        {
            match machine_operand_mem(operands) {
                Some(mem @ (MirMem::Global { .. } | MirMem::Static { .. })) => reads.record(mem),
                _ => reads.make_unknown(),
            }
        }
        AddressingMode::ZeroPageX | AddressingMode::ZeroPageY
            if !stores_only && !control_address =>
        {
            reads.make_unknown();
        }
        AddressingMode::IndirectIndexedY => {
            if let Some(lo) = machine_operand_mem(operands) {
                reads.record(lo.clone());
                reads.record(offset_mem(&lo, 1));
            } else {
                reads.make_unknown();
            }
            if !stores_only {
                reads.make_unknown();
            }
        }
        AddressingMode::IndexedIndirectX | AddressingMode::Indirect => {
            reads.make_unknown();
        }
        AddressingMode::Implied
        | AddressingMode::Accumulator
        | AddressingMode::Immediate
        | AddressingMode::Relative
        | AddressingMode::ZeroPage
        | AddressingMode::ZeroPageX
        | AddressingMode::ZeroPageY
        | AddressingMode::Absolute
        | AddressingMode::AbsoluteX
        | AddressingMode::AbsoluteY => {}
    }
}

fn expand_machine_operand(
    item: &MirMachineItem,
    remaining: usize,
    routine: &MirRoutine,
    program: &MirProgram,
) -> Option<Vec<MachineByte>> {
    let unknown = |count: usize, memory: Option<MirMem>| {
        (0..count)
            .map(|index| MachineByte {
                value: None,
                memory: (index == 0).then(|| memory.clone()).flatten(),
            })
            .collect::<Vec<_>>()
    };
    match item {
        MirMachineItem::Byte(value) => Some(vec![MachineByte {
            value: Some(*value),
            memory: None,
        }]),
        MirMachineItem::Word(value) if remaining >= 2 => Some(vec![
            MachineByte {
                value: Some(*value as u8),
                memory: None,
            },
            MachineByte {
                value: Some((*value >> 8) as u8),
                memory: None,
            },
        ]),
        MirMachineItem::Word(_) => None,
        MirMachineItem::CharLiteral(_) => Some(unknown(1, None)),
        MirMachineItem::Name(name) => {
            let memory = resolve_machine_storage(name, 0, routine, program);
            Some(machine_address_bytes(memory, remaining))
        }
        MirMachineItem::AddressExpr {
            selector,
            atom,
            offset,
            ..
        } => {
            let memory = match atom {
                MirMachineAtom::Name(name) => {
                    resolve_machine_storage(name, *offset, routine, program)
                }
                MirMachineAtom::Number(value) => offset_address(*value, *offset).map(absolute_mem),
                MirMachineAtom::Current => None,
            };
            let numeric = match atom {
                MirMachineAtom::Number(value) => offset_address(*value, *offset),
                MirMachineAtom::Name(_) | MirMachineAtom::Current => {
                    memory.as_ref().and_then(resolved_absolute_address)
                }
            };
            match selector {
                Some(MirMachineByteSelector::Low) => Some(vec![MachineByte {
                    value: numeric.map(|value| value as u8),
                    memory: None,
                }]),
                Some(MirMachineByteSelector::High) => Some(vec![MachineByte {
                    value: numeric.map(|value| (value >> 8) as u8),
                    memory: None,
                }]),
                None => Some(machine_address_bytes(memory, remaining)),
            }
        }
        MirMachineItem::AddressByte { .. } => Some(unknown(1, None)),
        MirMachineItem::Relocation { kind, target, .. } => {
            let width = match kind {
                crate::asm6502::InlineAsmRelocationKind::Absolute16 => 2,
                crate::asm6502::InlineAsmRelocationKind::Byte8
                | crate::asm6502::InlineAsmRelocationKind::Low8
                | crate::asm6502::InlineAsmRelocationKind::High8 => 1,
            };
            let memory = match target {
                crate::mir6502::ir::MirInlineAsmTarget::Memory(memory) => Some(memory.clone()),
                crate::mir6502::ir::MirInlineAsmTarget::Absolute(address) => {
                    Some(absolute_mem(*address))
                }
                crate::mir6502::ir::MirInlineAsmTarget::Routine(_)
                | crate::mir6502::ir::MirInlineAsmTarget::InlineOffset(_) => None,
            };
            let value = match target {
                crate::mir6502::ir::MirInlineAsmTarget::Absolute(address) => Some(*address),
                crate::mir6502::ir::MirInlineAsmTarget::Memory(memory) => {
                    resolved_absolute_address(memory)
                }
                crate::mir6502::ir::MirInlineAsmTarget::Routine(_)
                | crate::mir6502::ir::MirInlineAsmTarget::InlineOffset(_) => None,
            };
            Some(
                (0..width)
                    .map(|index| {
                        let shift = match kind {
                            crate::asm6502::InlineAsmRelocationKind::High8 => 8,
                            crate::asm6502::InlineAsmRelocationKind::Absolute16 => index * 8,
                            crate::asm6502::InlineAsmRelocationKind::Byte8
                            | crate::asm6502::InlineAsmRelocationKind::Low8 => 0,
                        };
                        MachineByte {
                            value: value.map(|value| (value >> shift) as u8),
                            memory: (index == 0).then(|| memory.clone()).flatten(),
                        }
                    })
                    .collect(),
            )
        }
        MirMachineItem::StringLiteral(_) => None,
    }
}

fn machine_address_bytes(memory: Option<MirMem>, width: usize) -> Vec<MachineByte> {
    let value = memory.as_ref().and_then(resolved_absolute_address);
    (0..width)
        .map(|index| MachineByte {
            value: value.map(|value| (value >> (index * 8)) as u8),
            memory: (index == 0).then(|| memory.clone()).flatten(),
        })
        .collect()
}

fn resolve_machine_storage(
    name: &str,
    offset: i32,
    routine: &MirRoutine,
    program: &MirProgram,
) -> Option<MirMem> {
    if let Some(slot) = routine
        .frame
        .params
        .iter()
        .chain(&routine.frame.locals)
        .find(|slot| slot.name.as_deref() == Some(name))
    {
        let memory = match slot.base {
            MirStorageBase::Param(id) | MirStorageBase::ParamAbiOnly(id) => {
                MirMem::Param { id, offset: 0 }
            }
            MirStorageBase::Local(id) | MirStorageBase::LocalAlias { id, .. } => {
                MirMem::Local { id, offset: 0 }
            }
            MirStorageBase::Spill(id) => MirMem::Spill { id, offset: 0 },
            MirStorageBase::Global(id) => MirMem::Global { id, offset: 0 },
            MirStorageBase::Static(id) => MirMem::Static { id, offset: 0 },
            MirStorageBase::Absolute(address) => absolute_mem(address),
        };
        return offset_memory(memory, i32::from(slot.offset).saturating_add(offset));
    }

    if let Some(global) = program.globals.iter().find(|global| global.name == name) {
        let memory = match global.backing {
            MirGlobalBacking::Ordinary { offset } => MirMem::Global {
                id: global.id,
                offset,
            },
            MirGlobalBacking::Absolute(address) => absolute_mem(address),
            MirGlobalBacking::Alias {
                target,
                offset: alias_offset,
            } => MirMem::Global {
                id: target,
                offset: alias_offset,
            },
        };
        return offset_memory(memory, offset);
    }

    program
        .statics
        .iter()
        .find(|static_data| static_data.name == name)
        .and_then(|static_data| {
            offset_memory(
                MirMem::Static {
                    id: static_data.id,
                    offset: 0,
                },
                offset,
            )
        })
}

fn offset_address(address: u16, offset: i32) -> Option<u16> {
    let value = i64::from(address).checked_add(i64::from(offset))?;
    u16::try_from(value).ok()
}

fn offset_memory(memory: MirMem, offset: i32) -> Option<MirMem> {
    let unsigned = u16::try_from(offset).ok()?;
    Some(offset_mem(&memory, unsigned))
}

fn resolved_absolute_address(memory: &MirMem) -> Option<u16> {
    match memory {
        MirMem::Absolute(address) => Some(*address),
        MirMem::FixedZeroPage(slot) => Some(u16::from(slot.0)),
        MirMem::Static { .. }
        | MirMem::Global { .. }
        | MirMem::Local { .. }
        | MirMem::Param { .. }
        | MirMem::Spill { .. }
        | MirMem::ZeroPage(_) => None,
    }
}

fn machine_operand_mem(operands: &[MachineByte]) -> Option<MirMem> {
    operands
        .first()
        .and_then(|operand| operand.memory.clone())
        .or_else(|| known_u16_or_u8(operands).map(absolute_mem))
}

fn record_machine_write(
    writes: &mut MirKnownMemoryWrites,
    accumulator: &mut Option<MirMachineValue>,
    zn: &mut Option<MirMachineValue>,
    mem: MirMem,
) {
    if accumulator
        .as_ref()
        .is_some_and(|value| value == &MirMachineValue::DirectMem(mem.clone()))
    {
        *accumulator = None;
    }
    if zn
        .as_ref()
        .is_some_and(|value| value == &MirMachineValue::DirectMem(mem.clone()))
    {
        *zn = None;
    }
    writes.record(mem);
}

fn record_machine_register_store(
    writes: &mut MirKnownMemoryWrites,
    accumulator: &mut Option<MirMachineValue>,
    zn: &mut Option<MirMachineValue>,
    zn_register: &mut Option<MirReg>,
    source: MirReg,
    mem: MirMem,
) {
    let source_established_zn = *zn_register == Some(source)
        || (source == MirReg::A && zn.as_ref() == accumulator.as_ref());
    record_machine_write(writes, accumulator, zn, mem.clone());
    if source == MirReg::A {
        *accumulator = Some(MirMachineValue::DirectMem(mem.clone()));
    }
    if source_established_zn {
        *zn = Some(MirMachineValue::DirectMem(mem));
        *zn_register = Some(source);
    }
}

fn invalidate_accumulator_after_unknown_write(accumulator: &mut Option<MirMachineValue>) {
    if matches!(accumulator, Some(MirMachineValue::DirectMem(_))) {
        *accumulator = None;
    }
}

fn invalidate_zn_after_unknown_write(zn: &mut Option<MirMachineValue>) {
    if matches!(zn, Some(MirMachineValue::DirectMem(_))) {
        *zn = None;
    }
}

fn opcode_changes_accumulator(opcode: u8) -> bool {
    matches!(
        opcode,
        0x01 | 0x05
            | 0x09
            | 0x0A
            | 0x0D
            | 0x11
            | 0x15
            | 0x19
            | 0x1D
            | 0x21
            | 0x25
            | 0x29
            | 0x2A
            | 0x2D
            | 0x31
            | 0x35
            | 0x39
            | 0x3D
            | 0x41
            | 0x45
            | 0x49
            | 0x4A
            | 0x4D
            | 0x51
            | 0x55
            | 0x59
            | 0x5D
            | 0x61
            | 0x65
            | 0x68
            | 0x69
            | 0x6A
            | 0x6D
            | 0x71
            | 0x75
            | 0x79
            | 0x7D
            | 0x8A
            | 0x98
            | 0xA1
            | 0xA5
            | 0xA9
            | 0xAD
            | 0xB1
            | 0xB5
            | 0xB9
            | 0xBD
            | 0xE1
            | 0xE5
            | 0xE9
            | 0xED
            | 0xF1
            | 0xF5
            | 0xF9
            | 0xFD
    )
}

fn opcode_changes_zn(opcode: u8) -> bool {
    matches!(
        opcode,
        0x24 | 0x28
            | 0x2C
            | 0xC0
            | 0xC1
            | 0xC4
            | 0xC5
            | 0xC9
            | 0xCC
            | 0xCD
            | 0xD1
            | 0xD5
            | 0xD9
            | 0xDD
            | 0xE0
            | 0xE4
            | 0xEC
    )
}

fn known_u16(bytes: &[MachineByte]) -> Option<u16> {
    Some(u16::from(bytes.first()?.value?) | (u16::from(bytes.get(1)?.value?) << 8))
}

fn known_u16_or_u8(bytes: &[MachineByte]) -> Option<u16> {
    match bytes {
        [byte] => byte.value.map(u16::from),
        [_, _, ..] => known_u16(bytes),
        [] => None,
    }
}

fn absolute_mem(address: u16) -> MirMem {
    match u8::try_from(address) {
        Ok(slot) => MirMem::FixedZeroPage(MirFixedZpSlot(slot)),
        Err(_) => MirMem::Absolute(address),
    }
}

fn offset_mem(mem: &MirMem, offset: u16) -> MirMem {
    match mem {
        MirMem::Absolute(address) => MirMem::Absolute(address.saturating_add(offset)),
        MirMem::Static {
            id,
            offset: current,
        } => MirMem::Static {
            id: *id,
            offset: current.saturating_add(offset),
        },
        MirMem::Global {
            id,
            offset: current,
        } => MirMem::Global {
            id: *id,
            offset: current.saturating_add(offset),
        },
        MirMem::Local {
            id,
            offset: current,
        } => MirMem::Local {
            id: *id,
            offset: current.saturating_add(offset),
        },
        MirMem::Param {
            id,
            offset: current,
        } => MirMem::Param {
            id: *id,
            offset: current.saturating_add(offset),
        },
        MirMem::Spill {
            id,
            offset: current,
        } => MirMem::Spill {
            id: *id,
            offset: current.saturating_add(offset),
        },
        MirMem::ZeroPage(slot) => MirMem::ZeroPage(*slot),
        MirMem::FixedZeroPage(slot) => MirMem::FixedZeroPage(MirFixedZpSlot(
            slot.0
                .saturating_add(u8::try_from(offset).unwrap_or(u8::MAX)),
        )),
    }
}

fn mem_may_alias_fixed_slot(mem: &MirMem, slot: MirFixedZpSlot) -> bool {
    match mem {
        MirMem::FixedZeroPage(candidate) => *candidate == slot,
        MirMem::Absolute(address) => *address == u16::from(slot.0),
        // Globals can have absolute or aliased backing. Without final layout,
        // do not claim that a global write preserves private zero page.
        MirMem::Global { .. } => true,
        MirMem::Static { .. }
        | MirMem::Local { .. }
        | MirMem::Param { .. }
        | MirMem::Spill { .. } => false,
        // An unallocated virtual slot has no caller-visible address yet. Once
        // fixed-pair preservation becomes a placement proof, treating it as
        // non-aliasing would be unsound: a later allocation may choose any
        // byte in the private scratch range.
        MirMem::ZeroPage(_) => true,
    }
}

fn mems_may_alias(left: &MirMem, right: &MirMem) -> bool {
    left == right
        || matches!(
            (left, right),
            (MirMem::FixedZeroPage(slot), MirMem::Absolute(address))
                | (MirMem::Absolute(address), MirMem::FixedZeroPage(slot))
                if u16::from(slot.0) == *address
        )
        || matches!(
            (left, right),
            (MirMem::Global { .. }, _) | (_, MirMem::Global { .. })
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{
        MirAddr, MirBlock, MirBlockId, MirCallAbi, MirDef, MirEffects, MirFrame, MirGlobal,
        MirMachineBlockId, MirRegisterSet, MirRoutineAbi, MirValue, MirWidth, MirZpAllocation,
        MirZpSlot,
    };

    fn routine(id: u32, ops: Vec<MirOp>, terminator: MirTerminator) -> MirRoutine {
        MirRoutine {
            id: RoutineId(id),
            name: format!("r{id}"),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: Vec::new(),
            blocks: vec![MirBlock {
                id: MirBlockId(id),
                label: format!("b{id}"),
                params: Vec::new(),
                ops,
                terminator,
            }],
            effects: MirEffects::default(),
        }
    }

    fn program(routines: Vec<MirRoutine>, machine_blocks: Vec<MirMachineBlock>) -> MirProgram {
        MirProgram {
            statics: Vec::new(),
            globals: Vec::new(),
            routines,
            machine_blocks,
            runtime_helpers: Vec::new(),
        }
    }

    #[test]
    fn exact_zn_exit_fact_is_part_of_the_known_callee_contract() {
        let value = MirMachineValue::ConstU8(0);
        let summary = MirKnownCalleeExitSummary {
            accumulator: None,
            zn: Some(value.clone()),
            preserves: MirRegisterSet::default(),
            reads: MirKnownMemoryReads::Unknown,
            writes: MirKnownMemoryWrites::Unknown,
        };
        assert_eq!(summary.zn(), Some(&value));
        assert_eq!(MirKnownCalleeExitSummary::default().zn(), None);
    }

    #[test]
    fn summarizes_registers_preserved_on_every_return_path() {
        let mut split = routine(0, Vec::new(), MirTerminator::Return);
        split.blocks = vec![
            MirBlock {
                id: MirBlockId(0),
                label: "entry".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: MirTerminator::Branch {
                    cond: crate::mir6502::ir::MirCond::FlagTest(
                        crate::mir6502::ir::MirFlagTest::ZSet,
                    ),
                    then_edge: crate::mir6502::ir::MirEdge::plain(MirBlockId(1)),
                    else_edge: crate::mir6502::ir::MirEdge::plain(MirBlockId(2)),
                },
            },
            MirBlock {
                id: MirBlockId(1),
                label: "clobber_x".to_string(),
                params: Vec::new(),
                ops: vec![MirOp::LoadImm {
                    dst: MirDef::Reg(MirReg::X),
                    value: 1,
                    width: MirWidth::Byte,
                }],
                terminator: MirTerminator::Return,
            },
            MirBlock {
                id: MirBlockId(2),
                label: "preserve".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: MirTerminator::Return,
            },
        ];

        let summaries = MirKnownCalleeSummaries::analyze(&program(vec![split], Vec::new()));
        let summary = summaries.get(RoutineId(0)).expect("routine summary");
        assert!(!summary.preserves_register(MirReg::X));
        assert!(summary.preserves_register(MirReg::Y));
    }

    #[test]
    fn propagates_register_preservation_through_known_direct_calls() {
        let callee = routine(
            1,
            vec![MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::X),
                value: 1,
                width: MirWidth::Byte,
            }],
            MirTerminator::Return,
        );
        let caller = routine(
            0,
            vec![MirOp::Call {
                target: MirCallTarget::Routine(callee.id),
                abi: MirCallAbi {
                    params: Vec::new(),
                    result: None,
                    clobbers: MirRegisterSet {
                        a: true,
                        x: true,
                        y: true,
                        flags: true,
                        ..MirRegisterSet::default()
                    },
                    preserves: MirRegisterSet::default(),
                },
                args: Vec::new(),
                result: None,
                effects: MirEffects::default(),
            }],
            MirTerminator::Return,
        );

        let summaries =
            MirKnownCalleeSummaries::analyze(&program(vec![caller, callee], Vec::new()));
        let summary = summaries.get(RoutineId(0)).expect("caller summary");
        assert!(!summary.preserves_register(MirReg::X));
        assert!(summary.preserves_register(MirReg::Y));
    }

    #[test]
    fn machine_summary_tracks_index_register_writes() {
        let machine = MirMachineBlock {
            id: MirMachineBlockId(0),
            items: vec![
                MirMachineItem::Byte(0xA2),
                MirMachineItem::Byte(7),
                MirMachineItem::Byte(0x60),
            ],
        };
        let summaries = MirKnownCalleeSummaries::analyze(&program(
            vec![routine(
                0,
                vec![MirOp::MachineBlock {
                    id: machine.id,
                    effects: MirEffects::default(),
                }],
                MirTerminator::Unreachable,
            )],
            vec![machine],
        ));

        let summary = summaries.get(RoutineId(0)).expect("machine summary");
        assert!(!summary.preserves_register(MirReg::X));
        assert!(summary.preserves_register(MirReg::Y));
    }

    #[test]
    fn machine_summary_resolves_symbolic_fixed_zero_page_operands() {
        let machine = MirMachineBlock {
            id: MirMachineBlockId(0),
            items: vec![
                MirMachineItem::Byte(0xA6),
                MirMachineItem::Name("source".to_string()),
                MirMachineItem::Byte(0x85),
                MirMachineItem::Name("scratch".to_string()),
                MirMachineItem::Byte(0x60),
            ],
        };
        let mut input = program(
            vec![routine(
                0,
                vec![MirOp::MachineBlock {
                    id: machine.id,
                    effects: MirEffects::default(),
                }],
                MirTerminator::Unreachable,
            )],
            vec![machine],
        );
        input.globals = vec![
            MirGlobal {
                id: crate::nir::SymbolId(0),
                name: "source".to_string(),
                kind: "byte".to_string(),
                width: Some(MirWidth::Byte),
                storage_size: 1,
                backing: MirGlobalBacking::Absolute(0x00FE),
                init: None,
            },
            MirGlobal {
                id: crate::nir::SymbolId(1),
                name: "scratch".to_string(),
                kind: "byte".to_string(),
                width: Some(MirWidth::Byte),
                storage_size: 1,
                backing: MirGlobalBacking::Absolute(0x00E6),
                init: None,
            },
        ];

        let summaries = MirKnownCalleeSummaries::analyze(&input);
        let summary = summaries.get(RoutineId(0)).expect("machine summary");
        assert!(!summary.preserves_register(MirReg::X));
        assert!(summary.preserves_register(MirReg::Y));
        assert!(summary.preserves_fixed_pair(MirFixedZpSlot(0xAC)));
        assert!(!summary.preserves_fixed_pair(MirFixedZpSlot(0xE6)));
        assert!(!summary.reads().may_read_caller_private_ram());
    }

    #[test]
    fn routine_summary_resolves_allocated_private_zero_page_writes() {
        let slot = MirZpSlot(0);
        let mut callee = routine(
            0,
            vec![MirOp::Store {
                dst: MirAddr::Direct(MirMem::ZeroPage(slot)),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            }],
            MirTerminator::Return,
        );
        callee.frame.virtual_zero_page.push(slot);
        callee.frame.zero_page_allocations.push(MirZpAllocation {
            slot,
            start: MirFixedZpSlot(0xE0),
            size: 1,
        });

        let summaries = MirKnownCalleeSummaries::analyze(&program(vec![callee], Vec::new()));
        let summary = summaries.get(RoutineId(0)).expect("routine summary");

        assert!(!summary.preserves_fixed_pair(MirFixedZpSlot(0xE0)));
        assert!(summary.preserves_fixed_pair(MirFixedZpSlot(0xE2)));
    }

    #[test]
    fn machine_summary_unions_forward_branch_effects_and_preservation() {
        let machine = MirMachineBlock {
            id: MirMachineBlockId(0),
            items: vec![
                MirMachineItem::Byte(0xA6),
                MirMachineItem::Byte(0x58),
                MirMachineItem::Byte(0xA9),
                MirMachineItem::Byte(1),
                MirMachineItem::Byte(0x85),
                MirMachineItem::Byte(0xE6),
                MirMachineItem::Byte(0x90),
                MirMachineItem::Byte(2),
                MirMachineItem::Byte(0xE6),
                MirMachineItem::Byte(0xE7),
                MirMachineItem::Byte(0x60),
            ],
        };
        let summaries = MirKnownCalleeSummaries::analyze(&program(
            vec![routine(
                0,
                vec![MirOp::MachineBlock {
                    id: machine.id,
                    effects: MirEffects::default(),
                }],
                MirTerminator::Unreachable,
            )],
            vec![machine],
        ));

        let summary = summaries.get(RoutineId(0)).expect("machine summary");
        assert!(!summary.preserves_register(MirReg::X));
        assert!(summary.preserves_register(MirReg::Y));
        assert!(
            summary
                .writes()
                .may_write_mem(&MirMem::FixedZeroPage(MirFixedZpSlot(0xE6)))
        );
        assert!(
            summary
                .writes()
                .may_write_mem(&MirMem::FixedZeroPage(MirFixedZpSlot(0xE7)))
        );
        assert_eq!(summary.accumulator(), None);
        assert_eq!(summary.zn(), None);
    }

    #[test]
    fn machine_summary_rejects_backward_branch_effect_walks() {
        let machine = MirMachineBlock {
            id: MirMachineBlockId(0),
            items: vec![
                MirMachineItem::Byte(0xD0),
                MirMachineItem::Byte(0xFE),
                MirMachineItem::Byte(0x60),
            ],
        };
        let summaries = MirKnownCalleeSummaries::analyze(&program(
            vec![routine(
                0,
                vec![MirOp::MachineBlock {
                    id: machine.id,
                    effects: MirEffects::default(),
                }],
                MirTerminator::Unreachable,
            )],
            vec![machine],
        ));

        let summary = summaries.get(RoutineId(0)).expect("default summary");
        assert!(!summary.preserves_register(MirReg::X));
        assert!(!summary.preserves_register(MirReg::Y));
        assert!(matches!(summary.reads(), MirKnownMemoryReads::Unknown));
        assert!(matches!(summary.writes(), MirKnownMemoryWrites::Unknown));
    }

    #[test]
    fn summarizes_accumulator_stored_to_return_slot() {
        let return_slot = MirMem::FixedZeroPage(MirFixedZpSlot(0xA0));
        let summaries = MirKnownCalleeSummaries::analyze(&program(
            vec![routine(
                0,
                vec![
                    MirOp::LoadImm {
                        dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                        value: 7,
                        width: MirWidth::Byte,
                    },
                    MirOp::Store {
                        dst: MirAddr::Direct(return_slot.clone()),
                        src: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
                        width: MirWidth::Byte,
                    },
                ],
                MirTerminator::Return,
            )],
            Vec::new(),
        ));
        assert_eq!(
            summaries
                .get(RoutineId(0))
                .and_then(|summary| summary.accumulator()),
            Some(&MirMachineValue::DirectMem(return_slot))
        );
        assert_eq!(
            summaries.get(RoutineId(0)).and_then(|summary| summary.zn()),
            Some(&MirMachineValue::DirectMem(MirMem::FixedZeroPage(
                MirFixedZpSlot(0xA0)
            )))
        );
    }

    #[test]
    fn summarizes_supported_machine_return_without_routine_name_special_case() {
        let machine = MirMachineBlock {
            id: MirMachineBlockId(0),
            items: vec![
                MirMachineItem::Byte(0x20),
                MirMachineItem::Word(0xF2F8),
                MirMachineItem::Byte(0x85),
                MirMachineItem::Byte(0xA0),
                MirMachineItem::Byte(0x60),
            ],
        };
        let summaries = MirKnownCalleeSummaries::analyze(&program(
            vec![routine(
                0,
                vec![MirOp::MachineBlock {
                    id: machine.id,
                    effects: MirEffects {
                        opaque: true,
                        ..MirEffects::default()
                    },
                }],
                MirTerminator::Unreachable,
            )],
            vec![machine],
        ));
        let summary = summaries.get(RoutineId(0)).expect("machine summary");
        assert_eq!(
            summary.accumulator(),
            Some(&MirMachineValue::DirectMem(MirMem::FixedZeroPage(
                MirFixedZpSlot(0xA0)
            )))
        );
        assert!(summary.preserves_fixed_pair(MirFixedZpSlot(0xAC)));
    }

    #[test]
    fn summarizes_machine_indirect_result_stored_to_return_slot() {
        let machine = MirMachineBlock {
            id: MirMachineBlockId(0),
            items: vec![
                MirMachineItem::Byte(0x85),
                MirMachineItem::Byte(0xA0),
                MirMachineItem::Byte(0x86),
                MirMachineItem::Byte(0xA1),
                MirMachineItem::Byte(0xA0),
                MirMachineItem::Byte(0x00),
                MirMachineItem::Byte(0xB1),
                MirMachineItem::Byte(0xA0),
                MirMachineItem::Byte(0x85),
                MirMachineItem::Byte(0xA0),
                MirMachineItem::Byte(0x60),
            ],
        };
        let summaries = MirKnownCalleeSummaries::analyze(&program(
            vec![routine(
                0,
                vec![MirOp::MachineBlock {
                    id: machine.id,
                    effects: MirEffects::default(),
                }],
                MirTerminator::Unreachable,
            )],
            vec![machine],
        ));
        let return_slot = MirMachineValue::DirectMem(MirMem::FixedZeroPage(MirFixedZpSlot(0xA0)));
        let summary = summaries.get(RoutineId(0)).expect("machine summary");
        assert_eq!(summary.accumulator(), Some(&return_slot));
        assert_eq!(summary.zn(), Some(&return_slot));
    }

    #[test]
    fn unsupported_machine_call_keeps_write_summary_unknown() {
        let machine = MirMachineBlock {
            id: MirMachineBlockId(0),
            items: vec![
                MirMachineItem::Byte(0x20),
                MirMachineItem::Word(0x1234),
                MirMachineItem::Byte(0x85),
                MirMachineItem::Byte(0xA0),
                MirMachineItem::Byte(0x60),
            ],
        };
        let summaries = MirKnownCalleeSummaries::analyze(&program(
            vec![routine(
                0,
                vec![MirOp::MachineBlock {
                    id: machine.id,
                    effects: MirEffects::default(),
                }],
                MirTerminator::Unreachable,
            )],
            vec![machine],
        ));
        let summary = summaries.get(RoutineId(0)).expect("machine summary");
        assert_eq!(
            summary.accumulator(),
            Some(&MirMachineValue::DirectMem(MirMem::FixedZeroPage(
                MirFixedZpSlot(0xA0)
            )))
        );
        assert!(!summary.preserves_fixed_pair(MirFixedZpSlot(0xAC)));
    }

    #[test]
    fn later_machine_write_invalidates_accumulator_memory_equality() {
        let machine = MirMachineBlock {
            id: MirMachineBlockId(0),
            items: vec![
                MirMachineItem::Byte(0xA9),
                MirMachineItem::Byte(7),
                MirMachineItem::Byte(0x85),
                MirMachineItem::Byte(0xA0),
                MirMachineItem::Byte(0xA2),
                MirMachineItem::Byte(9),
                MirMachineItem::Byte(0x86),
                MirMachineItem::Byte(0xA0),
                MirMachineItem::Byte(0x60),
            ],
        };
        let summaries = MirKnownCalleeSummaries::analyze(&program(
            vec![routine(
                0,
                vec![MirOp::MachineBlock {
                    id: machine.id,
                    effects: MirEffects::default(),
                }],
                MirTerminator::Unreachable,
            )],
            vec![machine],
        ));
        assert_eq!(
            summaries
                .get(RoutineId(0))
                .and_then(|summary| summary.accumulator()),
            None
        );
        assert_eq!(
            summaries.get(RoutineId(0)).and_then(|summary| summary.zn()),
            Some(&MirMachineValue::DirectMem(MirMem::FixedZeroPage(
                MirFixedZpSlot(0xA0)
            )))
        );
    }

    #[test]
    fn propagates_exit_accumulator_through_an_acyclic_direct_call() {
        let return_slot = MirMem::FixedZeroPage(MirFixedZpSlot(0xA0));
        let callee = routine(
            1,
            vec![
                MirOp::LoadImm {
                    dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                    value: 9,
                    width: MirWidth::Byte,
                },
                MirOp::Store {
                    dst: MirAddr::Direct(return_slot.clone()),
                    src: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
                    width: MirWidth::Byte,
                },
            ],
            MirTerminator::Return,
        );
        let caller = routine(
            0,
            vec![MirOp::Call {
                target: MirCallTarget::Routine(callee.id),
                abi: MirCallAbi {
                    params: Vec::new(),
                    result: None,
                    clobbers: MirRegisterSet {
                        a: true,
                        ..MirRegisterSet::default()
                    },
                    preserves: MirRegisterSet::default(),
                },
                args: Vec::new(),
                result: None,
                effects: MirEffects::default(),
            }],
            MirTerminator::Return,
        );
        let summaries =
            MirKnownCalleeSummaries::analyze(&program(vec![caller, callee], Vec::new()));
        assert_eq!(
            summaries
                .get(RoutineId(0))
                .and_then(|summary| summary.accumulator()),
            Some(&MirMachineValue::DirectMem(return_slot))
        );
    }

    #[test]
    fn disagreeing_return_paths_do_not_claim_an_accumulator_value() {
        let mut split = routine(0, Vec::new(), MirTerminator::Return);
        split.blocks = vec![
            MirBlock {
                id: MirBlockId(0),
                label: "entry".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: MirTerminator::Branch {
                    cond: crate::mir6502::ir::MirCond::FlagTest(
                        crate::mir6502::ir::MirFlagTest::ZSet,
                    ),
                    then_edge: crate::mir6502::ir::MirEdge::plain(MirBlockId(1)),
                    else_edge: crate::mir6502::ir::MirEdge::plain(MirBlockId(2)),
                },
            },
            MirBlock {
                id: MirBlockId(1),
                label: "one".to_string(),
                params: Vec::new(),
                ops: vec![MirOp::LoadImm {
                    dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                    value: 1,
                    width: MirWidth::Byte,
                }],
                terminator: MirTerminator::Return,
            },
            MirBlock {
                id: MirBlockId(2),
                label: "two".to_string(),
                params: Vec::new(),
                ops: vec![MirOp::LoadImm {
                    dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                    value: 2,
                    width: MirWidth::Byte,
                }],
                terminator: MirTerminator::Return,
            },
        ];
        let summaries = MirKnownCalleeSummaries::analyze(&program(vec![split], Vec::new()));
        assert_eq!(
            summaries
                .get(RoutineId(0))
                .and_then(|summary| summary.accumulator()),
            None
        );
        assert_eq!(
            summaries.get(RoutineId(0)).and_then(|summary| summary.zn()),
            None
        );
    }

    #[test]
    fn matching_return_paths_claim_an_exact_zn_value() {
        let mut split = routine(0, Vec::new(), MirTerminator::Return);
        split.blocks = vec![
            MirBlock {
                id: MirBlockId(0),
                label: "entry".to_string(),
                params: Vec::new(),
                ops: Vec::new(),
                terminator: MirTerminator::Branch {
                    cond: crate::mir6502::ir::MirCond::FlagTest(
                        crate::mir6502::ir::MirFlagTest::ZSet,
                    ),
                    then_edge: crate::mir6502::ir::MirEdge::plain(MirBlockId(1)),
                    else_edge: crate::mir6502::ir::MirEdge::plain(MirBlockId(2)),
                },
            },
            MirBlock {
                id: MirBlockId(1),
                label: "one".to_string(),
                params: Vec::new(),
                ops: vec![MirOp::LoadImm {
                    dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                    value: 7,
                    width: MirWidth::Byte,
                }],
                terminator: MirTerminator::Return,
            },
            MirBlock {
                id: MirBlockId(2),
                label: "two".to_string(),
                params: Vec::new(),
                ops: vec![MirOp::LoadImm {
                    dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                    value: 7,
                    width: MirWidth::Byte,
                }],
                terminator: MirTerminator::Return,
            },
        ];
        let summaries = MirKnownCalleeSummaries::analyze(&program(vec![split], Vec::new()));
        assert_eq!(
            summaries.get(RoutineId(0)).and_then(|summary| summary.zn()),
            Some(&MirMachineValue::ConstU8(7))
        );
    }

    #[test]
    fn routine_private_zn_provenance_is_not_exposed_to_callers() {
        let local = MirMem::Local {
            id: crate::nir::LocalId(0),
            offset: 0,
        };
        let summaries = MirKnownCalleeSummaries::analyze(&program(
            vec![routine(
                0,
                vec![MirOp::Load {
                    dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                    src: MirAddr::Direct(local),
                    width: MirWidth::Byte,
                }],
                MirTerminator::Return,
            )],
            Vec::new(),
        ));
        assert_eq!(
            summaries.get(RoutineId(0)).and_then(|summary| summary.zn()),
            None
        );
    }

    #[test]
    fn summarizes_unknown_indirect_byte_after_it_is_stored_to_a_public_home() {
        let return_slot = MirMem::FixedZeroPage(MirFixedZpSlot(0xA0));
        let summaries = MirKnownCalleeSummaries::analyze(&program(
            vec![routine(
                0,
                vec![
                    MirOp::LoadIndirect {
                        consumer: crate::mir6502::ir::MirAddressConsumer::IndirectIndexedY(
                            crate::mir6502::ir::MirPointerPair::Fixed {
                                lo: MirFixedZpSlot(0xAC),
                            },
                        ),
                        dst: MirDef::Reg(crate::mir6502::ir::MirReg::A),
                        offset: 0,
                    },
                    MirOp::Store {
                        dst: MirAddr::Direct(return_slot.clone()),
                        src: MirValue::Def(MirDef::Reg(crate::mir6502::ir::MirReg::A)),
                        width: MirWidth::Byte,
                    },
                ],
                MirTerminator::Return,
            )],
            Vec::new(),
        ));
        assert_eq!(
            summaries.get(RoutineId(0)).and_then(|summary| summary.zn()),
            Some(&MirMachineValue::DirectMem(return_slot))
        );
    }
}
