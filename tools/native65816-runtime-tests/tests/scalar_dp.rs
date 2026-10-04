mod support;
use actionc::{compiler::native65816, mir65816::image::Image};
use actionc::{
    mir65816::{
        self,
        emit::{self, Label, Location},
        *,
    },
    nir::{BlockId, TempId},
    target::ByteSize,
};
use actionc_vm::native65816::Inputs;
use support::*;

fn capacity_program(n: u32, optimize: bool) -> native65816::Prepared {
    let mut p = prepare(
        "CARD FUNC Work(CARD n) RETURN(n) PROC Main() RETURN",
        optimize,
    );
    let r = &mut p.mir.routines[0];
    let ty = r.temps[0].1.clone();
    r.temps = (0..n).map(|i| (TempId(i), ty.clone())).collect();
    let mut ret = r.blocks[0].terminator.clone();
    if let Mir65816Terminator::Return { value, .. } = &mut ret {
        *value = Some(Mir65816Value::Temp(TempId(n - 1), ByteSize::new(2)));
    }
    r.blocks = vec![
        Mir65816Block {
            id: BlockId(0),
            params: vec![],
            ops: vec![],
            terminator: Mir65816Terminator::Goto(Mir65816Edge {
                target: BlockId(1),
                args: (0..n)
                    .map(|i| Mir65816Value::U16(0x8000 + i as u16 * 0x101))
                    .collect(),
            }),
        },
        Mir65816Block {
            id: BlockId(1),
            params: (0..n).map(|i| (TempId(i), ByteSize::new(2))).collect(),
            ops: vec![],
            terminator: ret,
        },
    ];
    mir65816::verify_program(&p.mir).unwrap();
    p
}

#[test]
fn capacity_edges_execute_every_home_and_preserve_trace_output() {
    for optimize in [false, true] {
        for n in [1, 16, 17] {
            let p = capacity_program(n, optimize);
            let c = p.compile(&layout()).unwrap();
            let image = Image::from_json(&c.image.to_json().unwrap()).unwrap();
            let plain = emit::materialize(&p.mir).unwrap();
            let (traced, _) = emit::proof::materialize_with_trace(&p.mir).unwrap();
            for (a, b) in plain.routines.iter().zip(&traced.routines) {
                assert_eq!(a.code.bytes, b.code.bytes);
                assert_eq!(a.code.labels, b.code.labels);
                assert_eq!(
                    format!("{:?}", a.code.fixups),
                    format!("{:?}", b.code.fixups)
                );
                assert_eq!(a.code.mir_spans, b.code.mir_spans);
                assert_eq!(a.code.return_fixups, b.code.return_fixups);
            }
            let m = &c.machine.routines[0];
            let r = &image.routines[0];
            assert!(m.frame.edge_copies.is_empty());
            assert_eq!(m.frame.extent, if n <= 16 { 0 } else { 36 });
            for home in m.frame.temps.values() {
                assert_eq!(matches!(home, Location::DirectPage(_)), n <= 16);
            }
            let target = r.address + m.code.labels[&Label(1)] as u32;
            let caller = assemble_artifact(
                &format!(
                    "tsc\nsec\nsbc #3\ntcs\njsl ${:06x}\n.export returned\nreturned: sta f:$007200\ntsc\nclc\nadc #3\ntcs\nstp\nnop",
                    r.address
                ),
                0x040000,
            );
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller.bytes, mask);
                h.bus.ram[0x20a0..0x20c0].fill(0xa7);
                assert!(
                    h.cpu
                        .run_until(
                            &mut h.bus,
                            10000,
                            |_| Inputs::default(),
                            |c| c.is_instruction_boundary() && c.pc() == target
                        )
                        .unwrap()
                );
                let regs = h.cpu.registers();
                for (id, home) in &m.frame.temps {
                    let at = homes::address(regs.s, regs.d, homes::of(*home).unwrap());
                    assert_eq!(h.bus.value(at, 2), u32::from(0x8000 + id.0 as u16 * 0x101));
                }
                if n > 16 {
                    assert_eq!(&h.bus.ram[0x20a0..0x20c0], &[0xa7; 32]);
                }
                h.run();
                h.guards(mask);
                assert_eq!(
                    h.bus.value(0x7200, 2),
                    u32::from(0x8000 + (n - 1) as u16 * 0x101)
                );
            }
            if let Ok(directory) = std::env::var("A816_QUALIFICATION_DIR") {
                let stem = std::path::Path::new(&directory)
                    .join(format!("scalar-dp-capacity-{n}-{optimize}"));
                std::fs::write(stem.with_extension("a816.json"), image.to_json().unwrap()).unwrap();
                std::fs::write(stem.with_extension("caller.bin"), caller.bytes).unwrap();
            }
        }
    }
}

#[test]
fn zero_frame_scalar_entry_inherits_the_checked_caller_stack() {
    for optimize in [false, true] {
        let image = compile(
            "CARD FUNC Work(CARD n) RETURN(n+1) PROC Main() RETURN",
            optimize,
        );
        let work = &image.routines[0];
        assert_eq!((work.fixed_frame, work.local_stack_peak), (0, 0));
        let caller = assemble_artifact(
            &format!(
                "tsc\nsec\nsbc #3\ntcs\nlda f:$007100\nsta 1,s\njsl ${:06x}\n.export returned\nreturned: sta f:$007200\ntsc\nclc\nadc #3\ntcs\nstp\nnop",
                work.address
            ),
            0x040000,
        );
        for value in [0u16, 0x7fff, 0xffff] {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller.bytes, mask);
                h.bus.ram[0x7100..0x7102].copy_from_slice(&value.to_le_bytes());
                for pc in [work.address, caller.symbols["returned"]] {
                    assert!(
                        h.cpu
                            .run_until(
                                &mut h.bus,
                                10000,
                                |_| Inputs::default(),
                                |c| c.is_instruction_boundary() && c.pc() == pc
                            )
                            .unwrap()
                    );
                    if pc == work.address {
                        // A zero-frame entry makes no new stack reservation;
                        // the independent caller already owns its argument area.
                        h.bus.writes.clear();
                    }
                }
                assert!(
                    !h.bus
                        .writes
                        .iter()
                        .any(|(a, _)| (0x4000..0x6000).contains(a))
                );
                assert_eq!(h.cpu.registers().a, value.wrapping_add(1));
                h.run();
                h.guards(mask);
                assert_eq!(h.bus.value(0x7200, 2), u32::from(value.wrapping_add(1)));
            }
        }
    }
}
