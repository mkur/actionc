//! Allocate ordinary parameters and locals together when the lifetimes policy is active.
use super::*;
use crate::mir6502::analysis::{
    cfg::MirCfg,
    dominance::MirDominance,
    effects::{classify_op, classify_terminator},
};
use crate::mir6502::trace::Trace;
use std::collections::BTreeMap;

struct Candidate {
    object: logical::ObjectByte,
    name: Option<String>,
    width: u16,
    offset: u16,
    incoming: Option<u8>,
    forbidden: BTreeSet<u8>,
}

// A loop access gets the weight of four straight-line accesses. The final
// image comparison accounts for actual capture, spill and reload instructions.
fn weights(routine: &MirRoutine) -> BTreeMap<logical::ObjectByte, usize> {
    let mut scores = BTreeMap::new();
    let Ok(cfg) = MirCfg::from_routine(routine) else {
        return scores;
    };
    let dominance = MirDominance::from_cfg(&cfg);
    let mut loops = BTreeSet::new();
    for &header in cfg.block_ids() {
        let mut work: Vec<_> = cfg
            .predecessors(header)
            .iter()
            .copied()
            .filter(|tail| dominance.block_dominates(header, *tail))
            .collect();
        if work.is_empty() {
            continue;
        }
        let mut members = BTreeSet::from([header]);
        while let Some(block) = work.pop() {
            if members.insert(block) {
                work.extend(cfg.predecessors(block));
            }
        }
        loops.extend(members);
    }
    for block in &routine.blocks {
        let weight = if loops.contains(&block.id) { 4 } else { 1 };
        let effects = block
            .ops
            .iter()
            .map(|op| classify_op(op).memory)
            .chain(std::iter::once(
                classify_terminator(&block.terminator).memory,
            ));
        for effect in effects {
            for access in effect.direct_reads.iter().chain(&effect.direct_writes) {
                let key = match access.base {
                    MirMem::Param { id, .. } => logical::ObjectByte::Param(id, 0),
                    MirMem::Local { id, .. } => logical::ObjectByte::Local(id, 0),
                    _ => continue,
                };
                *scores.entry(key).or_default() += weight * usize::from(access.bytes);
            }
        }
    }
    scores
}

pub(in crate::mir6502::parameter_homes) fn allocate(
    program: &MirProgram,
    routine: &mut MirRoutine,
    parameters: bool,
    addresses: &ObjectAddressControl<'_>,
    forbidden: &impl Fn(&MirMem, u16) -> Option<BTreeSet<u8>>,
    trace: &Trace,
) -> bool {
    let Some(total) = routine.frame.abi_parameter_bytes() else {
        return false;
    };
    let mut used = used_bytes(program, routine, &BTreeSet::new());
    let mut params = Vec::new();
    let mut locals = Vec::new();
    let mut candidates = Vec::new();
    let mut changed = false;
    if parameters && routine.frame.parameter_homes.is_none() {
        if total > crate::action_abi::MAX_DIRECT_PARAM_CAPTURE_BYTES {
            if let Some(bindings) = sargs_bindings(routine, used, forbidden, addresses) {
                install_bindings(routine, bindings);
                for (_, slot) in routine.frame.resident_bytes() {
                    used[usize::from(slot.0)] = true;
                }
                changed = true;
            } else {
                trace.event("source-home-inline", || {
                    serde_json::json!({"routine":routine.name,
                    "frame":"SArgs","bytes":total,"reason":"no eligible contiguous capture frame"})
                });
            }
        } else {
            let mut offset = 0;
            for slot in &routine.frame.params {
                let MirStorageBase::Param(id) = slot.base else {
                    return false;
                };
                let mut home = MirParamHome::Inline;
                if eligible_parameter(addresses, routine, slot) {
                    if absent_parameter(routine, id) {
                        home = MirParamHome::Absent;
                    } else if let Some(conflicts) =
                        forbidden(&MirMem::Param { id, offset: 0 }, slot.storage_size)
                    {
                        candidates.push(Candidate {
                            object: logical::ObjectByte::Param(id, 0),
                            name: slot.name.clone(),
                            width: slot.storage_size,
                            offset,
                            incoming: incoming_lane(offset, slot.storage_size),
                            forbidden: conflicts,
                        });
                    }
                }
                params.push(MirParamBinding { param: id, home });
                offset += slot.storage_size;
            }
        }
    }
    if routine.frame.local_homes.is_none() && !routine.frame.locals.is_empty() {
        let entry = super::super::locals::entry_dependencies(routine);
        for slot in &routine.frame.locals {
            let MirStorageBase::Local(id) = slot.base else {
                continue;
            };
            locals.push(MirLocalBinding {
                local: id,
                home: MirLocalHome::Inline,
            });
            let reason = if !addresses
                .compiler_controls_address(routine.id, &MirMem::Local { id, offset: 0 })
            {
                Some("source owns the cell address")
            } else if slot.purpose == MirStoragePurpose::Own {
                Some("OWN requires persistent storage")
            } else if slot.init.is_some() {
                Some("initial contents require persistent storage")
            } else if !super::super::locals::eligible(addresses, routine, slot) {
                Some("nonordinary local storage")
            } else if entry.as_ref().is_none_or(|ids| ids.contains(&id)) {
                Some("value is live between invocations")
            } else {
                None
            };
            if let Some(reason) = reason {
                trace.event("source-home-inline", || {
                    serde_json::json!({
                        "routine":routine.name, "object":slot.name, "reason":reason
                    })
                });
                continue;
            }
            if let Some(conflicts) = forbidden(&MirMem::Local { id, offset: 0 }, slot.storage_size)
            {
                candidates.push(Candidate {
                    object: logical::ObjectByte::Local(id, 0),
                    name: slot.name.clone(),
                    width: slot.storage_size,
                    offset: total,
                    incoming: None,
                    forbidden: conflicts,
                });
            }
        }
    }

    let place = |candidates: &[Candidate]| {
        let mut occupied = used;
        let mut starts = Vec::with_capacity(candidates.len());
        let mut pressure = false;
        for c in candidates {
            let fits = |mask| {
                pick(
                    routine,
                    &excluding(mask, &c.forbidden),
                    c.width,
                    c.incoming,
                    total,
                    c.offset,
                )
            };
            let start = fits(occupied);
            if let Some(start) = start {
                occupied[usize::from(start)..usize::from(start) + usize::from(c.width)].fill(true);
            } else if fits(used).is_some() {
                pressure = true;
            }
            starts.push(start);
        }
        (starts, pressure)
    };
    let (mut starts, pressure) = place(&candidates);
    if pressure {
        let scores = weights(routine);
        candidates.sort_by_key(|c| {
            let capture = usize::from(
                matches!(c.object, logical::ObjectByte::Param(..)) && c.incoming.is_none(),
            );
            let score = scores
                .get(&c.object)
                .copied()
                .unwrap_or(0)
                .saturating_sub(capture);
            (std::cmp::Reverse(score), c.object)
        });
        starts = place(&candidates).0;
        // Share a byte only when the ordinary allocation has run out. ABI
        // capture lanes remain distinct, even for unused captured parameters.
        let logical = std::cell::LazyCell::new(|| logical::LogicalHomes::new(routine));
        for (index, c) in candidates.iter().enumerate() {
            let logical::ObjectByte::Local(id, _) = c.object else {
                continue;
            };
            if starts[index].is_some() {
                continue;
            }
            let Some(logical) = &*logical else { break };
            let mut occupied = used;
            for (other, &start) in candidates.iter().zip(&starts) {
                let Some(start) = start else { continue };
                if matches!(other.object, logical::ObjectByte::Local(other, _) if !logical.locals_interfere(id, other))
                {
                    continue;
                }
                occupied[usize::from(start)..usize::from(start) + usize::from(other.width)]
                    .fill(true);
            }
            starts[index] = pick(
                routine,
                &excluding(occupied, &c.forbidden),
                c.width,
                c.incoming,
                total,
                c.offset,
            );
        }
    }
    let mut placed = BTreeMap::new();
    for (c, start) in candidates.iter().zip(&starts) {
        if let Some(start) = start {
            placed.insert(c.object, MirFixedZpSlot(*start));
        } else {
            trace.event("source-home-inline", || serde_json::json!({"routine": routine.name, "object": c.name,
                "width": c.width, "reason": "no suitable contiguous pool bytes", "conflicts": c.forbidden,
                "reserved": used.iter().enumerate().filter_map(|(n, used)| used.then_some(n)).collect::<Vec<_>>(),
                "competing_homes": candidates.iter().zip(&starts).filter_map(|(other, &start)|
                    start.map(|s| (&other.name, s, other.width))).collect::<Vec<_>>() }));
        }
    }
    for b in &mut params {
        if let Some(slot) = placed.get(&logical::ObjectByte::Param(b.param, 0)) {
            b.home = MirParamHome::ZeroPage(*slot);
        }
    }
    if params
        .iter()
        .any(|b| matches!(b.home, MirParamHome::ZeroPage(_)))
    {
        install_bindings(routine, params);
        changed = true;
    }
    for b in &mut locals {
        if let Some(slot) = placed.get(&logical::ObjectByte::Local(b.local, 0)) {
            b.home = MirLocalHome::ZeroPage(*slot);
        }
    }
    if locals
        .iter()
        .any(|b| matches!(b.home, MirLocalHome::ZeroPage(_)))
    {
        routine.frame.local_homes = Some(locals);
        for (_, slot) in routine.frame.resident_bytes() {
            routine.frame.reserve_named_lane(slot);
        }
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_one_slot(declarations: &str, body: &str) -> (MirRoutine, Vec<String>) {
        let reserves = (0xa0..=0xa3)
            .chain(0xe0..=0xee)
            .map(|byte| format!("r{byte}=${byte:02X}"))
            .collect::<Vec<_>>()
            .join(",");
        let source = format!(
            "BYTE {reserves} BYTE input=$700,out=$701
            PROC Work() BYTE {declarations} {body} RETURN PROC Main() Work() RETURN"
        );
        let ast = crate::parser::parse(&crate::lexer::tokenize(&source).unwrap()).unwrap();
        let model = crate::semantic::analyze(&ast).unwrap();
        let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
        let p = crate::mir6502::lower_program(&nir).unwrap();
        let mut routine = p.routines[0].clone();
        let trace = crate::mir6502::trace::Mir6502Trace::default();
        assert!(allocate(
            &p,
            &mut routine,
            true,
            &ObjectAddressControl::new(&p),
            &|_, _| Some(BTreeSet::new()),
            &trace.context()
        ));
        let reasons = trace
            .events()
            .iter()
            .filter(|e| e.kind == "source-home-inline")
            .map(|e| e.data["object"].as_str().unwrap().to_owned())
            .collect();
        (routine, reasons)
    }

    #[test]
    fn competing_loop_local_wins_in_either_declaration_order() {
        for declarations in ["cold,hot", "hot,cold"] {
            let (r, inline) = with_one_slot(
                declarations,
                "cold=input hot=input WHILE hot DO out=hot hot==-1 OD out=cold",
            );
            assert_eq!(inline, ["cold"]);
            assert_eq!(r.frame.resident_bytes().len(), 1);
            assert_eq!(r.frame.resident_bytes()[0].1, MirFixedZpSlot(0xef));
        }
    }

    #[test]
    fn disjoint_local_lifetimes_share_the_last_byte_and_verify() {
        let (r, inline) = with_one_slot(
            "first,second",
            "first=input out=first second=input out=second",
        );
        assert!(inline.is_empty());
        assert_eq!(r.frame.resident_bytes().len(), 2);
        assert!(
            r.frame
                .resident_bytes()
                .iter()
                .all(|(_, slot)| slot.0 == 0xef)
        );
        assert!(super::super::super::locals::verify_bindings(&r).is_empty());
    }
}
