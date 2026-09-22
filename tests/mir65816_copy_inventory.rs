//! Read-only typed facts for the measured edge-copy inventory. No selection changes.
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

#[test]
#[ignore = "requires A816_COMPARISON_MANIFEST and A816_COPY_INVENTORY_FACTS"]
fn export_verified_copy_inventory() {
    let sources = Sources(
        std::env::temp_dir().join(format!("actionc-copy-inventory-{}", std::process::id())),
    );
    std::fs::create_dir(&sources.0).unwrap();
    let path = PathBuf::from(std::env::var_os("A816_COMPARISON_MANIFEST").unwrap());
    let out = PathBuf::from(std::env::var_os("A816_COPY_INVENTORY_FACTS").unwrap());
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
                    let moves: Vec<_> = e.args.iter().zip(params).enumerate().map(|(i,(arg,(dest,w)))| {
                        let src = source(arg,r,m);
                        let dst = home(m.frame.temps[dest]);
                        assert_eq!(src["width"],w.get()); assert_eq!(dst["width"],w.get());
                        let stage = m.frame.edge_copies[i];
                        json!({"source":src,"destination":dst,"destination_temp":dest.0,
                            "staging":{"kind":"stack","offset":stage.offset,"width":stage.width},"width":w.get()})
                    }).collect();
                    let word = !moves.is_empty()
                        && moves.iter().all(|v| {
                            v["width"] == 2
                                && v["source"]["word_operand"] == true
                                && v["destination"]["kind"] == "stack"
                        });
                    let form = if moves.is_empty() {
                        "empty"
                    } else if word && moves.len() == 1 {
                        "direct_word"
                    } else if word {
                        "staged_word"
                    } else {
                        "staged_byte"
                    };
                    edges.push(json!({"block":b.id.0,"arm":arm,"target_block":e.target.0,
                        "block_pc":placed.address+m.code.labels[&Label(bi as u32)] as u32,
                        "transfer_pc":placed.address+transfer.offset as u32,"target_pc":placed.address+target as u32,
                        "fallthrough":transfer.fallthrough,"form":form,"moves":moves}));
                }
            }
            assert!(transfers.next().is_none());
            routines.push(json!({"id":r.id.0,"name":r.name,"address":placed.address,"size":placed.size,
                "frame_extent":m.frame.extent,"staging_slots":m.frame.edge_copies.iter().map(|s|json!({"offset":s.offset,"width":s.width})).collect::<Vec<_>>(),"edges":edges}));
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
