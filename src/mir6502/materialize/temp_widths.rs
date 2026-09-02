use crate::mir6502::ir::{MirCondDest, MirDef, MirOp, MirRoutine, MirTempId, MirWidth};
use std::collections::BTreeMap;

pub(super) fn collect_routine_temp_widths(routine: &MirRoutine) -> BTreeMap<MirTempId, MirWidth> {
    let mut widths = BTreeMap::new();
    for block in &routine.blocks {
        for param in &block.params {
            note_width(&mut widths, param.dest, param.width);
        }
        merge_temp_widths(&mut widths, collect_temp_widths(&block.ops));
        for op in &block.ops {
            if let MirOp::LoadIndirect { dst, .. } = op {
                note_temp_width(&mut widths, dst, MirWidth::Byte);
            }
        }
    }
    widths
}

pub(super) fn collect_temp_widths(ops: &[MirOp]) -> BTreeMap<MirTempId, MirWidth> {
    let mut widths = BTreeMap::new();
    for op in ops {
        match op {
            MirOp::LoadImm { dst, width, .. }
            | MirOp::Load { dst, width, .. }
            | MirOp::Move { dst, width, .. }
            | MirOp::Unary { dst, width, .. }
            | MirOp::Binary { dst, width, .. } => {
                note_temp_width(&mut widths, dst, *width);
            }
            MirOp::LeaAddr {
                dst,
                width: MirWidth::Word,
                ..
            } => note_temp_width(&mut widths, dst, MirWidth::Word),
            MirOp::Extend { dst, to_width, .. } => note_temp_width(&mut widths, dst, *to_width),
            MirOp::Truncate { dst, to_width, .. } => note_temp_width(&mut widths, dst, *to_width),
            MirOp::Call {
                result: Some(result),
                ..
            } => note_temp_width(&mut widths, &result.dst, result.width),
            MirOp::Compare {
                dst: MirCondDest::Temp(id),
                ..
            }
            | MirOp::CompareDirectIndexedBytes {
                dst: MirCondDest::Temp(id),
                ..
            } => {
                widths.insert(*id, MirWidth::Byte);
            }
            MirOp::RuntimeHelper { .. }
            | MirOp::MaterializeAddress { .. }
            | MirOp::MaterializeIndexedAddress { .. }
            | MirOp::AdvanceAddress { .. }
            | MirOp::LoadIndirect { .. }
            | MirOp::StoreIndirect { .. }
            | MirOp::CopyIndirectWord { .. }
            | MirOp::CopyDirectWordToIndirect { .. }
            | MirOp::CopyIndirectBytesToFixedZp { .. }
            | MirOp::AbsoluteWordSubToIndirect { .. }
            | MirOp::IndirectByteCompound { .. }
            | MirOp::IndirectWordCompound { .. }
            | MirOp::UpdateMem { .. }
            | MirOp::UpdateReg { .. }
            | MirOp::UpdateIndexedMem { .. }
            | MirOp::BinaryDirectIndexedByte { .. }
            | MirOp::AddByteToWordMem { .. }
            | MirOp::SubByteFromWordMem { .. }
            | MirOp::OffsetPointerByIndirectByte { .. }
            | MirOp::Store { .. }
            | MirOp::CopyBytes { .. }
            | MirOp::Barrier { .. }
            | MirOp::MachineBlock { .. }
            | MirOp::LeaAddr { .. }
            | MirOp::Call { result: None, .. }
            | MirOp::Compare { .. }
            | MirOp::CompareDirectIndexedBytes { .. }
            | MirOp::CompareIndirectBytes { .. }
            | MirOp::CompareIndirectWords { .. }
            | MirOp::PackedRealCompare { .. }
            | MirOp::PackedRealCopy { .. } => {}
        }
    }
    widths
}

pub(super) fn merge_temp_widths(
    widths: &mut BTreeMap<MirTempId, MirWidth>,
    additional: BTreeMap<MirTempId, MirWidth>,
) {
    for (id, width) in additional {
        note_width(widths, id, width);
    }
}

fn note_temp_width(widths: &mut BTreeMap<MirTempId, MirWidth>, def: &MirDef, width: MirWidth) {
    match def {
        MirDef::VTemp(id) => {
            widths.insert(*id, width);
        }
        MirDef::VTempByte { id, byte } => {
            let lane_width = if *byte == 0 {
                MirWidth::Byte
            } else {
                MirWidth::Word
            };
            widths
                .entry(*id)
                .and_modify(|existing| {
                    if lane_width == MirWidth::Word {
                        *existing = MirWidth::Word;
                    }
                })
                .or_insert(lane_width);
        }
        MirDef::Reg(_) => {}
    }
}

fn note_width(widths: &mut BTreeMap<MirTempId, MirWidth>, id: MirTempId, width: MirWidth) {
    widths
        .entry(id)
        .and_modify(|existing| {
            if width == MirWidth::Word {
                *existing = MirWidth::Word;
            }
        })
        .or_insert(width);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir6502::ir::{
        MirBlock, MirBlockId, MirBlockParam, MirEffects, MirFrame, MirRoutineAbi, MirTerminator,
        RoutineId,
    };

    #[test]
    fn routine_widths_retain_typed_block_parameters() {
        let routine = MirRoutine {
            id: RoutineId(0),
            name: "Loop".to_string(),
            abi: MirRoutineAbi::Action,
            frame: MirFrame::default(),
            temps: Vec::new(),
            blocks: vec![MirBlock {
                id: MirBlockId(0),
                label: "loop".to_string(),
                params: vec![
                    MirBlockParam {
                        dest: MirTempId(0),
                        width: MirWidth::Byte,
                    },
                    MirBlockParam {
                        dest: MirTempId(1),
                        width: MirWidth::Word,
                    },
                ],
                ops: Vec::new(),
                terminator: MirTerminator::Return,
            }],
            effects: MirEffects::default(),
        };

        let widths = collect_routine_temp_widths(&routine);
        assert_eq!(widths.get(&MirTempId(0)), Some(&MirWidth::Byte));
        assert_eq!(widths.get(&MirTempId(1)), Some(&MirWidth::Word));
    }
}
