//! Bounded, whole-routine promotion of verified word home classes. No spilling.
use super::*;
use crate::nir::{NirIntegerRole, NirTypeKind};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const START: u16 = 32;
pub(crate) const END: u16 = 64;
pub(crate) fn word_offset(offset: u16) -> bool {
    (START..END).contains(&offset) && offset % 2 == 0
}
const _: () = {
    assert!(START as u32 >= abi::generated::DP_SCRATCH_OFFSET);
    assert!(END as u32 <= abi::generated::DP_SCRATCH_OFFSET + abi::generated::DP_SCRATCH_SIZE);
    assert!(END as u32 <= abi::generated::DP_OWNER_POINTER_OFFSET);
    // Current selector workspaces end at byte 30. Pointer leaves use 0..9.
    assert!(START > 30);
    assert!(START as u32 >= abi::generated::DP_POINTER2_OFFSET + 3);
};

fn is_word(ty: &crate::nir::NirType) -> bool {
    !ty.pointer
        && ty.width == Some(ByteSize::new(2))
        && matches!(ty.kind,
        NirTypeKind::Integer(i) if i.bits == 16 && i.role == NirIntegerRole::Ordinary)
}
fn is_bool(ty: &crate::nir::NirType) -> bool {
    !ty.pointer && ty.width == Some(ByteSize::new(1)) && ty.kind == NirTypeKind::Bool
}
fn edges(r: &Mir65816Routine) -> impl Iterator<Item = &Mir65816Edge> {
    r.blocks.iter().flat_map(|b| match &b.terminator {
        Mir65816Terminator::Goto(e) => vec![e],
        Mir65816Terminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![then_edge, else_edge],
        _ => vec![],
    })
}

/// Closed selector-effect whitelist: native word operations do not use selector
/// scratch. Branch truth tests may use RESULT (8..12), never resident scratch.
/// Any new MIR form must be reviewed here; absence of a Call is insufficient.
pub(super) fn admitted(r: &Mir65816Routine) -> bool {
    if !matches!(
        r.result_home,
        None | Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16))
    ) || r.frame.parameters.iter().any(
        |p| !matches!(p.incoming, Mir65816AbiHome::StackArgument { size, .. } if size.get() == 2),
    ) || r
        .frame
        .objects
        .iter()
        .any(|o| o.size.get() != 2 || o.addressable)
        || !r.temps.iter().any(|(_, t)| is_word(t))
        || r.temps.iter().any(|(_, t)| !is_word(t) && !is_bool(t))
    {
        return false;
    }
    let word_temp = |id| r.temps.iter().any(|(i, t)| *i == id && is_word(t));
    let bool_temp = |id| r.temps.iter().any(|(i, t)| *i == id && is_bool(t));
    let word = |v: &Mir65816Value| match v {
        Mir65816Value::U8(_) | Mir65816Value::U16(_) => true,
        Mir65816Value::Temp(id, w) => w.get() == 2 && word_temp(*id),
        Mir65816Value::Param(id) => r.frame.parameters.iter().any(|p| p.param == *id),
        _ => false,
    };
    let address = |a: &Mir65816Address| {
        a.index.is_none()
            && match a.base {
                Mir65816AddressBase::AutomaticFrame(id) => r.frame.objects.iter().any(|o| {
                    o.id == id
                        && !o.addressable
                        && a.displacement
                            .get()
                            .checked_add(2)
                            .is_some_and(|end| end <= o.size.get())
                }),
                Mir65816AddressBase::Parameter(id) => {
                    a.displacement.get() == 0 && r.frame.parameters.iter().any(|p| p.param == id)
                }
                _ => false,
            }
    };
    let mut bool_defs = BTreeSet::new();
    for b in &r.blocks {
        if b.params
            .iter()
            .any(|(id, w)| w.get() != 2 || !word_temp(*id))
        {
            return false;
        }
        for op in &b.ops {
            let safe = match op {
                Mir65816Op::Load {
                    dest,
                    width,
                    address: a,
                    volatile,
                } => width.get() == 2 && !volatile && word_temp(*dest) && address(a),
                Mir65816Op::Store {
                    width,
                    address: a,
                    value,
                    volatile,
                } => width.get() == 2 && !volatile && word(value) && address(a),
                Mir65816Op::Binary {
                    dest,
                    width,
                    operation,
                    left,
                    right,
                    ..
                } => {
                    width.get() == 2
                        && word_temp(*dest)
                        && matches!(operation, NirBinaryOp::Add | NirBinaryOp::Sub)
                        && word(left)
                        && word(right)
                }
                Mir65816Op::Compare {
                    dest,
                    width,
                    signed,
                    operation,
                    left,
                    right,
                } => {
                    bool_defs.insert(*dest);
                    width.get() == 2
                        && bool_temp(*dest)
                        && (!signed || matches!(operation, NirCompareOp::Eq | NirCompareOp::Ne))
                        && word(left)
                        && word(right)
                }
                _ => false,
            };
            if !safe {
                return false;
            }
        }
        match &b.terminator {
            Mir65816Terminator::Exit => return false,
            Mir65816Terminator::Return { value, .. } => match (value, r.result_home) {
                (None, None) => (),
                (Some(v), Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16)))
                    if word(v) =>
                {
                    ()
                }
                _ => return false,
            },
            Mir65816Terminator::Branch { condition, .. } => {
                if !word(condition)
                    && !matches!(condition, Mir65816Value::Temp(id,w) if w.get()==1 && bool_temp(*id))
                {
                    return false;
                }
            }
            _ => (),
        }
    }
    r.temps
        .iter()
        .all(|(id, t)| !is_bool(t) || bool_defs.contains(id))
        && edges(r).all(|e| e.args.iter().all(word))
}

impl AllocatedFrame {
    pub(super) fn promote_scalar(&mut self, r: &Mir65816Routine) -> Result<(), String> {
        // The caller has already verified the original stack/coalescing result.
        if !admitted(r) {
            return Ok(());
        }
        let offsets: BTreeSet<_> = self
            .temps
            .values()
            .filter(|h| h.slot().width == 2)
            .map(|h| h.slot().offset)
            .collect();
        if offsets.len() > usize::from((END - START) / 2) {
            return Ok(());
        }
        let mapping: BTreeMap<_, _> = offsets
            .into_iter()
            .enumerate()
            .map(|(i, o)| (o, START + i as u16 * 2))
            .collect();
        let mut trial = self.clone();
        for home in trial.temps.values_mut() {
            if home.slot().width == 2 {
                *home = Location::DirectPage(Slot {
                    offset: mapping[&home.slot().offset],
                    width: 2,
                });
            }
        }
        let end = trial
            .temps
            .values()
            .filter_map(|h| match h {
                Location::Stack(s) => Some(u32::from(s.offset) + u32::from(s.width) - 1),
                _ => None,
            })
            .fold(r.frame.extent.get(), u32::max);
        let mut cursor = end + 1;
        trial.edge_copies.clear();
        // Same home equalities and dependencies must retain the capture plan.
        let required = trial.staging_widths(r)?;
        if required != self.staging_widths(r)? {
            return Ok(());
        }
        for width in required {
            cursor = (cursor + 1) & !1;
            trial.edge_copies.push(Slot {
                offset: cursor as u16,
                width,
            });
            cursor += u32::from(width);
        }
        trial.extent = abi::stack::fixed_extent(ByteSize::new(cursor - 1))
            .map_err(|e| e.to_string())?
            .get() as u16;
        trial.spill_bytes = trial.extent - r.frame.extent.get() as u16;
        trial.peak_below_entry = trial.extent; // Admission excludes every returning call.
        trial.verify_scalar_dp(r)?;
        for e in edges(r) {
            let before = self.word_copies(r, e, 0)?;
            let after = trial.word_copies(r, e, 0)?;
            match (before, after) {
                (None, None) if e.args.is_empty() => (),
                (Some(b), Some(a)) if a.cost().0 <= b.cost().0 && a.cost().1 <= b.cost().1 => (),
                _ => return Ok(()),
            }
        }
        *self = trial;
        Ok(())
    }

    /// Mixed-space geometry plus closed-operation liveness and effect checks.
    /// This deliberately does not weaken the all-stack verifier.
    pub fn verify_scalar_dp(&self, r: &Mir65816Routine) -> Result<(), String> {
        if !admitted(r) || self.temps.len() != r.temps.len() {
            return Err("invalid scalar DP admission".into());
        }
        let graph = super::liveness::interference(r)?;
        let mut end = r.frame.extent.get();
        for (id, ty) in &r.temps {
            let home = *self.temps.get(id).ok_or("missing scalar DP temporary")?;
            match home {
                Location::DirectPage(s) if is_word(ty) && s.width == 2 && word_offset(s.offset) => {
                    ()
                }
                Location::Stack(s)
                    if is_bool(ty)
                        && s.width == 1
                        && u32::from(s.offset) > r.frame.extent.get() =>
                {
                    abi::stack::access_displacement(
                        ByteOffset::new(s.offset.into()),
                        ByteSize::new(1),
                        ByteSize::ZERO,
                    )
                    .map_err(|e| e.to_string())?;
                    end = end.max(s.offset.into());
                }
                _ => return Err("invalid scalar DP home".into()),
            }
            for other in &graph[id] {
                if home.overlaps(*self.temps.get(other).ok_or("missing scalar DP temporary")?) {
                    return Err("overlapping live scalar DP temporaries".into());
                }
            }
        }
        let required = self.staging_widths(r)?;
        if required.len() != self.edge_copies.len() {
            return Err("invalid scalar DP staging count".into());
        }
        let mut cursor = end + 1;
        for (&s, w) in self.edge_copies.iter().zip(required) {
            cursor = (cursor + 1) & !1;
            if w != 2 || s.width != 2 || u32::from(s.offset) != cursor {
                return Err("invalid scalar DP staging geometry".into());
            }
            abi::stack::access_displacement(
                ByteOffset::new(cursor),
                ByteSize::new(2),
                ByteSize::ZERO,
            )
            .map_err(|e| e.to_string())?;
            cursor += 2;
        }
        let extent = abi::stack::fixed_extent(ByteSize::new(cursor - 1))
            .map_err(|e| e.to_string())?
            .get();
        if u32::from(self.extent) != extent
            || u32::from(self.spill_bytes) != extent - r.frame.extent.get()
            || self.peak_below_entry != self.extent
        {
            return Err("invalid scalar DP frame accounting".into());
        }
        for p in &r.frame.parameters {
            let Mir65816AbiHome::StackArgument { offset, size, .. } = p.incoming else {
                return Err("invalid scalar parameter home".into());
            };
            abi::stack::incoming_displacement(ByteSize::new(extent), offset, size)
                .map_err(|e| e.to_string())?;
        }
        for e in edges(r) {
            if let Some(plan) = self.word_copies(r, e, 0)? {
                self.word_staging(&plan, 0)?;
            } else if !e.args.is_empty() {
                return Err("invalid scalar DP edge".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn example(words: u32) -> Mir65816Routine {
        let mut r = super::super::select::word_tests::program()
            .routines
            .remove(0);
        let ty = r.temps[0].1.clone();
        r.temps = (0..words).map(|i| (TempId(i), ty.clone())).collect();
        let mut ret = r.blocks[0].terminator.clone();
        if let Mir65816Terminator::Return { value, .. } = &mut ret {
            *value = Some(Mir65816Value::Temp(TempId(0), ByteSize::new(2)));
        }
        r.blocks = vec![
            Mir65816Block {
                id: BlockId(0),
                params: vec![],
                ops: vec![],
                terminator: Mir65816Terminator::Goto(Mir65816Edge {
                    target: BlockId(1),
                    args: (0..words).map(|i| Mir65816Value::U16(i as u16)).collect(),
                }),
            },
            Mir65816Block {
                id: BlockId(1),
                params: (0..words).map(|i| (TempId(i), ByteSize::new(2))).collect(),
                ops: vec![],
                terminator: ret,
            },
        ];
        r
    }
    #[test]
    fn whole_home_classes_fit_or_fall_back_without_partial_promotion() {
        for n in [1, 16, 17] {
            let r = example(n);
            let f = AllocatedFrame::new(&r).unwrap();
            if n <= 16 {
                f.verify_scalar_dp(&r).unwrap();
                assert!(f.verify_stack(&r).is_err());
                assert_eq!((f.extent, f.spill_bytes, f.peak_below_entry), (0, 0, 0));
                for i in 0..n {
                    assert_eq!(
                        f.temps[&TempId(i)],
                        Location::DirectPage(Slot {
                            offset: 32 + 2 * i as u16,
                            width: 2
                        })
                    );
                }
            } else {
                f.verify_stack(&r).unwrap();
                assert!(f.verify_scalar_dp(&r).is_err());
                assert_eq!(
                    format!("{f:?}"),
                    format!("{:?}", AllocatedFrame::stack(&r).unwrap())
                );
            }
            assert_eq!(
                format!("{f:?}"),
                format!("{:?}", AllocatedFrame::new(&r).unwrap())
            );
        }
    }
    #[test]
    fn mixed_verifier_rejects_bad_geometry_interference_and_accounting() {
        let r = example(2);
        let f = AllocatedFrame::new(&r).unwrap();
        for offset in [0, 30, 31, 33, 63, 64, 254] {
            let mut bad = f.clone();
            bad.temps
                .insert(TempId(0), Location::DirectPage(Slot { offset, width: 2 }));
            assert!(bad.verify_scalar_dp(&r).is_err());
        }
        let mut bad = f.clone();
        bad.temps.insert(TempId(1), f.temps[&TempId(0)]);
        assert!(bad.verify_scalar_dp(&r).is_err());
        let mut bad = f.clone();
        bad.extent = 2;
        bad.spill_bytes = 2;
        bad.peak_below_entry = 2;
        assert!(bad.verify_scalar_dp(&r).is_err());
        let mut bad = f.clone();
        bad.edge_copies.push(Slot {
            offset: 2,
            width: 2,
        });
        assert!(bad.verify_scalar_dp(&r).is_err());
        let mut bad = f;
        bad.temps.remove(&TempId(0));
        assert!(bad.verify_scalar_dp(&r).is_err());
    }
    #[test]
    fn unsupported_selector_effects_and_memory_are_whole_routine_barriers() {
        let r = super::super::select::word_tests::program()
            .routines
            .remove(0);
        assert!(admitted(&r));
        for case in 0..9 {
            let mut bad = r.clone();
            match case {
                0 => {
                    if let Mir65816Op::Load { volatile, .. } = &mut bad.blocks[0].ops[0] {
                        *volatile = true;
                    }
                }
                1 => {
                    if let Mir65816Op::Binary { operation, .. } =
                        bad.blocks[0].ops.last_mut().unwrap()
                    {
                        *operation = NirBinaryOp::And;
                    }
                }
                2 => {
                    if let Mir65816Op::Load { address, .. } = &mut bad.blocks[0].ops[0] {
                        address.base = Mir65816AddressBase::Indirect(Mir65816Value::U24(0x120000));
                    }
                }
                3 => {
                    if let Mir65816Op::Load { address, .. } = &mut bad.blocks[0].ops[0] {
                        address.displacement = ByteOffset::new(u32::MAX);
                    }
                }
                4 => bad.blocks[0].terminator = Mir65816Terminator::Exit,
                5 => bad.temps[0].1.pointer = true,
                6 => bad.blocks[0].ops.push(Mir65816Op::AddressOf {
                    dest: TempId(0),
                    address: match &r.blocks[0].ops[0] {
                        Mir65816Op::Load { address, .. } => address.clone(),
                        _ => panic!(),
                    },
                    width: ByteSize::new(2),
                }),
                7 => {
                    if let Mir65816Op::Load { address, .. } = &mut bad.blocks[0].ops[0] {
                        address.index = Some(Mir65816Index {
                            value: Mir65816Value::U16(0),
                            stride: ByteSize::new(2),
                        });
                    }
                }
                _ => bad.temps[0].1.width = Some(ByteSize::new(4)),
            }
            assert!(!admitted(&bad), "{case}");
            let mut f = AllocatedFrame::stack(&r).unwrap();
            let before = format!("{f:?}");
            f.promote_scalar(&bad).unwrap();
            assert_eq!(format!("{f:?}"), before);
        }
    }
}
