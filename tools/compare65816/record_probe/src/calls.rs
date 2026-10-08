//! Read-only typed call/value census and final selected instruction effects.
//! Reporting classifications grant no optimization or aliasing permission.
use actionc::mir65816::{
    analysis::*,
    emit::{Location, MachineProgram, proof},
    *,
};
use actionc::nir::RoutineId;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

pub(super) fn operation(op: &Mir65816Op) -> &'static str {
    match op {
        Mir65816Op::Load { .. } => "Load",
        Mir65816Op::Store { .. } => "Store",
        Mir65816Op::AddressOf { .. } => "AddressOf",
        Mir65816Op::Copy { .. } => "Copy",
        Mir65816Op::Unary { .. } => "Unary",
        Mir65816Op::Cast { .. } => "Cast",
        Mir65816Op::PointerOffset { .. } => "PointerOffset",
        Mir65816Op::Binary { .. } => "Binary",
        Mir65816Op::Compare { .. } => "Compare",
        Mir65816Op::Call { .. } => "Call",
    }
}

fn point_kind(r: &Mir65816Routine, point: ProgramPoint) -> &'static str {
    let block = r.blocks.iter().find(|b| b.id == point.block).unwrap();
    if let Some(op) = block.ops.get(point.index) {
        return operation(op);
    }
    match block.terminator {
        Mir65816Terminator::Return { .. } => "Return",
        Mir65816Terminator::Branch { .. } => "Branch",
        Mir65816Terminator::Goto(_) => "Goto",
        Mir65816Terminator::Fallthrough => "Fallthrough",
        Mir65816Terminator::Exit => "Exit",
        Mir65816Terminator::ArithmeticFault => "ArithmeticFault",
    }
}

pub(super) fn address(r: &Mir65816Routine, a: &Mir65816Address) -> Value {
    let (kind, id, private) = match &a.base {
        Mir65816AddressBase::Parameter(id) => {
            let p = r.frame.parameters.iter().find(|p| p.param == *id).unwrap();
            (
                if p.frame_object.is_none() {
                    "immutable_parameter"
                } else {
                    "mutable_parameter"
                },
                Some(id.0),
                p.frame_object.is_none(),
            )
        }
        Mir65816AddressBase::AutomaticFrame(id) => {
            let o = r.frame.objects.iter().find(|o| o.id == *id).unwrap();
            (
                if o.addressable {
                    "addressable_frame"
                } else {
                    "private_frame"
                },
                Some(id.0),
                !o.addressable,
            )
        }
        Mir65816AddressBase::Static(_) => ("static", None, false),
        Mir65816AddressBase::External(_) => ("external", None, false),
        Mir65816AddressBase::Indirect(_) => ("indirect", None, false),
    };
    json!({"kind":kind,"id":id,"private_source":private,
        "displacement":a.displacement.get(),"indexed":a.index.is_some()})
}

pub(super) fn operand(v: &Mir65816Value) -> Value {
    match v {
        Mir65816Value::Temp(id, w) => json!({"kind":"temp","id":id.0,"width":w.get()}),
        Mir65816Value::Param(id) => json!({"kind":"parameter","id":id.0}),
        Mir65816Value::U8(n) => json!({"kind":"constant","value":n,"width":1}),
        Mir65816Value::U16(n) => json!({"kind":"constant","value":n,"width":2}),
        Mir65816Value::U24(n) => json!({"kind":"constant","value":n,"width":3}),
        Mir65816Value::U32(n) => json!({"kind":"constant","value":n,"width":4}),
        Mir65816Value::Null(w) => json!({"kind":"constant","value":0,"width":w.get()}),
        Mir65816Value::Address(_, w) => json!({"kind":"absolute_address","width":w.get()}),
        Mir65816Value::StaticAddress(id, w) => {
            json!({"kind":"static_address","id":id.0,"width":w.get()})
        }
        Mir65816Value::GlobalAddress(id, w) => {
            json!({"kind":"global_address","id":id.0,"width":w.get()})
        }
        Mir65816Value::RoutineAddress(id, w) => {
            json!({"kind":"routine_address","id":id,"width":w.get()})
        }
    }
}

fn memory(m: proof::EffectMemory) -> Value {
    match m {
        proof::EffectMemory::Stack {
            displacement,
            bytes,
        } => json!({"kind":"stack","offset":displacement,"bytes":bytes}),
        proof::EffectMemory::DirectPage { offset, bytes } => {
            json!({"kind":"dp","offset":offset,"bytes":bytes})
        }
        proof::EffectMemory::Long { bytes, .. } => json!({"kind":"external","bytes":bytes}),
        proof::EffectMemory::Symbol { bytes, .. } => json!({"kind":"symbol","bytes":bytes}),
        proof::EffectMemory::SymbolIndexedX { bytes, .. } => {
            json!({"kind":"symbol_indexed","bytes":bytes})
        }
        proof::EffectMemory::IndirectLong { bytes, .. } => json!({"kind":"indirect","bytes":bytes}),
        proof::EffectMemory::Unknown => json!({"kind":"unknown","bytes":null}),
    }
}

pub(super) fn report(machine: &MachineProgram, out: &Path) {
    let names: BTreeMap<_, _> = machine
        .prepared
        .routines
        .iter()
        .map(|r| (r.id, &r.name))
        .collect();
    let mut rows = vec![];
    for m in &machine.routines {
        let r = machine
            .prepared
            .routines
            .iter()
            .find(|r| r.id == m.id)
            .unwrap();
        let a = if r.helper.is_none() && !r.entry.external {
            Some(RoutineAnalysis::new(r).unwrap())
        } else {
            None
        };
        let mut crossings = BTreeMap::<_, Vec<_>>::new();
        let mut calls = vec![];
        for b in &r.blocks {
            for (index, op) in b.ops.iter().enumerate() {
                let Mir65816Op::Call {
                    target,
                    args,
                    result,
                    plan,
                    ..
                } = op
                else {
                    continue;
                };
                let (kind, target_id, name) = match target {
                    Mir65816CallTarget::Direct(id) => {
                        ("direct", Some(*id), names.get(&RoutineId(*id)).copied())
                    }
                    Mir65816CallTarget::Helper(id) => {
                        ("helper", Some(id.0), names.get(id).copied())
                    }
                    Mir65816CallTarget::Runtime(id) => ("runtime", Some(id.0), None),
                    Mir65816CallTarget::Indirect(..) => ("indirect", None, None),
                    Mir65816CallTarget::Builtin(_) => panic!("unresolved builtin in prepared MIR"),
                };
                let mut reachable = false;
                let mut cyclic = false;
                let mut live = BTreeSet::new();
                if let Some(a) = &a {
                    let block = a.block(b.id).unwrap();
                    if a.dominates(block, block) == Ok(true) {
                        reachable = true;
                        cyclic = a.is_cyclic(block).unwrap();
                        let before: BTreeSet<_> = a
                            .live_at(a.point(block, index).unwrap())
                            .unwrap()
                            .into_iter()
                            .map(|t| t.id())
                            .collect();
                        let after: BTreeSet<_> = a
                            .live_at(a.point(block, index + 1).unwrap())
                            .unwrap()
                            .into_iter()
                            .map(|t| t.id())
                            .collect();
                        live = before.intersection(&after).copied().collect();
                        for id in &live {
                            crossings
                                .entry(*id)
                                .or_default()
                                .push([b.id.0, index as u32]);
                        }
                    }
                }
                let arguments: Vec<_> = args.iter().zip(&plan.arguments).map(|(v, h)| {
                    let Mir65816AbiHome::StackArgument { offset, size, alignment } = h else { panic!("non-stack native argument") };
                    json!({"value":operand(v),"offset":offset.get(),"width":size.get(),"alignment":alignment.get()})
                }).collect();
                assert_eq!(arguments.len(), args.len());
                let range = m.code.mir_spans.get(&(b.id, index));
                calls.push(json!({"block":b.id.0,"index":index,"target_kind":kind,"target_id":target_id,"target_name":name,
                    "arguments":arguments,"outgoing_bytes":plan.outgoing_bytes.get(),
                    "result":result.map(|(id,w)|json!({"id":id.0,"width":w.get()})),
                    "declared_result_bytes":match plan.result {
                        None => 0,
                        Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A8ZeroExtended)) => 1,
                        Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16)) => 2,
                        Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16X8ZeroExtended)) => 3,
                        Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16X16)) => 4,
                        _ => panic!("unsupported result home"),
                    },"reachable":reachable,"cyclic":cyclic,
                    "live_across":live.iter().map(|id|id.0).collect::<Vec<_>>(),
                    "span":range.map(|r|[r.start,r.end])}));
            }
        }
        let mut temps = vec![];
        if let Some(a) = &a {
            for (id, ty) in &r.temps {
                let f = a.value(a.temp(*id).unwrap()).unwrap();
                let producer = match f.definition {
                    Definition::BlockParameter { block, ordinal } => {
                        json!({"kind":"BlockParameter","block":block.0,"ordinal":ordinal})
                    }
                    Definition::Operation(p) => {
                        let block = r.blocks.iter().find(|b| b.id == p.block).unwrap();
                        let op = &block.ops[p.index];
                        let mut value =
                            json!({"kind":operation(op),"block":p.block.0,"index":p.index});
                        if let Mir65816Op::Load {
                            address: addr,
                            volatile,
                            ..
                        } = op
                        {
                            value["address"] = address(r, addr);
                            value["volatile"] = json!(volatile);
                        }
                        value
                    }
                };
                let home = m.frame.temps.get(id).map(|home|json!({"kind":match home { Location::Stack(_) => "stack",Location::DirectPage(_) => "dp"},"offset":home.slot().offset,"width":home.slot().width}));
                let uses: Vec<_> = f.uses.iter().map(|u|json!({"block":u.point.block.0,"index":u.point.index,"ordinal":u.ordinal,"kind":point_kind(r,u.point)})).collect();
                temps.push(json!({"id":id.0,"width":f.width.get(),"pointer":ty.pointer,"producer":producer,"uses":uses,
                    "home":home,"crossings":crossings.get(id).cloned().unwrap_or_default()}));
            }
        }
        // Normal builds do not retain the trace-only effect vector. Replay the
        // sealed final selection with observations enabled, checking complete
        // code/metadata equality before using its instruction effects.
        let (unobserved, _) = proof::replay_code(&m.code, false).unwrap();
        proof::compare_replay_output(&m.code, &unobserved).unwrap();
        let (replayed, _) = proof::replay_code(&m.code, true).unwrap();
        // Trace observations differ by design; compare every public encoding
        // and position field while replay checks the sealed actions/boundaries.
        assert_eq!(
            (
                &unobserved.bytes,
                &unobserved.fixups,
                &unobserved.labels,
                &unobserved.return_fixups,
                &unobserved.mir_spans,
                &unobserved.mir_transfers,
                &unobserved.conditional_branches,
                &unobserved.local_jumps
            ),
            (
                &replayed.bytes,
                &replayed.fixups,
                &replayed.labels,
                &replayed.return_fixups,
                &replayed.mir_spans,
                &replayed.mir_transfers,
                &replayed.conditional_branches,
                &replayed.local_jumps
            )
        );
        let selected = proof::selected_actions(&replayed).unwrap();
        let observations: BTreeMap<_, _> = selected
            .iter()
            .filter(|s| s.kind == "instruction")
            .map(|s| ((s.encoded.start, s.encoded.end), s))
            .collect();
        let instructions: Vec<_> = proof::instruction_effects(&replayed)
            .iter()
            .map(|e| {
                let s = observations
                    .get(&(e.start, e.end))
                    .expect("effect has no final selected instruction");
                let mem: Vec<_> = e
                    .effects
                    .memory
                    .iter()
                    .map(|m| {
                        let mut value = memory(m.memory);
                        value["access"] = json!(match m.access {
                            proof::EffectAccess::Read => "read",
                            proof::EffectAccess::Write => "write",
                            proof::EffectAccess::MayWrite => "may_write",
                        });
                        value
                    })
                    .collect();
                let control = match e.effects.control {
                    proof::EffectControl::Next => "next",
                    proof::EffectControl::Branch { .. } => "branch",
                    proof::EffectControl::Jump(_) => "jump",
                    proof::EffectControl::Call { .. } => "call",
                    proof::EffectControl::Forward(_) => "forward",
                    proof::EffectControl::Return => "return",
                };
                json!({"start":e.start,"end":e.end,"source":s.source.map(|(b,i)|[b.0 as usize,i]),
                "depth":s.depth_before,"control":control,"memory":mem})
            })
            .collect();
        let requests: Vec<_> = selected.iter().filter(|s|s.decision.is_some()).map(|s| {
            let request = s.request.or_else(|| s.parent.and_then(|parent| proof::selected_site(&replayed, parent).unwrap().request));
            json!({"request":request,"accepted":s.decision,"source":s.source.map(|(b,i)|[b.0 as usize,i])})
        }).collect();
        rows.push(json!({"id":r.id.0,"name":r.name,"helper":r.helper.map(|h|json!({"operation":format!("{:?}",h.operation),"width":h.bytes,"signed":h.signed})),
            "frame":m.frame.extent,"spill":m.frame.spill_bytes,"local_peak":m.frame.peak_below_entry,
            "objects":r.frame.objects.iter().map(|o|json!({"id":o.id.0,"offset":o.stack_offset.get(),"bytes":o.size.get(),"addressable":o.addressable})).collect::<Vec<_>>(),
            "calls":calls,"temps":temps,"instructions":instructions,"requests":requests}));
    }
    fs::write(
        out.with_extension("calls.json"),
        serde_json::to_vec(&json!({"schema":1,"routines":rows})).unwrap(),
    )
    .unwrap();
}
