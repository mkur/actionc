//! Per-site uses of a pool byte by the ABI, instruction scratch and final layout.
use super::*;
use super::interference::Lifetime;
use crate::mir6502::analysis::{record_bases::MirPhysicalMemory, sites::MirSite};

#[derive(Default)]
pub(super) struct Hazards {
    occupied: BTreeMap<MirSite, BTreeSet<u8>>,
    across_call: BTreeMap<MirSite, BTreeSet<u8>>,
}

impl Hazards {
    pub fn new(r: &MirRoutine, calls: &mut ResidenceCalls<'_>, layout: Option<&MirPhysicalMemory>) -> Self {
        let mut result = Self::default();
        let fixed = calls.fixed_storage();
        for b in &r.blocks {
            for (i, op) in b.ops.iter().enumerate() {
                let site = MirSite::Op { block: b.id, op_index: i };
                if matches!(op, MirOp::Call { .. } | MirOp::RuntimeHelper { .. }) {
                    let writes = calls.home_writes_at(r, site, op);
                    result.across_call.insert(site, (0xe0..=0xef).filter(|&byte|
                        !writes.as_ref().is_ok_and(|w| w.preserves_private_fixed_byte(MirFixedZpSlot(byte), &fixed))).collect());
                    // Input sources must survive until consumed by argument setup.
                    if let MirOp::Call { args, .. } = op {
                        let lanes = args.iter().map(|arg| crate::mir6502::abi::physical_lanes(&arg.home, arg.width))
                            .collect::<Option<Vec<_>>>();
                        let setup = lanes.map(|lanes| lanes.into_iter().flatten().filter_map(|lane| match lane {
                            MirParamLane::ZeroPage(s) => Some(s.0), _ => None,
                        }).collect()).unwrap_or_else(|| (0xe0..=0xef).collect());
                        result.occupied.insert(site, setup);
                    }
                } else {
                    let e = classify_op(op);
                    let opaque = matches!(op, MirOp::MachineBlock { .. } | MirOp::SArgsCapture { .. } | MirOp::Barrier { .. })
                        || e.memory.opaque || (e.memory.indirect_writes && !writes_ordinary_pointee(op));
                    let mut bytes = if opaque { (0xe0..=0xef).collect() } else { BTreeSet::new() };
                    for home in e.addresses.pair_reads.iter().chain(&e.addresses.pair_writes)
                        .chain(&e.homes.reads).chain(&e.homes.writes) {
                        if let MirHomeByte::FixedZeroPage(s) = home { bytes.insert(s.0); }
                    }
                    // Logical residents are checked individually by Contexts.
                    for range in e.memory.direct_reads.iter().chain(&e.memory.direct_writes) {
                        if matches!(range.base, MirMem::ZeroPage(_) | MirMem::Spill { .. }) { continue; }
                        for delta in 0..range.bytes {
                            if r.frame.object_lane(&range.base, delta).is_some() { continue; }
                            if let Some(base) = layout.and_then(|m| m.address(r.id, &range.base)) {
                                if let Ok(byte) = u8::try_from(base.wrapping_add(delta)) { bytes.insert(byte); }
                            } else if let Some(slot) = super::super::logical::physical_slot(&range.base, delta) {
                                bytes.insert(slot.0);
                            } else if layout.is_some() {
                                bytes.extend(0xe0..=0xef);
                            }
                        }
                    }
                    result.occupied.insert(site, bytes);
                }
            }
        }
        result
    }

    pub fn allows(&self, range: &Lifetime, address: u8) -> bool {
        !range.across_calls.iter().any(|s| self.across_call.get(s).is_some_and(|b| b.contains(&address)))
            && !range.occupied.iter().any(|s| self.occupied.get(s).is_some_and(|b| b.contains(&address)))
    }
}
