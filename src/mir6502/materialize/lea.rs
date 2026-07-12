use super::layout::MaterializeLayout;
use super::values::{offset_mem, split_def};
use crate::mir6502::ir::{
    MirAddr, MirDef, MirMem, MirOp, MirReg, MirSpillId, MirValue, MirWidth, RoutineId,
};

pub(super) fn lower_lea_addrs_with_final_layout(
    routine_id: RoutineId,
    ops: Vec<MirOp>,
    layout: &MaterializeLayout,
) -> Vec<MirOp> {
    let mut out = Vec::new();
    for op in ops {
        match op {
            MirOp::LeaAddr {
                dst,
                target,
                width: MirWidth::Word,
            } => {
                if layout.is_descriptor_storage(routine_id, &target) {
                    lower_descriptor_pointer_to_def(dst, target, &mut out);
                    continue;
                }
                if !super::can_resolve_address_early(&target) {
                    lower_storage_address_to_def(dst, target, &mut out);
                    continue;
                }
                let Some(address) = layout.mem_address(routine_id, &target) else {
                    out.push(MirOp::LeaAddr {
                        dst,
                        target,
                        width: MirWidth::Word,
                    });
                    continue;
                };
                lower_address_to_def(dst, address, &mut out);
            }
            other => out.push(other),
        }
    }
    out
}

fn lower_descriptor_pointer_to_def(dst: MirDef, mem: MirMem, out: &mut Vec<MirOp>) {
    if let MirDef::VTemp(temp) = dst {
        let lo_spill = MirSpillId(temp.0.saturating_mul(2));
        let hi_spill = MirSpillId(temp.0.saturating_mul(2).saturating_add(1));
        super::materialize_value_to_mem(
            MirValue::PointerCell(mem.clone()),
            MirMem::Spill {
                id: lo_spill,
                offset: 0,
            },
            out,
        );
        super::materialize_value_to_mem(
            MirValue::PointerCell(offset_mem(&mem, 1)),
            MirMem::Spill {
                id: hi_spill,
                offset: 0,
            },
            out,
        );
        return;
    }
    if let Some((lo_dst, hi_dst)) = split_def(dst.clone()) {
        out.push(MirOp::Load {
            dst: lo_dst,
            src: MirAddr::Direct(mem.clone()),
            width: MirWidth::Byte,
        });
        out.push(MirOp::Load {
            dst: hi_dst,
            src: MirAddr::Direct(offset_mem(&mem, 1)),
            width: MirWidth::Byte,
        });
    } else {
        out.push(MirOp::LeaAddr {
            dst,
            target: mem,
            width: MirWidth::Word,
        });
    }
}

pub(super) fn lower_address_to_def(dst: MirDef, address: u16, out: &mut Vec<MirOp>) {
    if let MirDef::VTemp(temp) = dst {
        let lo_spill = MirSpillId(temp.0.saturating_mul(2));
        let hi_spill = MirSpillId(temp.0.saturating_mul(2).saturating_add(1));
        out.push(MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value: address & 0x00FF,
            width: MirWidth::Byte,
        });
        super::store_a_to_spill(out, lo_spill);
        out.push(MirOp::LoadImm {
            dst: MirDef::Reg(MirReg::A),
            value: address >> 8,
            width: MirWidth::Byte,
        });
        super::store_a_to_spill(out, hi_spill);
        return;
    }
    if let Some((lo_dst, hi_dst)) = split_def(dst.clone()) {
        out.push(MirOp::LoadImm {
            dst: lo_dst,
            value: address & 0x00FF,
            width: MirWidth::Byte,
        });
        out.push(MirOp::LoadImm {
            dst: hi_dst,
            value: address >> 8,
            width: MirWidth::Byte,
        });
    } else {
        out.push(MirOp::LoadImm {
            dst,
            value: address & 0x00FF,
            width: MirWidth::Byte,
        });
    }
}

fn lower_storage_address_to_def(dst: MirDef, mem: MirMem, out: &mut Vec<MirOp>) {
    let fallback = mem.clone();
    let lo = MirValue::StorageAddrByte {
        mem: mem.clone(),
        byte: 0,
    };
    let hi = MirValue::StorageAddrByte { mem, byte: 1 };
    if let MirDef::VTemp(temp) = dst {
        let lo_spill = MirSpillId(temp.0.saturating_mul(2));
        let hi_spill = MirSpillId(temp.0.saturating_mul(2).saturating_add(1));
        super::materialize_value_to_mem(
            lo,
            MirMem::Spill {
                id: lo_spill,
                offset: 0,
            },
            out,
        );
        super::materialize_value_to_mem(
            hi,
            MirMem::Spill {
                id: hi_spill,
                offset: 0,
            },
            out,
        );
        return;
    }
    out.push(MirOp::LeaAddr {
        dst,
        target: fallback,
        width: MirWidth::Word,
    });
}
