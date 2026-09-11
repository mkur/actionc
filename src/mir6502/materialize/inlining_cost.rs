//! Bounded path costs over final emitted code, including allocation, relaxed
//! branches, tail calls and retained callee bodies. Unknown code rejects a trial.
use std::collections::{BTreeMap, BTreeSet};

use crate::codegen::{AddressingMode, decode_6502_opcode, tracked_emitter::TrackedEmitter};
use crate::mir6502::{emit, ir::*};

const MIN_CYCLES_PER_SITE: u32 = 4;

pub(super) struct Image {
    pub bytes: Vec<u8>,
    pub blocks: Vec<(RoutineId, MirBlockId, std::ops::Range<usize>)>,
    entries: BTreeMap<usize, RoutineId>,
    helpers: BTreeMap<RoutineId, MirRuntimeHelperDecl>,
    nested_arithmetic: BTreeMap<RoutineId, BTreeSet<RoutineId>>,
    origin: u16,
}

impl Image {
    pub fn build(program: &MirProgram, origin: u16) -> Option<Self> {
        let mut emitter = TrackedEmitter::with_origin(origin);
        let summary = emit::emit_program(program, origin, &mut emitter).ok()?;
        let bytes = emitter.finish_with_relocations().ok()?.bytes;
        let mut entries = BTreeMap::new();
        for routine in &program.routines {
            let first = routine.blocks.first()?;
            let (_, _, range) = summary
                .block_ranges
                .iter()
                .find(|(r, b, _)| *r == routine.id && *b == first.id)?;
            entries.insert(range.start, routine.id);
        }
        let helpers: BTreeMap<_, _> = program
            .runtime_helpers
            .iter()
            .filter_map(|decl| match decl.target {
                MirRuntimeHelperTarget::Routine(id) => Some((decl.helper, id)),
                _ => None,
            })
            .collect();
        let edges: BTreeMap<_, BTreeSet<_>> = program
            .routines
            .iter()
            .map(|routine| {
                let targets = routine
                    .blocks
                    .iter()
                    .flat_map(|b| &b.ops)
                    .filter_map(|op| match op {
                        MirOp::RuntimeHelper { helper, .. } => helpers.get(helper).copied(),
                        MirOp::Call {
                            target: MirCallTarget::Routine(id),
                            ..
                        } => Some(*id),
                        _ => None,
                    })
                    .collect();
                (routine.id, targets)
            })
            .collect();
        let arithmetic: BTreeSet<_> = helpers
            .iter()
            .filter(|(helper, _)| helper.is_wide())
            .map(|(_, id)| *id)
            .collect();
        let mut nested_arithmetic: BTreeMap<_, BTreeSet<_>> = edges
            .iter()
            .map(|(id, targets)| (*id, targets.intersection(&arithmetic).copied().collect()))
            .collect();
        loop {
            let previous = nested_arithmetic.clone();
            for (id, targets) in &mut nested_arithmetic {
                for called in &edges[id] {
                    targets.extend(previous.get(called).into_iter().flatten());
                }
            }
            if nested_arithmetic == previous {
                break;
            }
        }
        Some(Self {
            nested_arithmetic,
            bytes,
            blocks: summary.block_ranges,
            entries,
            helpers: program
                .runtime_helpers
                .iter()
                .filter_map(|decl| match decl.target {
                    MirRuntimeHelperTarget::Routine(id) => Some((id, decl.clone())),
                    _ => None,
                })
                .collect(),
            origin,
        })
    }

    fn target(&self, pc: usize) -> Option<usize> {
        let address = u16::from_le_bytes([*self.bytes.get(pc + 1)?, *self.bytes.get(pc + 2)?]);
        usize::from(address).checked_sub(usize::from(self.origin))
    }

    pub fn routine_bytes(&self, id: RoutineId) -> usize {
        self.blocks
            .iter()
            .filter(|(r, _, _)| *r == id)
            .map(|(_, _, range)| range.len())
            .sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Exit {
    block: Option<MirBlockId>,
    /// Opaque calls may cancel only against the same calls in the same order.
    calls: Vec<RoutineId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Bounds {
    min: u32,
    max: u32,
}
type Paths = BTreeMap<Exit, Bounds>;

fn merge(into: &mut Paths, paths: Paths, cost: Bounds, calls: &[RoutineId]) -> Option<()> {
    for (mut exit, bound) in paths {
        exit.calls.splice(0..0, calls.iter().copied());
        let bound = Bounds {
            min: bound.min.checked_add(cost.min)?,
            max: bound.max.checked_add(cost.max)?,
        };
        into.entry(exit)
            .and_modify(|old| {
                old.min = old.min.min(bound.min);
                old.max = old.max.max(bound.max);
            })
            .or_insert(bound);
    }
    (into.len() <= 128).then_some(())
}

fn terminal(block: Option<MirBlockId>) -> Paths {
    BTreeMap::from([(
        Exit {
            block,
            calls: vec![],
        },
        Bounds { min: 0, max: 0 },
    )])
}

struct Walk<'a> {
    image: &'a Image,
    boundaries: BTreeMap<usize, MirBlockId>,
    allowed: Vec<std::ops::Range<usize>>,
    leaf: Option<(RoutineId, Paths)>,
    visiting: BTreeSet<usize>,
    memo: BTreeMap<usize, Paths>,
    fuel: usize,
}

impl Walk<'_> {
    fn next(&mut self, pc: usize) -> Option<Paths> {
        if let Some(block) = self.boundaries.get(&pc) {
            return Some(terminal(Some(*block)));
        }
        self.at(pc)
    }

    fn at(&mut self, pc: usize) -> Option<Paths> {
        if let Some(paths) = self.memo.get(&pc) {
            return Some(paths.clone());
        }
        self.fuel = self.fuel.checked_sub(1)?;
        if !self.allowed.iter().any(|r| r.contains(&pc))
            || !self.visiting.insert(pc)
            || self.visiting.len() > 1024
        {
            return None;
        }
        let opcode = *self.image.bytes.get(pc)?;
        let (name, mode, len) = decode_6502_opcode(opcode)?;
        let mut result = Paths::new();
        let fixed = |n| Bounds { min: n, max: n };
        match name {
            "RTS" => {
                merge(&mut result, terminal(None), fixed(6), &[])?;
            }
            "JSR" | "JMP" if mode == AddressingMode::Absolute => {
                let target = self.image.target(pc)?;
                if name == "JMP"
                    && (self.boundaries.contains_key(&target)
                        || self.allowed.iter().any(|r| r.contains(&target)))
                {
                    let paths = self.next(target)?;
                    merge(&mut result, paths, fixed(3), &[])?;
                } else if let Some(id) = self.image.entries.get(&target).copied() {
                    let paths = if name == "JSR" {
                        self.next(pc + len)?
                    } else {
                        terminal(None)
                    };
                    if let Some((_, summaries)) = self.leaf.as_ref().filter(|(leaf, _)| *leaf == id)
                    {
                        for (exit, cost) in summaries {
                            if exit.block.is_some() {
                                return None;
                            }
                            let overhead = if name == "JSR" { 6 } else { 3 };
                            merge(
                                &mut result,
                                paths.clone(),
                                Bounds {
                                    min: cost.min.checked_add(overhead)?,
                                    max: cost.max.checked_add(overhead)?,
                                },
                                &exit.calls,
                            )?;
                        }
                    } else {
                        // Equivalent opaque work cancels, including its normal
                        // return; a tail call supplies this region's RTS as well.
                        merge(
                            &mut result,
                            paths,
                            fixed(if name == "JSR" { 6 } else { 3 + 6 }),
                            &[id],
                        )?;
                    }
                } else if name == "JMP" {
                    let paths = self.next(target)?;
                    merge(&mut result, paths, fixed(3), &[])?;
                } else {
                    return None;
                }
            }
            _ if mode == AddressingMode::Relative => {
                let next = pc + len;
                let delta = *self.image.bytes.get(pc + 1)? as i8;
                let target = next.checked_add_signed(isize::from(delta))?;
                let crossed = ((next + usize::from(self.image.origin)) >> 8)
                    != ((target + usize::from(self.image.origin)) >> 8);
                let paths = self.next(next)?;
                merge(&mut result, paths, fixed(2), &[])?;
                let paths = self.next(target)?;
                merge(&mut result, paths, fixed(3 + u32::from(crossed)), &[])?;
            }
            _ => {
                let cost = instruction_cycles(name, mode)?;
                let paths = self.next(pc + len)?;
                merge(&mut result, paths, fixed(cost), &[])?;
            }
        }
        self.visiting.remove(&pc);
        self.memo.insert(pc, result.clone());
        Some(result)
    }
}

/// Indexed reads are charged their page-crossing maximum in both versions.
/// This is an estimate, not a prediction of input-dependent emulator timings.
fn instruction_cycles(name: &str, mode: AddressingMode) -> Option<u32> {
    use AddressingMode::*;
    Some(match (name, mode) {
        (
            "LDA" | "LDX" | "LDY" | "ADC" | "SBC" | "AND" | "ORA" | "EOR" | "CMP" | "CPX" | "CPY",
            Immediate,
        ) => 2,
        (
            "LDA" | "LDX" | "LDY" | "ADC" | "SBC" | "AND" | "ORA" | "EOR" | "CMP" | "CPX" | "CPY"
            | "BIT" | "STA" | "STX" | "STY",
            ZeroPage,
        ) => 3,
        (
            "LDA" | "LDX" | "LDY" | "ADC" | "SBC" | "AND" | "ORA" | "EOR" | "CMP" | "STA" | "STX"
            | "STY",
            ZeroPageX | ZeroPageY,
        ) => 4,
        (
            "LDA" | "LDX" | "LDY" | "ADC" | "SBC" | "AND" | "ORA" | "EOR" | "CMP" | "CPX" | "CPY"
            | "BIT" | "STA" | "STX" | "STY",
            Absolute,
        ) => 4,
        (
            "LDA" | "LDX" | "LDY" | "ADC" | "SBC" | "AND" | "ORA" | "EOR" | "CMP" | "STA",
            AbsoluteX | AbsoluteY,
        ) => 5,
        (
            "LDA" | "ADC" | "SBC" | "AND" | "ORA" | "EOR" | "CMP" | "STA",
            IndirectIndexedY | IndexedIndirectX,
        ) => 6,
        ("ASL" | "LSR" | "ROL" | "ROR", Accumulator) => 2,
        ("ASL" | "LSR" | "ROL" | "ROR" | "INC" | "DEC", ZeroPage) => 5,
        ("ASL" | "LSR" | "ROL" | "ROR" | "INC" | "DEC", ZeroPageX | Absolute) => 6,
        ("ASL" | "LSR" | "ROL" | "ROR" | "INC" | "DEC", AbsoluteX) => 7,
        (
            "TAX" | "TAY" | "TXA" | "TYA" | "TSX" | "TXS" | "INX" | "INY" | "DEX" | "DEY" | "CLC"
            | "SEC" | "CLD" | "SED" | "CLV" | "NOP",
            Implied,
        ) => 2,
        ("PHA" | "PHP", Implied) => 3,
        ("PLA" | "PLP", Implied) => 4,
        _ => return None,
    })
}

/// Longest path of *timing differences*, with X tracked across emitted
/// LDX/DEX loops. Unknown branches explore both successors; TAX explores every
/// byte value. Unrecognized control or a cycle without a proven counter is
/// unknown. No data-dependent helper body is assigned a fixed/free cost.
fn relative_layout_penalty(
    code: &[u8],
    old_base: usize,
    new_base: usize,
    error: Option<usize>,
) -> Option<u32> {
    type State = (usize, Option<u8>, Option<bool>);
    let mut instructions = BTreeMap::new();
    let mut pc = 0;
    while pc < code.len() {
        let decoded = decode_6502_opcode(code[pc])?;
        instructions.insert(pc, decoded);
        pc += decoded.2;
    }
    let mut graph = BTreeMap::<State, Vec<(State, i32)>>::new();
    let mut pending = vec![(0, None, None)];
    while let Some(state @ (pc, x, zero)) = pending.pop() {
        if graph.contains_key(&state) {
            continue;
        }
        if graph.len() >= 65536 {
            return None;
        }
        let &(name, mode, len) = instructions.get(&pc)?;
        let next = pc + len;
        let mut edges = Vec::new();
        match (name, mode) {
            ("RTS", _) => {}
            // Error is nonreturning even if the installed handler returns.
            ("JSR", AddressingMode::Absolute) if error == Some(pc + 1) => {}
            ("LDX", AddressingMode::Immediate) => {
                edges.push(((next, Some(code[pc + 1]), Some(code[pc + 1] == 0)), 0))
            }
            ("DEX", AddressingMode::Implied) => {
                edges.push(((next, Some(x?.wrapping_sub(1)), Some(x? == 1)), 0))
            }
            ("TAX", AddressingMode::Implied) => {
                edges.extend((0..=255).map(|x| ((next, Some(x), Some(x == 0)), 0)))
            }
            (_, AddressingMode::Relative) => {
                let target = next.checked_add_signed(isize::from(code[pc + 1] as i8))?;
                let crossing = |base| i32::from(((base + next) >> 8) != ((base + target) >> 8));
                let delta = crossing(new_base) - crossing(old_base);
                let taken = match name {
                    "BNE" => zero.map(|z| !z),
                    "BEQ" => zero,
                    _ => None,
                };
                if taken != Some(true) {
                    edges.push(((next, x, zero), 0));
                }
                if taken != Some(false) {
                    edges.push(((target, x, zero), delta));
                }
            }
            ("JMP" | "JSR" | "BRK" | "RTI" | "INX" | "PLX" | "TSX" | "LDX", _) => return None,
            _ => edges.push(((next, x, None), 0)),
        }
        pending.extend(edges.iter().map(|(next, _)| *next));
        graph.insert(state, edges);
    }
    // Reverse topological evaluation avoids recursion through up to 256 loop
    // rounds. A remaining cycle rejects the estimate instead of truncating it.
    let mut predecessors = BTreeMap::<State, Vec<(State, i32)>>::new();
    let mut remaining = BTreeMap::new();
    let mut best = BTreeMap::<State, i32>::new();
    let mut ready = Vec::new();
    for (&state, edges) in &graph {
        remaining.insert(state, edges.len());
        if edges.is_empty() {
            best.insert(state, 0);
            ready.push(state);
        }
        for &(next, weight) in edges {
            predecessors.entry(next).or_default().push((state, weight));
        }
    }
    while let Some(state) = ready.pop() {
        for &(parent, weight) in predecessors.get(&state).into_iter().flatten() {
            let value = best[&state].checked_add(weight)?;
            best.entry(parent)
                .and_modify(|old| *old = (*old).max(value))
                .or_insert(value);
            let count = remaining.get_mut(&parent)?;
            *count -= 1;
            if *count == 0 {
                ready.push(parent);
            }
        }
    }
    if remaining.values().any(|n| *n != 0) {
        return None;
    }
    Some((*best.get(&(0, None, None))?).max(0) as u32)
}

/// Pair only the exact compiler-owned wide kernel, ABI and fault target.
/// Data-dependent work cancels; relocation overhead is bounded separately over
/// its emitted control graph, including counted loops and page crossings.
fn helper_relocation_penalty(before: &Image, after: &Image, calls: &[RoutineId]) -> Option<u32> {
    let mut penalty = 0u32;
    for id in calls {
        let Some(decl) = before.helpers.get(id).filter(|d| d.helper.is_wide()) else {
            // An unchanged opaque callee may itself reach data-dependent wide
            // helpers. Without a bounded invocation count, cancel it only when
            // none of those retained kernels can become slower after moving.
            if before.nested_arithmetic.get(id) != after.nested_arithmetic.get(id) {
                return None;
            }
            for helper in before.nested_arithmetic.get(id).into_iter().flatten() {
                if helper_relocation_penalty(before, after, &[*helper])? != 0 {
                    return None;
                }
            }
            continue;
        };
        if after.helpers.get(id) != Some(decl) {
            return None;
        }
        let (expected, error) = match decl.helper {
            MirRuntimeHelper::Mul32 => (crate::integer6502::wide::multiply(), None),
            MirRuntimeHelper::Lsh32 | MirRuntimeHelper::Rsh32 => (
                crate::integer6502::wide::shift_body(decl.helper == MirRuntimeHelper::Lsh32),
                None,
            ),
            helper => {
                let body = crate::integer6502::wide::division(
                    matches!(helper, MirRuntimeHelper::Div32 | MirRuntimeHelper::Mod32),
                    matches!(helper, MirRuntimeHelper::Mod32 | MirRuntimeHelper::UMod32),
                );
                (body.bytes, Some(body.error_operand))
            }
        };
        let range = |image: &Image| {
            let ranges: Vec<_> = image.blocks.iter().filter(|(r, _, _)| r == id).collect();
            (ranges.len() == 1).then(|| ranges[0].2.clone())
        };
        let old_range = range(before)?;
        let new_range = range(after)?;
        if old_range.len() != expected.len() + 1 || new_range.len() != old_range.len() {
            return None;
        }
        let old = before.bytes.get(old_range.clone())?;
        let new = after.bytes.get(new_range.clone())?;
        for (offset, expected) in expected.iter().chain(std::iter::once(&0x60)).enumerate() {
            if error.is_some_and(|e| (e..e + 2).contains(&offset)) {
                continue;
            }
            if old[offset] != *expected || new[offset] != *expected {
                return None;
            }
        }
        if let Some(operand) = error {
            let target = |image: &Image, start: usize| {
                let address = u16::from_le_bytes([
                    image.bytes[start + operand],
                    image.bytes[start + operand + 1],
                ]);
                let routine = usize::from(address)
                    .checked_sub(usize::from(image.origin))
                    .and_then(|p| image.entries.get(&p))
                    .copied();
                (routine, routine.is_none().then_some(address))
            };
            if target(before, old_range.start) != target(after, new_range.start) {
                return None;
            }
        }
        penalty = penalty.checked_add(relative_layout_penalty(
            old,
            usize::from(before.origin) + old_range.start,
            usize::from(after.origin) + new_range.start,
            error,
        )?)?;
    }
    Some(penalty)
}

/// Renumbered block-local virtual ZP bytes do not change an unchanged region's
/// computation. Require no incoming/outgoing value, address escape or indirect
/// access for every renamed byte; all other operations and control stay exact.
fn equivalent_private_block(old: &MirRoutine, new: &MirRoutine, id: MirBlockId) -> bool {
    use crate::mir6502::analysis::{
        cfg::MirCfg, effects::MirHomeByte, home_liveness::MirHomeLiveness,
    };
    fn normalized(routine: &MirRoutine, id: MirBlockId) -> Option<MirBlock> {
        let cfg = MirCfg::from_routine(routine).ok()?;
        let live = MirHomeLiveness::analyze(routine, &cfg);
        let facts = live.block_by_id(id)?;
        let mut block = routine.blocks.iter().find(|b| b.id == id)?.clone();
        let mut slots = BTreeMap::new();
        for op in &block.ops {
            super::spills::visit_op_mems(op, &mut |mem| {
                if let MirMem::ZeroPage(slot) = mem {
                    let next = MirZpSlot(slots.len() as u32);
                    slots.entry(*slot).or_insert(next);
                }
            });
        }
        for &slot in slots.keys() {
            if facts.live_in.iter().chain(facts.live_out.iter()).any(|home|
                matches!(home, MirHomeByte::VirtualZeroPage { slot: live, .. } if live == slot)) {
                return None;
            }
        }
        fn mem(mem: &mut MirMem, slots: &BTreeMap<MirZpSlot, MirZpSlot>) {
            if let MirMem::ZeroPage(slot) = mem {
                *slot = slots[slot];
            }
        }
        fn value(v: &mut MirValue, slots: &BTreeMap<MirZpSlot, MirZpSlot>) -> Option<()> {
            match v {
                MirValue::PointerCell(m) => mem(m, slots),
                MirValue::Word { lo, hi } => {
                    value(lo, slots)?;
                    value(hi, slots)?;
                }
                MirValue::StorageAddrByte {
                    mem: MirMem::ZeroPage(_),
                    ..
                } => return None,
                _ => {}
            }
            Some(())
        }
        for op in &mut block.ops {
            match op {
                MirOp::Load {
                    src: MirAddr::Direct(m),
                    width: MirWidth::Byte,
                    ..
                } => mem(m, &slots),
                MirOp::Store {
                    dst: MirAddr::Direct(m),
                    src,
                    width: MirWidth::Byte,
                } => {
                    value(src, &slots)?;
                    mem(m, &slots);
                }
                MirOp::Move {
                    src,
                    width: MirWidth::Byte,
                    ..
                }
                | MirOp::Unary {
                    src,
                    width: MirWidth::Byte,
                    ..
                } => value(src, &slots)?,
                MirOp::Binary {
                    left,
                    right,
                    width: MirWidth::Byte,
                    ..
                }
                | MirOp::Compare {
                    left,
                    right,
                    width: MirWidth::Byte,
                    ..
                } => {
                    value(left, &slots)?;
                    value(right, &slots)?;
                }
                _ => {
                    let mut unsupported = false;
                    super::spills::visit_op_mems(op, &mut |m| {
                        unsupported |= matches!(m, MirMem::ZeroPage(_))
                    });
                    if unsupported {
                        return None;
                    }
                }
            }
        }
        if crate::mir6502::analysis::effects::classify_terminator(&block.terminator)
            .homes
            .reads
            .iter()
            .any(|home| matches!(home, MirHomeByte::VirtualZeroPage { .. }))
        {
            return None;
        }
        Some(block)
    }
    let old_block = old.blocks.iter().find(|b| b.id == id);
    let new_block = new.blocks.iter().find(|b| b.id == id);
    old_block == new_block
        || normalized(old, id)
            .zip(normalized(new, id))
            .is_some_and(|(a, b)| a == b)
}

/// Prove every expanded region's worst path beats the old region's best
/// path, grouped by exit and intervening opaque calls. Cycles and unsupported
/// code inside a region are unknown (never guessed or assigned zero cost).
pub(super) fn saving(
    before: &Image,
    after: &Image,
    old_caller: &MirRoutine,
    new_caller: &MirRoutine,
    leaf: RoutineId,
    origins: &BTreeMap<MirBlockId, MirBlockId>,
    sites: &BTreeMap<MirBlockId, usize>,
) -> Option<u32> {
    let leaf_ranges = before
        .blocks
        .iter()
        .filter(|(r, _, _)| *r == leaf)
        .map(|(_, _, range)| range.clone())
        .collect::<Vec<_>>();
    let leaf_entry = before
        .entries
        .iter()
        .find_map(|(pc, r)| (*r == leaf).then_some(*pc))?;
    let mut walker = Walk {
        image: before,
        boundaries: BTreeMap::new(),
        allowed: leaf_ranges,
        leaf: None,
        visiting: BTreeSet::new(),
        memo: BTreeMap::new(),
        fuel: 4096,
    };
    let leaf_paths = walker.at(leaf_entry)?;
    // A helper-bearing leaf is admitted only with a separate typed input
    // witness from expansion. Its ordered events, ABI and effects must also
    // survive final materialization. Ordinary nested calls remain unsupported.
    if leaf_paths.keys().any(|exit| {
        exit.block.is_some()
            || exit.calls.iter().any(|id| {
                before
                    .helpers
                    .get(id)
                    .is_none_or(|decl| !decl.helper.is_wide())
            })
    }) {
        return None;
    }
    let starts = |image: &Image| -> BTreeMap<_, _> {
        image
            .blocks
            .iter()
            .filter(|(r, b, _)| *r == old_caller.id && origins.get(b) == Some(b))
            .map(|(_, b, range)| (*b, range.start))
            .collect()
    };
    let old_starts = starts(before);
    let new_starts = starts(after);
    if old_starts.keys().ne(new_starts.keys()) || sites.keys().any(|b| !old_starts.contains_key(b))
    {
        return None;
    }
    let mut total = 0;
    for (id, start) in &old_starts {
        let paths =
            |image: &Image, starts: &BTreeMap<MirBlockId, usize>, entry: usize, original: bool| {
                let allowed = image
                    .blocks
                    .iter()
                    // Materialization can introduce compare/continuation
                    // blocks. Walk them until the next original boundary;
                    // their generated IDs need no guessed source identity.
                    .filter(|(r, _, _)| *r == old_caller.id)
                    .map(|(_, _, range)| range.clone())
                    .collect();
                let mut walker = Walk {
                    image,
                    boundaries: starts.iter().map(|(b, p)| (*p, *b)).collect(),
                    allowed,
                    leaf: original.then_some((leaf, leaf_paths.clone())),
                    visiting: BTreeSet::new(),
                    memo: BTreeMap::new(),
                    fuel: 16384,
                };
                walker.at(entry)
            };
        let old = paths(before, &old_starts, *start, true)?;
        let new = paths(after, &new_starts, new_starts[id], false)?;
        if old.keys().ne(new.keys()) {
            return None;
        }
        let count = sites.get(id).copied().unwrap_or(0) as u32;
        // Identical unaffected MIR regions can retain identical path estimates.
        if count == 0
            && old == new
            && equivalent_private_block(old_caller, new_caller, *id)
            && old_caller
                .blocks
                .iter()
                .filter(|b| !origins.contains_key(&b.id))
                .eq(new_caller
                    .blocks
                    .iter()
                    .filter(|b| !origins.contains_key(&b.id)))
            && old
                .keys()
                .all(|exit| helper_relocation_penalty(before, after, &exit.calls) == Some(0))
        {
            continue;
        }
        let saved = old
            .iter()
            .map(|(exit, bounds)| {
                let penalty = helper_relocation_penalty(before, after, &exit.calls)?;
                Some(i64::from(bounds.min) - i64::from(new[exit].max) - i64::from(penalty))
            })
            .collect::<Option<Vec<_>>>()?
            .into_iter()
            .min()?;
        if saved < i64::from(count * MIN_CYCLES_PER_SITE) {
            // Known paths that fail the saving threshold differ from unknown
            // cost/control, so requested-inline reports can explain the fallback.
            return Some(0);
        }
        total += saved as u32;
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn costs(
        bytes: &[u8],
        origin: u16,
        entries: BTreeMap<usize, RoutineId>,
        leaf: Option<(RoutineId, Paths)>,
    ) -> Option<Paths> {
        let image = Image {
            bytes: bytes.to_vec(),
            blocks: vec![],
            entries,
            helpers: BTreeMap::new(),
            nested_arithmetic: BTreeMap::new(),
            origin,
        };
        Walk {
            image: &image,
            boundaries: BTreeMap::new(),
            allowed: vec![0..bytes.len()],
            leaf,
            visiting: BTreeSet::new(),
            memo: BTreeMap::new(),
            fuel: 4096,
        }
        .at(0)
    }

    #[test]
    fn path_costs_include_taken_branch_pages_and_returns() {
        let bytes = [0xd0, 1, 0xea, 0x60]; // BNE +1; NOP; RTS
        let plain = costs(&bytes, 0x2000, BTreeMap::new(), None).unwrap();
        assert_eq!(
            plain[&Exit {
                block: None,
                calls: vec![]
            }],
            Bounds { min: 9, max: 10 }
        );
        let crossing = costs(&bytes, 0x20fd, BTreeMap::new(), None).unwrap();
        assert_eq!(
            crossing[&Exit {
                block: None,
                calls: vec![]
            }],
            Bounds { min: 10, max: 10 }
        );
    }

    #[test]
    fn call_cost_includes_the_executed_leaf_not_just_jsr() {
        let entries = BTreeMap::from([(0x100, RoutineId(1))]);
        let paths = costs(
            &[0x20, 0x00, 0x21, 0x60],
            0x2000,
            entries,
            Some((
                RoutineId(1),
                BTreeMap::from([(
                    Exit {
                        block: None,
                        calls: vec![],
                    },
                    Bounds { min: 10, max: 12 },
                )]),
            )),
        )
        .unwrap();
        assert_eq!(
            paths[&Exit {
                block: None,
                calls: vec![]
            }],
            Bounds { min: 22, max: 24 }
        );
    }

    #[test]
    fn only_private_block_local_zero_page_renaming_preserves_correspondence() {
        let ast =
            crate::parser::parse(&crate::lexer::tokenize("PROC Main() RETURN").unwrap()).unwrap();
        let sem =
            crate::semantic::ir::lower_program(&ast, &crate::semantic::analyze(&ast).unwrap());
        let nir = crate::nir::lower_program(&sem);
        let mut old = crate::mir6502::lower_program(&nir)
            .unwrap()
            .routines
            .remove(0);
        let id = old.blocks[0].id;
        old.blocks[0].ops = vec![
            MirOp::LoadImm {
                dst: MirDef::Reg(MirReg::A),
                value: 7,
                width: MirWidth::Byte,
            },
            MirOp::Store {
                dst: MirAddr::Direct(MirMem::ZeroPage(MirZpSlot(4))),
                src: MirValue::Def(MirDef::Reg(MirReg::A)),
                width: MirWidth::Byte,
            },
            MirOp::Load {
                dst: MirDef::Reg(MirReg::A),
                src: MirAddr::Direct(MirMem::ZeroPage(MirZpSlot(4))),
                width: MirWidth::Byte,
            },
        ];
        let mut new = old.clone();
        if let MirOp::Store { dst, .. } = &mut new.blocks[0].ops[1] {
            *dst = MirAddr::Direct(MirMem::ZeroPage(MirZpSlot(2)));
        }
        if let MirOp::Load { src, .. } = &mut new.blocks[0].ops[2] {
            *src = MirAddr::Direct(MirMem::ZeroPage(MirZpSlot(2)));
        }
        assert!(equivalent_private_block(&old, &new, id));
        if let MirOp::LoadImm { value, .. } = &mut new.blocks[0].ops[0] {
            *value = 8;
        }
        assert!(!equivalent_private_block(&old, &new, id));
        old.blocks[0].ops.remove(1);
        new.blocks[0].ops.remove(1);
        new.blocks[0].ops[0] = old.blocks[0].ops[0].clone();
        assert!(
            !equivalent_private_block(&old, &new, id),
            "live incoming homes cannot be renamed"
        );
    }

    #[test]
    fn relocated_helper_loops_use_the_emitted_counter_and_taken_page_penalty() {
        let body = [0xa2, 8, 0xca, 0xd0, 0xfd, 0x60]; // LDX #8; DEX; BNE; RTS
        assert_eq!(
            relative_layout_penalty(&body, 0x20fe, 0x20fd, None),
            Some(7)
        );
        assert_eq!(
            relative_layout_penalty(&body, 0x20fd, 0x20fe, None),
            Some(0)
        );
        assert_eq!(
            relative_layout_penalty(&[0xd0, 0xfe, 0x60], 0x2000, 0x2100, None),
            None
        );
        let mut multiply = crate::integer6502::wide::multiply();
        multiply.push(0x60);
        assert_eq!(
            relative_layout_penalty(&multiply, 0x2000, 0x2100, None),
            Some(0)
        );
    }

    #[test]
    fn unknown_control_and_internal_cycles_are_not_free() {
        for bytes in [
            &[0x00, 0x60][..],
            &[0x6c, 0x00, 0x20][..],
            &[0x4c, 0x00, 0x20][..],
        ] {
            assert!(costs(bytes, 0x2000, BTreeMap::new(), None).is_none());
        }
        assert_eq!(
            instruction_cycles("LDA", AddressingMode::AbsoluteX),
            Some(5)
        );
        assert_eq!(
            instruction_cycles("STA", AddressingMode::IndirectIndexedY),
            Some(6)
        );
        assert_eq!(instruction_cycles("BRK", AddressingMode::Implied), None);
    }
}
