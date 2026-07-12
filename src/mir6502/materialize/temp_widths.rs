use crate::mir6502::ir::{MirCondDest, MirDef, MirOp, MirTempId, MirWidth};
use std::collections::BTreeMap;

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
            } => {
                widths.insert(*id, MirWidth::Byte);
            }
            MirOp::RuntimeHelper { .. }
            | MirOp::MaterializeAddress { .. }
            | MirOp::MaterializeIndexedAddress { .. }
            | MirOp::AdvanceAddress { .. }
            | MirOp::LoadIndirect { .. }
            | MirOp::StoreIndirect { .. }
            | MirOp::IndirectByteCompound { .. }
            | MirOp::UpdateMem { .. }
            | MirOp::AddByteToWordMem { .. }
            | MirOp::SubByteFromWordMem { .. }
            | MirOp::Store { .. }
            | MirOp::Barrier { .. }
            | MirOp::MachineBlock { .. }
            | MirOp::LeaAddr { .. }
            | MirOp::Call { result: None, .. }
            | MirOp::Compare { .. } => {}
        }
    }
    widths
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
