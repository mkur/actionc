//! Region proof for a changing, unexposed local pointer in an entered scan.
use super::*;
use crate::mir6502::analysis::{
    posthome::PostHomeAnalysisSnapshot, private_cells, sites::MirRoutineGeneration,
};
use crate::mir6502::ir::*;

pub(super) fn try_promote(
    program: &MirProgram,
    r: &MirRoutine,
    map: &MirPhysicalMemory,
    calls: &CallWrites,
    known: &MirKnownCalleeSummaries,
) -> Option<MirRoutine> {
    if !matches!(r.abi, MirRoutineAbi::Action | MirRoutineAbi::ProgramEntry)
        || private_cells::frame_exposed_in_data(program, r.id)
        || r.blocks.iter().flat_map(|b| &b.ops).any(|op| {
            crate::mir6502::analysis::effects::classify_op(op)
                .memory
                .address_references
                .iter()
                .any(|m| matches!(m, MirMem::Local { .. } | MirMem::Param { .. }))
        })
    {
        return None;
    }
    let regions: Vec<_> = retention::entered_loops(r, map, calls)
        .into_iter()
        .filter(|region| {
            r.blocks
                .iter()
                .find(|b| b.id == region.body)
                .is_some_and(|b| {
                    b.ops.iter().any(|op| {
                        matches!(
                            op,
                            MirOp::UpdateMem {
                                op: MirUpdateOp::Inc,
                                mem: MirMem::Local { .. },
                                width: MirWidth::Word
                            }
                        )
                    })
                })
        })
        .collect();
    if regions.is_empty() {
        return None;
    }
    let facts = retention::ResidentFacts::analyze(r, map, calls)?;
    let snapshot =
        PostHomeAnalysisSnapshot::new_with_known_callees(r, MirRoutineGeneration::initial(), known)
            .ok()?;
    let c = PostHomeRewriteContext::new(&snapshot);
    for region in regions {
        let body = r.blocks.iter().find(|b| b.id == region.body)?;
        let candidates: Vec<_> = body
            .ops
            .iter()
            .filter_map(|op| match op {
                MirOp::UpdateMem {
                    op: MirUpdateOp::Inc,
                    mem: MirMem::Local { id, offset: 0 },
                    width: MirWidth::Word,
                } => Some(*id),
                _ => None,
            })
            .collect();
        // Multiple competing changing pointers require joint pair placement.
        let [id] = candidates.as_slice() else {
            continue;
        };
        let low = MirMem::Local { id: *id, offset: 0 };
        let high = MirMem::Local { id: *id, offset: 1 };
        let Some(slot) = r.frame.locals.iter().find(|s| {
            s.base == MirStorageBase::Local(*id)
                && s.offset == 0
                && s.storage_size == 2
                && s.scalar_width == Some(MirWidth::Word)
                && s.storage == MirStorageClass::Scalar
        }) else {
            continue;
        };
        if slot.init.is_some()
            || r.frame
                .locals
                .iter()
                .any(|s| matches!(s.base,MirStorageBase::LocalAlias{target,..} if target==*id))
        {
            continue;
        }
        let Some(address) = map.address(r.id, &low) else {
            continue;
        };
        if map
            .address(r.id, &region.counter)
            .is_none_or(|counter| counter.wrapping_sub(address) < 2)
        {
            continue;
        }
        if !map.ordinary(r.id, &low) || !map.ordinary(r.id, &high) {
            continue;
        }
        let Some(base) = facts.constant_word(
            r,
            MirSite::Terminator {
                block: region.preheader,
            },
            &low,
            &high,
            map,
            calls,
        ) else {
            continue;
        };
        if !map.contains_write(base, region.iterations)
            || address.wrapping_sub(base) < region.iterations
            || address.wrapping_add(1).wrapping_sub(base) < region.iterations
        {
            continue;
        }
        if program.globals.iter().any(|g|matches!(g.backing,MirGlobalBacking::Absolute(start) if address.wrapping_sub(start)<g.storage_size || address.wrapping_add(1).wrapping_sub(start)<g.storage_size)) {continue;}
        if program.routines.iter().flat_map(|r|r.frame.locals.iter().chain(&r.frame.params)).any(|s|matches!(s.base,MirStorageBase::Absolute(start) if address.wrapping_sub(start.wrapping_add(s.offset))<s.storage_size || address.wrapping_add(1).wrapping_sub(start.wrapping_add(s.offset))<s.storage_size)) {continue;}
        let cfg = c.cfg();
        if cfg.predecessors(region.exit) != &BTreeSet::from([region.guard]) {
            continue;
        }
        let preheader = r.blocks.iter().find(|b| b.id == region.preheader)?;
        let Some(initializer)=preheader.ops.windows(4).enumerate().filter(|(_,w)| {
            matches!(&w[0],MirOp::Move {dst:MirDef::Reg(MirReg::A),src:MirValue::StorageAddrByte{..},width:MirWidth::Byte} | MirOp::LoadImm {dst:MirDef::Reg(MirReg::A),width:MirWidth::Byte,..})
            && matches!(&w[1],MirOp::Store {dst:MirAddr::Direct(m),src:MirValue::Def(MirDef::Reg(MirReg::A)),width:MirWidth::Byte} if *m==low)
            && matches!(&w[2],MirOp::Move {dst:MirDef::Reg(MirReg::A),src:MirValue::StorageAddrByte{..},width:MirWidth::Byte} | MirOp::LoadImm {dst:MirDef::Reg(MirReg::A),width:MirWidth::Byte,..})
            && matches!(&w[3],MirOp::Store {dst:MirAddr::Direct(m),src:MirValue::Def(MirDef::Reg(MirReg::A)),width:MirWidth::Byte} if *m==high)
        }).map(|(i,_)|i).last() else {continue;};
        if preheader.ops[initializer + 4..].iter().any(|op| {
            if matches!(
                op,
                MirOp::Call { .. }
                    | MirOp::RuntimeHelper { .. }
                    | MirOp::MachineBlock { .. }
                    | MirOp::Barrier { .. }
            ) {
                return true;
            }
            let e = crate::mir6502::analysis::effects::classify_op(op);
            e.memory.indirect_reads
                || e.memory.indirect_writes
                || e.memory.may_write_any
                || e.memory.opaque
                || e.memory
                    .direct_reads
                    .iter()
                    .chain(&e.memory.direct_writes)
                    .any(|range| {
                        map.address(r.id, &range.base).is_none_or(|a| {
                            address.wrapping_sub(a) < range.bytes
                                || address.wrapping_add(1).wrapping_sub(a) < range.bytes
                        })
                    })
        }) {
            continue;
        }
        let mut windows = BTreeMap::new();
        let mut pair = None;
        let mut valid = true;
        let mut prepared = false;
        let mut i = 0;
        while i < body.ops.len() {
            if let Some(window) = body.ops.get(i..i + 4)
                && let Some(candidate) = record_bases::setup_pair(window)
                && matches!(&window[0],MirOp::Load{dst:MirDef::Reg(MirReg::A),src:MirAddr::Direct(m),width:MirWidth::Byte} if *m==low)
                && matches!(&window[2],MirOp::Load{dst:MirDef::Reg(MirReg::A),src:MirAddr::Direct(m),width:MirWidth::Byte} if *m==high)
            {
                if pair.is_some_and(|p| p != candidate) {
                    valid = false;
                    break;
                }
                pair = Some(candidate);
                windows.insert(i, 4);
                prepared = true;
                i += 4;
                continue;
            }
            let op = &body.ops[i];
            if matches!(op,MirOp::UpdateMem{op:MirUpdateOp::Inc,mem,width:MirWidth::Word} if *mem==low)
            {
                prepared = false;
                i += 1;
                continue;
            }
            if matches!(
                op,
                MirOp::Call { .. }
                    | MirOp::RuntimeHelper { .. }
                    | MirOp::MachineBlock { .. }
                    | MirOp::Barrier { .. }
            ) {
                valid = false;
                break;
            }
            let e = crate::mir6502::analysis::effects::classify_op(op);
            if e.memory.indirect_writes
                || e.memory.may_write_any
                || e.memory.opaque
                || !e.addresses.pair_writes.is_empty()
            {
                valid = false;
                break;
            }
            if e.memory.indirect_reads {
                if !prepared
                    || !matches!(op,MirOp::LoadIndirect {consumer:MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed{lo}),offset:0,..} if Some(lo.0)==pair)
                {
                    valid = false;
                    break;
                }
            }
            if e.memory
                .direct_reads
                .iter()
                .chain(&e.memory.direct_writes)
                .any(|range| {
                    map.address(r.id, &range.base).is_none_or(|a| {
                        address.wrapping_sub(a) < range.bytes
                            || address.wrapping_add(1).wrapping_sub(a) < range.bytes
                    })
                })
            {
                valid = false;
                break;
            }
            // No explicit use of the chosen pair is admitted outside its loads.
            if !e.memory.indirect_reads && e.addresses.pair_reads.iter().any(|home|matches!(home,MirHomeByte::FixedZeroPage(lo) if pair.is_some_and(|p|lo.0==p || lo.0==p+1))) {valid=false;break;}
            i += 1;
        }
        let Some(pair) = pair.filter(|_| valid && !windows.is_empty()) else {
            continue;
        };
        let aliases_pair=body.ops.iter().enumerate().any(|(index,op)| {
            if windows.keys().any(|start|index>=*start && index<*start+4) || matches!(op,MirOp::LoadIndirect{consumer:MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed{lo}),offset:0,..} if lo.0==pair) {return false;}
            let e=crate::mir6502::analysis::effects::classify_op(op);
            e.memory.direct_reads.iter().chain(&e.memory.direct_writes).any(|range|map.address(r.id,&range.base).is_none_or(|a|u16::from(pair).wrapping_sub(a)<range.bytes || (u16::from(pair)+1).wrapping_sub(a)<range.bytes))
        });
        if aliases_pair {
            continue;
        }
        if r.blocks
            .iter()
            .find(|b| b.id == region.guard)?
            .ops
            .iter()
            .any(|op| {
                crate::mir6502::analysis::effects::classify_op(op)
                    .memory
                    .direct_reads
                    .iter()
                    .any(|range| {
                        map.address(r.id, &range.base).is_none_or(|a| {
                            address.wrapping_sub(a) < range.bytes
                                || address.wrapping_add(1).wrapping_sub(a) < range.bytes
                        })
                    })
            })
        {
            continue;
        }
        let consumer = MirAddressConsumer::IndirectIndexedY(MirPointerPair::Fixed {
            lo: MirFixedZpSlot(pair),
        });
        let entry = c.point(MirSite::Terminator {
            block: region.preheader,
        });
        let exit = c.point(MirSite::BlockEntry { block: region.exit });
        if ![entry, exit].into_iter().all(|point| {
            matches!(
                c.register_dead_after(MirReg::A, point),
                MirProof::Proven(())
            ) && matches!(
                c.flags_dead_after(
                    MirFlagSet {
                        n: true,
                        z: true,
                        ..Default::default()
                    },
                    point
                ),
                MirProof::Proven(())
            )
        }) {
            continue;
        }
        if !matches!(
            c.pointer_pair_dead_after(consumer, entry),
            MirProof::Proven(())
        ) || !matches!(
            c.pointer_pair_dead_after(
                consumer,
                c.point(MirSite::Terminator {
                    block: region.guard
                })
            ),
            MirProof::Proven(())
        ) {
            continue;
        }
        if !matches!(
            c.pointer_pair_dead_after(
                consumer,
                c.point(MirSite::Op {
                    block: preheader.id,
                    op_index: initializer
                })
            ),
            MirProof::Proven(())
        ) {
            continue;
        }
        let mut selected = r.clone();
        let pre = selected
            .blocks
            .iter_mut()
            .find(|b| b.id == region.preheader)?;
        pre.ops[initializer + 1] = store(pair);
        pre.ops[initializer + 3] = store(pair + 1);
        let mut replacement = vec![];
        let mut i = 0;
        while i < body.ops.len() {
            if windows.contains_key(&i) {
                let end = c.point(MirSite::Op {
                    block: body.id,
                    op_index: i + 3,
                });
                if !matches!(c.register_dead_after(MirReg::A, end), MirProof::Proven(()))
                    || !matches!(
                        c.flags_dead_after(
                            MirFlagSet {
                                n: true,
                                z: true,
                                ..Default::default()
                            },
                            end
                        ),
                        MirProof::Proven(())
                    )
                {
                    replacement.push(load(MirMem::FixedZeroPage(MirFixedZpSlot(pair + 1))));
                }
                i += 4;
            } else {
                let op = &body.ops[i];
                replacement.push(if matches!(op,MirOp::UpdateMem{op:MirUpdateOp::Inc,mem,width:MirWidth::Word} if *mem==low) {MirOp::UpdateMem{op:MirUpdateOp::Inc,mem:MirMem::FixedZeroPage(MirFixedZpSlot(pair)),width:MirWidth::Word}} else {op.clone()});
                i += 1;
            }
        }
        selected
            .blocks
            .iter_mut()
            .find(|b| b.id == region.body)?
            .ops = replacement;
        let target = selected.blocks.iter_mut().find(|b| b.id == region.exit)?;
        let mut writeback = vec![
            load(MirMem::FixedZeroPage(MirFixedZpSlot(pair))),
            save(low.clone()),
            load(MirMem::FixedZeroPage(MirFixedZpSlot(pair + 1))),
            save(high.clone()),
        ];
        // Existing compare producers keep their stable operation references.
        if let MirTerminator::Branch {
            cond: MirCond::FusedCompare { producer, .. },
            ..
        } = &mut target.terminator
        {
            if producer.block == target.id {
                producer.op_index += writeback.len();
            }
        }
        writeback.append(&mut target.ops);
        target.ops = writeback;
        return Some(selected);
    }
    None
}
fn load(mem: MirMem) -> MirOp {
    MirOp::Load {
        dst: MirDef::Reg(MirReg::A),
        src: MirAddr::Direct(mem),
        width: MirWidth::Byte,
    }
}
fn store(slot: u8) -> MirOp {
    save(MirMem::FixedZeroPage(MirFixedZpSlot(slot)))
}
fn save(mem: MirMem) -> MirOp {
    MirOp::Store {
        dst: MirAddr::Direct(mem),
        src: MirValue::Def(MirDef::Reg(MirReg::A)),
        width: MirWidth::Byte,
    }
}
