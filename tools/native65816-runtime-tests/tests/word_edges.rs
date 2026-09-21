mod support;
use actionc::mir65816::{self, image::Image, *};
use actionc::target::ByteSize;
use support::*;

#[test]
fn rotations_repeated_sources_live_ins_unused_and_mixed_parameters_execute() {
    for optimize in [false, true] {
        for mixed in [false, true] {
            for ordinary in [false, true] {
                let p = edges::rotation(optimize, mixed, ordinary);
                let r = p.mir.routines.iter().find(|r| r.name == "Work").unwrap();
                assert_eq!(r.blocks[1].params.len(), if mixed { 9 } else { 6 });
                let image =
                    Image::from_json(&p.compile(&layout()).unwrap().image.to_json().unwrap())
                        .unwrap();
                let caller = caller(image.entry);
                for a in [0u16, 1, 0xff, 0x100, 0x7fff, 0x8000, 0xffff] {
                    let b = a.rotate_left(8) ^ 0x5aa5;
                    for mask in [0, 4] {
                        let mut h = Harness::new(&image, &caller, mask);
                        h.bus.ram[0x7100..0x7102].copy_from_slice(&a.to_le_bytes());
                        h.bus.ram[0x7102..0x7104].copy_from_slice(&b.to_le_bytes());
                        h.run();
                        h.guards(mask);
                        assert_eq!(
                            h.bus.value(0x7200, 2),
                            u32::from(a.wrapping_sub(b).wrapping_add(a))
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn mutable_parameter_edges_use_current_home_and_preserve_full_words() {
    for optimize in [false, true] {
        let source = "CARD out=$7200 CARD FUNC Work(CARD x) x=x+1 RETURN(x) PROC Main() out=Work($FFFF) RETURN";
        let mut p = prepare(source, optimize);
        let r = p
            .mir
            .routines
            .iter_mut()
            .find(|r| r.name == "Work")
            .unwrap();
        let parameter = r.frame.parameters[0].clone();
        assert!(parameter.frame_object.is_some());
        let last = r
            .blocks
            .iter_mut()
            .find(|b| matches!(b.terminator, Mir65816Terminator::Return { .. }))
            .unwrap();
        let mut ret = last.terminator.clone();
        let Mir65816Terminator::Return {
            value: Some(Mir65816Value::Temp(id, w)),
            ..
        } = &ret
        else {
            panic!()
        };
        let (old_id, w) = (*id, *w);
        let id = actionc::nir::TempId(99);
        if let Mir65816Terminator::Return { value, .. } = &mut ret {
            *value = Some(Mir65816Value::Temp(id, w));
        }
        last.terminator = Mir65816Terminator::Goto(Mir65816Edge {
            target: actionc::nir::BlockId(99),
            args: vec![Mir65816Value::Param(parameter.param)],
        });
        r.blocks.push(Mir65816Block {
            id: actionc::nir::BlockId(99),
            params: vec![(id, w)],
            ops: vec![],
            terminator: ret,
        });
        let ty = r
            .temps
            .iter()
            .find(|(id, _)| *id == old_id)
            .unwrap()
            .1
            .clone();
        r.temps.push((id, ty));
        assert_eq!(w, ByteSize::new(2));
        mir65816::verify_program(&p.mir).unwrap();
        let image =
            Image::from_json(&p.compile(&layout()).unwrap().image.to_json().unwrap()).unwrap();
        for mask in [0, 4] {
            let mut h = Harness::new(&image, &caller(image.entry), mask);
            h.run();
            h.guards(mask);
            assert_eq!(h.bus.value(0x7200, 2), 0);
        }
    }
}
