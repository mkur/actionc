//! Read-only typed movement facts for the selective-staging baseline.
use actionc::{
    compiler::native65816,
    mir65816::{self, emit::*, image::Image, *},
    target::ByteSize,
};
use serde_json::{Value, json};
use std::path::PathBuf;

struct Sources(PathBuf);
impl Drop for Sources {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn home(location: Location) -> Value {
    let (kind, slot) = match location {
        Location::Stack(s) => ("stack", s),
        Location::DirectPage(s) => ("dp", s),
    };
    json!({"kind":kind,"offset":slot.offset,"width":slot.width})
}

fn source(v: &Mir65816Value, r: &Mir65816Routine, m: &MachineRoutine) -> Value {
    let immediate = |value: u32, width: u32, word: bool| json!({"kind":"immediate","value":value,"width":width,"word_operand":word});
    match v {
        Mir65816Value::Temp(id, w) => {
            let mut h = home(m.frame.temps[id]);
            assert_eq!(h["width"], w.get());
            h["temp"] = json!(id.0);
            h["word_operand"] = json!(h["kind"] == "stack" && w.get() == 2);
            h
        }
        Mir65816Value::Param(id) => {
            let p = r.frame.parameters.iter().find(|p| p.param == *id).unwrap();
            let Mir65816AbiHome::StackArgument { offset, size, .. } = p.incoming else {
                panic!("parameter without stack home")
            };
            let at = if let Some(id) = p.frame_object {
                r.frame
                    .objects
                    .iter()
                    .find(|o| o.id == id)
                    .unwrap()
                    .stack_offset
                    .get()
            } else {
                abi::stack::incoming_displacement(
                    ByteSize::new(m.frame.extent.into()),
                    offset,
                    size,
                )
                .unwrap()
                .get()
            };
            json!({"kind":"stack","offset":at,"width":size.get(),"param":id.0,
                "mutable_home":p.frame_object.is_some(),"word_operand":size.get()==2})
        }
        Mir65816Value::U8(v) => immediate((*v).into(), 1, false),
        Mir65816Value::U16(v) => immediate((*v).into(), 2, true),
        Mir65816Value::U24(v) => immediate(*v, 3, false),
        Mir65816Value::U32(v) => immediate(*v, 4, false),
        Mir65816Value::Null(w) => immediate(0, w.get(), false),
        Mir65816Value::Address(v, w) => immediate(v.value.try_into().unwrap(), w.get(), false),
        Mir65816Value::StaticAddress(_, w)
        | Mir65816Value::GlobalAddress(_, w)
        | Mir65816Value::RoutineAddress(_, w) => {
            json!({"kind":"reference","width":w.get(),"debug":format!("{v:?}"),"word_operand":false})
        }
    }
}

fn value_use(v: &Mir65816Value, uses: &mut std::collections::BTreeSet<u32>) {
    if let Mir65816Value::Temp(id, _) = v {
        uses.insert(id.0);
    }
}
fn address_uses(a: &Mir65816Address, uses: &mut std::collections::BTreeSet<u32>) {
    if let Mir65816AddressBase::Indirect(v) = &a.base {
        value_use(v, uses);
    }
    if let Some(i) = &a.index {
        value_use(&i.value, uses);
    }
}
fn address(a: &Mir65816Address, r: &Mir65816Routine, m: &MachineRoutine) -> Value {
    let (kind, offset) = match &a.base {
        Mir65816AddressBase::AutomaticFrame(id) => (
            "frame",
            Some(
                r.frame
                    .objects
                    .iter()
                    .find(|o| o.id == *id)
                    .unwrap()
                    .stack_offset
                    .get(),
            ),
        ),
        Mir65816AddressBase::Parameter(id) => (
            "parameter",
            source(&Mir65816Value::Param(*id), r, m)["offset"]
                .as_u64()
                .map(|x| x as u32),
        ),
        Mir65816AddressBase::External(_) => ("external", None),
        Mir65816AddressBase::Static(_) => ("static", None),
        Mir65816AddressBase::Indirect(_) => ("indirect", None),
    };
    json!({"kind":kind,"indexed":a.index.is_some(),"offset":offset.map(|x|x+a.displacement.get())})
}
fn operation(op: &Mir65816Op, r: &Mir65816Routine, m: &MachineRoutine) -> Value {
    use actionc::nir::{NirBinaryOp, NirCompareOp};
    let mut uses = std::collections::BTreeSet::new();
    let mut def = None;
    let mut out = json!({"producer":null,"consumer":null});
    let mut input = |v| value_use(v, &mut uses);
    let kind = match op {
        Mir65816Op::Load {
            dest,
            width,
            address: a,
            volatile,
        } => {
            def = Some(dest.0);
            address_uses(a, &mut uses);
            out["address"] = address(a, r, m);
            out["width"] = json!(width.get());
            out["volatile"] = json!(volatile);
            if width.get() == 2
                && !volatile
                && a.index.is_none()
                && !matches!(a.base, Mir65816AddressBase::Indirect(_))
            {
                out["producer"] = json!(dest.0);
            }
            "load"
        }
        Mir65816Op::Store {
            address: a,
            value,
            width,
            volatile,
        } => {
            input(value);
            address_uses(a, &mut uses);
            out["address"] = address(a, r, m);
            out["width"] = json!(width.get());
            out["volatile"] = json!(volatile);
            if width.get() == 2
                && !volatile
                && a.index.is_none()
                && !matches!(a.base, Mir65816AddressBase::Indirect(_))
            {
                out["consumer"] = source(value, r, m);
            }
            "store"
        }
        Mir65816Op::AddressOf {
            dest, address: a, ..
        } => {
            def = Some(dest.0);
            address_uses(a, &mut uses);
            "address_of"
        }
        Mir65816Op::Copy {
            source,
            destination,
            ..
        } => {
            address_uses(source, &mut uses);
            address_uses(destination, &mut uses);
            "copy"
        }
        Mir65816Op::Unary { dest, value, .. } => {
            def = Some(dest.0);
            input(value);
            "unary"
        }
        Mir65816Op::Cast { dest, value, .. } => {
            def = Some(dest.0);
            input(value);
            "cast"
        }
        Mir65816Op::PointerOffset {
            dest, base, offset, ..
        } => {
            def = Some(dest.0);
            input(base);
            input(offset);
            "pointer_offset"
        }
        Mir65816Op::Binary {
            dest,
            left,
            right,
            width,
            operation,
            ..
        } => {
            def = Some(dest.0);
            input(left);
            input(right);
            out["width"] = json!(width.get());
            if width.get() == 2 && matches!(operation, NirBinaryOp::Add | NirBinaryOp::Sub) {
                out["producer"] = json!(dest.0);
                out["consumer"] = source(left, r, m);
            }
            "binary"
        }
        Mir65816Op::Compare {
            dest,
            left,
            right,
            width,
            operation,
            signed,
        } => {
            def = Some(dest.0);
            input(left);
            input(right);
            out["width"] = json!(width.get());
            if width.get() == 2
                && (!signed || matches!(operation, NirCompareOp::Eq | NirCompareOp::Ne))
            {
                out["consumer"] = source(
                    if matches!(operation, NirCompareOp::Gt | NirCompareOp::Le) {
                        right
                    } else {
                        left
                    },
                    r,
                    m,
                );
            }
            "compare"
        }
        Mir65816Op::Call {
            target,
            args,
            result,
            ..
        } => {
            if let Mir65816CallTarget::Indirect(v, _) = target {
                input(v);
            }
            for v in args {
                input(v);
            }
            def = result.map(|v| v.0.0);
            "call"
        }
    };
    out["kind"] = json!(kind);
    out["uses"] = json!(uses);
    out["definition"] = json!(def);
    out
}

#[test]
#[ignore = "requires A816_COMPARISON_MANIFEST and A816_MOVEMENT_FACTS"]
fn export_verified_movement_inventory() {
    let sources = Sources(
        std::env::temp_dir().join(format!("actionc-movement-inventory-{}", std::process::id())),
    );
    std::fs::create_dir(&sources.0).unwrap();
    let path = PathBuf::from(std::env::var_os("A816_COMPARISON_MANIFEST").unwrap());
    let out = PathBuf::from(std::env::var_os("A816_MOVEMENT_FACTS").unwrap());
    if out.exists() {
        std::fs::remove_file(&out).unwrap();
    }
    let manifest: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut builds = vec![];
    for a in manifest["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["compiler"] == "actionc")
    {
        let command = a["commands"][0].as_array().unwrap();
        let layout = command.iter().position(|v| v == "--layout").unwrap();
        let options =
            serde_json::from_slice(&std::fs::read(command[layout + 1].as_str().unwrap()).unwrap())
                .unwrap();
        let p = native65816::prepare_file(
            command.last().unwrap().as_str().unwrap(),
            a["mode"] == "optimized",
            &Default::default(),
        )
        .unwrap();
        mir65816::verify_program(&p.mir).unwrap();
        let c = p.compile(&options).unwrap();
        let saved =
            Image::from_json(&std::fs::read(a["image"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(
            c.image.to_json().unwrap(),
            saved.to_json().unwrap(),
            "inventory must describe the measured image"
        );
        let text = std::fs::read_to_string(command.last().unwrap().as_str().unwrap())
            .unwrap()
            .replace("\r\n", "\n");
        for text in [text.clone(), text.replace('\n', "\r\n")] {
            let path = sources.0.join("corpus.act");
            std::fs::write(&path, text).unwrap();
            let normalized =
                native65816::prepare_file(&path, a["mode"] == "optimized", &Default::default())
                    .unwrap()
                    .compile(&options)
                    .unwrap();
            assert_eq!(
                normalized.image.to_json().unwrap(),
                saved.to_json().unwrap(),
                "LF/CRLF parsing must match the saved image"
            );
        }
        let mut routines = vec![];
        for m in &c.machine.routines {
            let placed = saved.routines.iter().find(|r| r.id == m.id.0).unwrap();
            if !a["code_ranges"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v[0] == placed.address)
            {
                continue;
            }
            let r = p.mir.routines.iter().find(|r| r.id == m.id).unwrap();
            let mut transfers = m.code.mir_transfers.iter();
            let mut edges = vec![];
            for (bi, b) in r.blocks.iter().enumerate() {
                let expected = match &b.terminator {
                    Mir65816Terminator::Goto(e) => vec![("goto", e.clone())],
                    Mir65816Terminator::Branch {
                        then_edge,
                        else_edge,
                        ..
                    } => vec![("false", else_edge.clone()), ("true", then_edge.clone())],
                    Mir65816Terminator::Fallthrough => vec![(
                        "fallthrough",
                        Mir65816Edge {
                            target: r.blocks[bi + 1].id,
                            args: vec![],
                        },
                    )],
                    _ => vec![],
                };
                for (arm, e) in expected {
                    let ti = r.blocks.iter().position(|b| b.id == e.target).unwrap();
                    let transfer = transfers.next().unwrap();
                    assert_eq!(
                        (transfer.source, transfer.target),
                        (Label(bi as u32), Label(ti as u32))
                    );
                    let target = m.code.labels[&transfer.target];
                    if transfer.fallthrough {
                        assert_eq!(transfer.offset, target);
                    } else {
                        assert_eq!(m.code.bytes[transfer.offset], 0x5c);
                        assert!(m.code.fixups.iter().any(|f| f.offset == transfer.offset + 1
                            && f.target == Target::Label(transfer.target)
                            && f.addend == 0
                            && f.byte.is_none()));
                    }
                    let params = &r.blocks[ti].params;
                    assert_eq!(e.args.len(), params.len());
                    let moves: Vec<_> = e
                        .args
                        .iter()
                        .zip(params)
                        .map(|(arg, (dest, w))| {
                            let src = source(arg, r, m);
                            let dst = home(m.frame.temps[dest]);
                            assert_eq!(src["width"], w.get());
                            assert_eq!(dst["width"], w.get());
                            let attempts = if let Mir65816Value::Temp(id, _) = arg {
                                [(*id, *dest), (*dest, *id)]
                                    .into_iter()
                                    .map(|(changed, onto)| {
                                        let mut frame = m.frame.clone();
                                        frame.temps.insert(changed, frame.temps[&onto]);
                                        json!({"changed":changed.0,"onto":onto.0,
                                    "verifier_error":frame.verify_stack(r).err()})
                                    })
                                    .collect::<Vec<_>>()
                            } else {
                                vec![]
                            };
                            json!({"source":src,"destination":dst,"destination_temp":dest.0,
                            "width":w.get(),"coalescing_attempts":attempts})
                        })
                        .collect();
                    // Bounded diagnostic probes only; never feed an altered frame
                    // to emission. Combinations reveal third-party home conflicts.
                    let pairs: Vec<_> = e
                        .args
                        .iter()
                        .zip(params)
                        .filter_map(|(v, (d, w))| {
                            if let Mir65816Value::Temp(a, sw) = v {
                                (*sw == *w
                                    && matches!(m.frame.temps[a], Location::Stack(_))
                                    && matches!(m.frame.temps[d], Location::Stack(_)))
                                .then_some((*a, *d))
                            } else {
                                None
                            }
                        })
                        .collect();
                    let mut probes = vec![];
                    if pairs.len() <= 8 {
                        for choices in 1..3usize.pow(pairs.len() as u32) {
                            let mut choices = choices;
                            let mut changes = std::collections::BTreeMap::new();
                            let mut consistent = true;
                            for &(a, d) in &pairs {
                                let direction = choices % 3;
                                choices /= 3;
                                let (changed, onto) = match direction {
                                    1 => (a, d),
                                    2 => (d, a),
                                    _ => continue,
                                };
                                let slot = m.frame.temps[&onto];
                                if let Some(old) = changes.insert(changed, slot) {
                                    consistent &= old == slot;
                                }
                            }
                            if !consistent {
                                continue;
                            }
                            let mut frame = m.frame.clone();
                            frame
                                .temps
                                .extend(changes.iter().map(|(&id, &slot)| (id, slot)));
                            probes.push(json!({"changes":changes.iter().map(|(id,slot)|json!({"temp":id.0,"home":home(*slot)})).collect::<Vec<_>>(),
                                "verifier_error":frame.verify_stack(r).err()}));
                        }
                    }
                    edges.push(json!({"block":b.id.0,"arm":arm,"target_block":e.target.0,
                        "block_pc":placed.address+m.code.labels[&Label(bi as u32)] as u32,
                        "transfer_pc":placed.address+transfer.offset as u32,"target_pc":placed.address+target as u32,
                        "fallthrough":transfer.fallthrough,"moves":moves,"layout_probe_complete":pairs.len()<=8,"layout_probes":probes}));
                }
            }
            assert!(transfers.next().is_none());
            let blocks: Vec<_> = r.blocks.iter().enumerate().map(|(bi,b)| {
                let mut term_uses = std::collections::BTreeSet::new();
                let (kind, successors, consumer) = match &b.terminator {
                    Mir65816Terminator::Goto(e) => {
                        for v in &e.args { value_use(v,&mut term_uses); }
                        ("goto",vec![e.target.0],None)
                    }
                    Mir65816Terminator::Branch { condition, then_edge, else_edge } => {
                        value_use(condition,&mut term_uses);
                        for v in then_edge.args.iter().chain(&else_edge.args) { value_use(v,&mut term_uses); }
                        ("branch",vec![else_edge.target.0,then_edge.target.0],None)
                    }
                    Mir65816Terminator::Return { value,.. } => {
                        if let Some(v) = value { value_use(v,&mut term_uses); }
                        ("return",vec![],value.as_ref().map(|v| source(v,r,m)))
                    }
                    Mir65816Terminator::Fallthrough => ("fallthrough",vec![r.blocks[bi+1].id.0],None),
                    Mir65816Terminator::Exit => ("exit",vec![],None),
                };
                let mut ops: Vec<_> = b.ops.iter().enumerate().map(|(i,op)| {
                    let mut fact = operation(op,r,m);
                    let span = &m.code.mir_spans[&(b.id,i)];
                    fact["range"] = json!([placed.address+span.start as u32,placed.address+span.end as u32]);
                    fact["index"] = json!(i);
                    fact
                }).collect();
                // A fused comparison's span owns the terminator as well.
                if let Some(span) = m.code.mir_spans.get(&(b.id,b.ops.len())) {
                    ops.push(json!({"kind":kind,"terminator":true,"index":b.ops.len(),"uses":term_uses,
                        "definition":null,"producer":null,"consumer":consumer,"width":placed.result_bytes,
                        "range":[placed.address+span.start as u32,placed.address+span.end as u32]}));
                }
                json!({"id":b.id.0,"params":b.params.iter().map(|p|p.0.0).collect::<Vec<_>>(),
                    "ops":ops,"terminator_uses":term_uses,"successors":successors})
            }).collect();
            routines.push(json!({"id":r.id.0,"name":r.name,"address":placed.address,"size":placed.size,
                "frame_extent":m.frame.extent,"staging_slots":m.frame.edge_copies.iter().map(|s|json!({"offset":s.offset,"width":s.width})).collect::<Vec<_>>(),"edges":edges,"blocks":blocks,
                "labels":m.code.labels.values().map(|o|placed.address+*o as u32).collect::<Vec<_>>(),
                "temp_homes":r.temps.iter().map(|(id,_)|json!({"id":id.0,"home":home(m.frame.temps[id])})).collect::<Vec<_>>(),
                "objects":placed.objects,"arguments":placed.arguments}));
        }
        assert_eq!(routines.len(), a["routines"].as_array().unwrap().len());
        builds.push(json!({"case":a["case"],"mode":a["mode"],"routines":routines}));
    }
    assert_eq!(builds.len(), 28);
    std::fs::write(
        out,
        serde_json::to_string_pretty(
            &json!({"schema":1,"lf_crlf_images_equal":true,"builds":builds}),
        )
        .unwrap()
            + "\n",
    )
    .unwrap();
}
