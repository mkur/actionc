//! Typed operands and storage facts for read-only call-flow investigation.
use super::calls::{address, operand, operation};
use actionc::mir65816::{emit::MachineProgram, *};
use serde_json::{Value, json};
use std::{fs, path::Path};

fn address_facts(r: &Mir65816Routine, a: &Mir65816Address) -> Value {
    let mut value = address(r, a);
    if let Mir65816AddressBase::Indirect(base) = &a.base {
        value["base_value"] = operand(base);
    }
    value["index"] = json!(
        a.index
            .as_ref()
            .map(|i| json!({"value":operand(&i.value),"stride":i.stride.get()}))
    );
    value
}

fn facts(r: &Mir65816Routine, op: &Mir65816Op) -> Value {
    let mut value = json!({"kind":operation(op)});
    let details = match op {
        Mir65816Op::Load {
            dest,
            width,
            address,
            volatile,
        } => {
            json!({"dest":dest.0,"width":width.get(),"address":address_facts(r,address),"volatile":volatile})
        }
        Mir65816Op::Store {
            address,
            value,
            width,
            volatile,
        } => {
            json!({"address":address_facts(r,address),"value":operand(value),"width":width.get(),"volatile":volatile})
        }
        Mir65816Op::AddressOf {
            dest,
            address,
            width,
        } => json!({"dest":dest.0,"address":address_facts(r,address),"width":width.get()}),
        Mir65816Op::Copy {
            source,
            destination,
            bytes,
            ..
        } => {
            json!({"source":address_facts(r,source),"destination":address_facts(r,destination),"width":bytes.get()})
        }
        Mir65816Op::Unary {
            dest,
            width,
            operation,
            value,
        } => {
            json!({"dest":dest.0,"width":width.get(),"operation":format!("{operation:?}"),"value":operand(value)})
        }
        Mir65816Op::Cast {
            dest,
            from,
            to,
            from_signed,
            kind,
            value,
        } => {
            json!({"dest":dest.0,"from":from.get(),"width":to.get(),"from_signed":from_signed,"cast_kind":format!("{kind:?}"),"value":operand(value)})
        }
        Mir65816Op::PointerOffset {
            dest,
            width,
            base,
            offset,
            subtract,
            offset_signed,
        } => {
            json!({"dest":dest.0,"width":width.get(),"base":operand(base),"offset":operand(offset),"subtract":subtract,"offset_signed":offset_signed})
        }
        Mir65816Op::Binary {
            dest,
            width,
            signed,
            left,
            right,
            ..
        }
        | Mir65816Op::Compare {
            dest,
            width,
            signed,
            left,
            right,
            ..
        } => {
            let operator = match op {
                Mir65816Op::Binary { operation, .. } => format!("{operation:?}"),
                Mir65816Op::Compare { operation, .. } => format!("{operation:?}"),
                _ => unreachable!(),
            };
            json!({"dest":dest.0,"width":width.get(),"signed":signed,"operation":operator,"left":operand(left),"right":operand(right)})
        }
        Mir65816Op::Call {
            args,
            result,
            target,
            ..
        } => {
            json!({"args":args.iter().map(operand).collect::<Vec<_>>(),"result":result.map(|(id,w)|json!({"id":id.0,"width":w.get()})),
            "target_value":if let Mir65816CallTarget::Indirect(v,_) = target {Some(operand(v))} else {None}})
        }
    };
    value
        .as_object_mut()
        .unwrap()
        .extend(details.as_object().unwrap().clone());
    value
}

pub(super) fn report(machine: &MachineProgram, out: &Path) {
    let routines: Vec<_> = machine.routines.iter().map(|m| {
        let r = machine.prepared.routines.iter().find(|r|r.id==m.id).unwrap();
        let blocks: Vec<_> = r.blocks.iter().map(|b| json!({"id":b.id.0,
            "ops":b.ops.iter().map(|op|facts(r,op)).collect::<Vec<_>>()})).collect();
        json!({"id":r.id.0,"name":r.name,"blocks":blocks,
            "parameters":r.frame.parameters.iter().map(|p|json!({"id":p.param.0,"object":p.frame_object.map(|o|o.0),
                "width":if let Mir65816AbiHome::StackArgument {size,..} = p.incoming {size.get()} else {0}})).collect::<Vec<_>>(),
            "objects":r.frame.objects.iter().map(|o|json!({"id":o.id.0,"width":o.size.get(),"addressable":o.addressable,"mutable":o.mutable})).collect::<Vec<_>>(),
            "types":r.temps.iter().map(|(id,t)|json!({"id":id.0,"pointer":t.pointer,
                "integer":t.kind.integer().map(|i|json!({"signed":i.signed,"bits":i.bits,"address":matches!(i.role,actionc::nir::NirIntegerRole::Address)}))})).collect::<Vec<_>>()})
    }).collect();
    fs::write(
        out.with_extension("flow.json"),
        serde_json::to_vec(&json!({"schema":1,"routines":routines})).unwrap(),
    )
    .unwrap();
}
