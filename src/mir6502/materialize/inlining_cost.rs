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
        Some(Self {
            bytes,
            blocks: summary.block_ranges,
            entries,
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

fn merge(into: &mut Paths, paths: Paths, cost: Bounds, call: Option<RoutineId>) -> Option<()> {
    for (mut exit, bound) in paths {
        if let Some(id) = call {
            exit.calls.insert(0, id);
        }
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
    leaf: Option<(RoutineId, Bounds)>,
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
                merge(&mut result, terminal(None), fixed(6), None)?;
            }
            "JSR" | "JMP" if mode == AddressingMode::Absolute => {
                let target = self.image.target(pc)?;
                if name == "JMP"
                    && (self.boundaries.contains_key(&target)
                        || self.allowed.iter().any(|r| r.contains(&target)))
                {
                    let paths = self.next(target)?;
                    merge(&mut result, paths, fixed(3), None)?;
                } else if let Some(id) = self.image.entries.get(&target).copied() {
                    let paths = if name == "JSR" {
                        self.next(pc + len)?
                    } else {
                        terminal(None)
                    };
                    let (cost, call) =
                        if let Some((_, cost)) = self.leaf.filter(|(leaf, _)| *leaf == id) {
                            (
                                Bounds {
                                    min: cost.min + if name == "JSR" { 6 } else { 3 },
                                    max: cost.max + if name == "JSR" { 6 } else { 3 },
                                },
                                None,
                            )
                        } else {
                            // A tail call also supplies the caller's return; retain
                            // that return cost when pairing with a normal JSR.
                            (fixed(if name == "JSR" { 6 } else { 3 + 6 }), Some(id))
                        };
                    merge(&mut result, paths, cost, call)?;
                } else if name == "JMP" {
                    let paths = self.next(target)?;
                    merge(&mut result, paths, fixed(3), None)?;
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
                merge(&mut result, paths, fixed(2), None)?;
                let paths = self.next(target)?;
                merge(&mut result, paths, fixed(3 + u32::from(crossed)), None)?;
            }
            _ => {
                let cost = instruction_cycles(name, mode)?;
                let paths = self.next(pc + len)?;
                merge(&mut result, paths, fixed(cost), None)?;
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
    let leaf_cost = *leaf_paths.get(&Exit {
        block: None,
        calls: vec![],
    })?;
    if leaf_paths.len() != 1 {
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
                    .filter(|(r, b, _)| {
                        *r == old_caller.id
                            && if original {
                                b == id
                            } else {
                                origins.get(b) == Some(id)
                            }
                    })
                    .map(|(_, _, range)| range.clone())
                    .collect();
                let mut walker = Walk {
                    image,
                    boundaries: starts.iter().map(|(b, p)| (*p, *b)).collect(),
                    allowed,
                    leaf: original.then_some((leaf, leaf_cost)),
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
            && old_caller.blocks.iter().find(|b| b.id == *id)
                == new_caller.blocks.iter().find(|b| b.id == *id)
        {
            continue;
        }
        let saved = old
            .iter()
            .map(|(exit, bounds)| bounds.min.checked_sub(new[exit].max))
            .collect::<Option<Vec<_>>>()?
            .into_iter()
            .min()?;
        if saved < count * MIN_CYCLES_PER_SITE {
            return None;
        }
        total += saved;
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
        leaf: Option<(RoutineId, Bounds)>,
    ) -> Option<Paths> {
        let image = Image {
            bytes: bytes.to_vec(),
            blocks: vec![],
            entries,
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
            Some((RoutineId(1), Bounds { min: 10, max: 12 })),
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
