//! Facts from typed MIR, deliberately independent of production allocation.
use super::*;
use actionc::nir::{NirBinaryOp, NirCompareOp, NirIntegerRole, NirTypeKind};

fn ty(t: &actionc::nir::NirType) -> Value {
    let kind = match &t.kind {
        NirTypeKind::Void => "void",
        NirTypeKind::Bool => "bool",
        NirTypeKind::Integer(i) => {
            return json!({"kind":"integer","bits":i.bits,"signed":i.signed,
            "ordinary":i.role==NirIntegerRole::Ordinary,"width":t.width.map(|w|w.get()),"pointer":t.pointer});
        }
        NirTypeKind::Pointer { .. } => "pointer",
        NirTypeKind::Callable { .. } => "callable",
        NirTypeKind::Record { .. } => "record",
        NirTypeKind::Real => "real",
        NirTypeKind::Error => "error",
    };
    json!({"kind":kind,"width":t.width.map(|w|w.get()),"pointer":t.pointer})
}

pub fn enrich(f: &mut Value, r: &Mir65816Routine, m: &MachineRoutine) {
    f["fixed_object_extent"] = json!(r.frame.extent.get());
    f["spill_bytes"] = json!(m.frame.spill_bytes);
    f["peak"] = json!(m.frame.peak_below_entry);
    f["result"] = json!(match r.result_home {
        None => "void",
        Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16)) => "word",
        _ => "other",
    });
    f["parameters"] = json!(r.frame.parameters.iter().map(|p| {
        let Mir65816AbiHome::StackArgument {offset,size,alignment}=p.incoming else {panic!("unverified parameter")};
        json!({"id":p.param.0,"offset":offset.get(),"width":size.get(),"alignment":alignment.get(),"object":p.frame_object.map(|id|id.0)})
    }).collect::<Vec<_>>());
    f["fixed_objects"] = json!(
        r.frame
            .objects
            .iter()
            .map(|o| json!({"id":o.id.0,"offset":o.stack_offset.get(),
        "width":o.size.get(),"addressable":o.addressable,"mutable":o.mutable}))
            .collect::<Vec<_>>()
    );
    for t in f["temp_homes"].as_array_mut().unwrap() {
        t["type"] = ty(&r
            .temps
            .iter()
            .find(|(id, _)| json!(id.0) == t["id"])
            .unwrap()
            .1);
    }
    for (b, block) in r.blocks.iter().zip(f["blocks"].as_array_mut().unwrap()) {
        block["parameter_widths"] =
            json!(b.params.iter().map(|(_, w)| w.get()).collect::<Vec<_>>());
        block["terminator_kind"] = json!(match b.terminator {
            Mir65816Terminator::Goto(_) => "goto",
            Mir65816Terminator::Branch { .. } => "branch",
            Mir65816Terminator::Return { .. } => "return",
            Mir65816Terminator::Fallthrough => "fallthrough",
            Mir65816Terminator::Exit => "exit",
        });
        for (op, row) in b.ops.iter().zip(block["ops"].as_array_mut().unwrap()) {
            let mut values = vec![];
            let native = match op {
                Mir65816Op::Load { width, .. } => width.get() == 2,
                Mir65816Op::Store { width, value, .. } => {
                    values.push(value);
                    width.get() == 2
                }
                Mir65816Op::Binary {
                    width,
                    operation,
                    left,
                    right,
                    signed,
                    ..
                } => {
                    values.extend([left, right]);
                    row["signed"] = json!(signed);
                    row["binary"] = json!(match operation {
                        NirBinaryOp::Add => "add",
                        NirBinaryOp::Sub => "sub",
                        _ => "other",
                    });
                    width.get() == 2 && matches!(operation, NirBinaryOp::Add | NirBinaryOp::Sub)
                }
                Mir65816Op::Compare {
                    width,
                    operation,
                    left,
                    right,
                    signed,
                    ..
                } => {
                    values.extend([left, right]);
                    row["signed"] = json!(signed);
                    row["predicate"] = json!(match operation {
                        NirCompareOp::Eq => "eq",
                        NirCompareOp::Ne => "ne",
                        NirCompareOp::Lt => "lt",
                        NirCompareOp::Le => "le",
                        NirCompareOp::Gt => "gt",
                        NirCompareOp::Ge => "ge",
                    });
                    width.get() == 2
                        && (!signed || matches!(operation, NirCompareOp::Eq | NirCompareOp::Ne))
                }
                Mir65816Op::Cast {
                    from,
                    to,
                    value,
                    from_signed,
                    ..
                } => {
                    values.push(value);
                    row["from_width"] = json!(from.get());
                    row["to_width"] = json!(to.get());
                    row["from_signed"] = json!(from_signed);
                    false
                }
                _ => false,
            };
            if let Mir65816Op::Load { address: a, .. } | Mir65816Op::Store { address: a, .. } = op {
                row["address"]["displacement"] = json!(a.displacement.get());
                row["address"]["id"] = match a.base {
                    Mir65816AddressBase::AutomaticFrame(id) => json!(id.0),
                    Mir65816AddressBase::Parameter(id) => json!(id.0),
                    _ => Value::Null,
                };
            }
            row["values"] = json!(
                values
                    .iter()
                    .map(|v| {
                        let mut f = source(v, r, m);
                        if matches!(v, Mir65816Value::U8(_)) {
                            f["word_operand"] = json!(true);
                        }
                        f
                    })
                    .collect::<Vec<_>>()
            );
            let memory_safe=match op {
                Mir65816Op::Load{address:a,volatile,..}|Mir65816Op::Store{address:a,volatile,..} => !volatile && a.index.is_none() && match a.base {
                    Mir65816AddressBase::AutomaticFrame(id)=>r.frame.objects.iter().any(|o|o.id==id&&!o.addressable&&a.displacement.get()+2<=o.size.get()),
                    Mir65816AddressBase::Parameter(id)=>r.frame.parameters.iter().any(|p|p.param==id&&matches!(p.incoming,Mir65816AbiHome::StackArgument{size,..} if a.displacement.get()+2<=size.get())),_=>false,
                },_=>true,
            };
            row["native_word_form"] = json!(native);
            row["safe_direct_memory"] = json!(memory_safe);
            // Only the stated whitelist has a closed scratch contract. Unknown
            // operations are barriers rather than an empty/clobber-free set.
            row["effects"] = if native && memory_safe {
                json!({"scratch_reads":[],"scratch_writes":[],"registers":["a"],"flags":["n","z","c","v"],
                    "width":"selector_tracked_m_x16","calls":false,"barrier":false})
            } else {
                json!({"barrier":true,"scratch_reads":null,"scratch_writes":null,"calls":matches!(op,Mir65816Op::Call{..})})
            };
        }
    }
    f["boundary_effects"] = json!({"entry":{"scratch_reads":[[68,72]],"scratch_writes":[],"registers":["a","x"],"flags":["n","z","c","v"],"fault":"nonreturning"},
        "exit":{"scratch_reads":[],"scratch_writes":[],"registers":["a","y"],"flags":["n","z","c","v"]},"alignment":256,"domain_fixed":true});
}
