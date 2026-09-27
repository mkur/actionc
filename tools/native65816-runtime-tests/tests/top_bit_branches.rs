mod support;
use actionc::mir65816::Mir65816Op;
use actionc::nir::NirBinaryOp;
use support::*;

fn check_type(ty: &str, bytes: usize, values: Vec<u32>) {
    let top = 1u32 << (bytes * 8 - 1);
    let mut source = format!("{ty} input=$7100 BYTE ARRAY result(8)\n");
    for i in 0..8 {
        let mask = if i & 1 == 0 {
            format!("x AND ${top:X}")
        } else {
            format!("${top:X} AND x")
        };
        let op = if i & 2 == 0 { "#" } else { "=" };
        let condition = if i & 4 == 0 {
            format!("({mask}){op}0")
        } else {
            format!("0{op}({mask})")
        };
        source += &format!("BYTE FUNC F{i}({ty} x) IF {condition} THEN RETURN(17) FI RETURN(23)\n");
    }
    source += "PROC Main()\n";
    for i in 0..8 {
        source += &format!("result({i})=F{i}(input)\n");
    }
    source += "RETURN\n";
    for optimize in [false, true] {
        let p = prepare(&source, optimize);
        let c = p.compile(&layout()).unwrap();
        assert_eq!(
            c.image.to_json().unwrap(),
            compile(&source.replace('\n', "\r\n"), optimize)
                .to_json()
                .unwrap()
        );
        let mut omitted = 0;
        for r in &c.machine.prepared.routines {
            let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
            for b in &r.blocks {
                for (i, op) in b.ops.iter().enumerate() {
                    if matches!(
                        op,
                        Mir65816Op::Binary {
                            operation: NirBinaryOp::And,
                            ..
                        }
                    ) {
                        let empty = m.code.mir_spans[&(b.id, i)].is_empty();
                        // Raw wide comparisons retain a zero-extension Cast
                        // between AND and Compare, so the adjacent rule declines.
                        assert_eq!(
                            empty,
                            optimize || bytes == 1,
                            "{ty}/{optimize}/{}: {b:?}",
                            r.name
                        );
                        omitted += usize::from(empty);
                    }
                }
            }
        }
        assert_eq!(omitted, if optimize || bytes == 1 { 8 } else { 0 });
        for &value in &values {
            let mut h = Harness::new(&c.image, &caller(c.image.entry), 0);
            h.bus.ram[0x7100..0x7100 + bytes].copy_from_slice(&value.to_le_bytes()[..bytes]);
            h.run();
            h.guards(0);
            let at = context::symbol(&c.image, "result") as usize;
            for i in 0..8 {
                let yes = (value & top != 0) ^ (i & 2 != 0);
                assert_eq!(
                    h.bus.ram[at + i],
                    if yes { 17 } else { 23 },
                    "{ty}/{optimize}/{value:x}/{i}"
                );
            }
        }
    }
}
#[test]
fn byte_and_card_masks_use_actual_width_and_all_equality_senses() {
    check_type("BYTE", 1, (0..=255).collect());
    check_type("CARD", 2, vec![0, 1, 0x7fff, 0x8000, 0x8001, 0xffff]);
}
#[test]
fn top_bit_branches_survive_interrupt_reentry() {
    for ty in ["BYTE", "CARD", "LONGCARD"] {
        let mask = if ty == "BYTE" {
            "$80"
        } else if ty == "CARD" {
            "$8000"
        } else {
            "$80000000"
        };
        let source = format!(
            "MODULE TEST VOLATILE BYTE irqAck=$7800 BYTE scratch\nBYTE FUNC Work({ty} x) IF (x AND {mask})#0 THEN RETURN(17) FI RETURN(23)\nCARD FUNC Dispatch(CARD saved BYTE reason) scratch=Work({mask}) irqAck=1 RETURN(saved)\nPROC Task(CARD POINTER argument) argument^=CARD(Work({ty}(argument^))) RETURN PROC Main() RETURN ENDMODULE"
        );
        windows::check_work_interrupts(&source);
    }
}

#[test]
fn byte_top_bit_ignores_poisoned_hidden_b_across_distant_branch_targets() {
    use actionc_vm::native65816::{Inputs, Machine};
    let source = format!(
        "BYTE input=$7100,result=$7101 VOLATILE BYTE sink=$7400\nBYTE FUNC Work(BYTE x) IF (x AND $80)=0 THEN {} RETURN(17) FI RETURN(23)\nPROC Main() result=Work(input) RETURN",
        "sink=7\n".repeat(100)
    );
    for optimize in [false, true] {
        let p = prepare(&source, optimize);
        let c = p.compile(&layout()).unwrap();
        let r = c
            .machine
            .prepared
            .routines
            .iter()
            .find(|r| r.name == "Work")
            .unwrap();
        let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
        let base = c
            .image
            .routines
            .iter()
            .find(|r| r.id == m.id.0)
            .unwrap()
            .address;
        let (block, index) = r
            .blocks
            .iter()
            .find_map(|b| {
                b.ops
                    .iter()
                    .position(|op| matches!(op, Mir65816Op::Compare { .. }))
                    .map(|i| (b, i))
            })
            .unwrap();
        let at = base + m.code.mir_spans[&(block.id, index)].start as u32;
        for value in [0, 1, 127, 128, 255] {
            for high in [0, 0x8000, 0xff00] {
                let mut h = Harness::new(&c.image, &caller(c.image.entry), 0);
                h.bus.ram[0x7100] = value;
                h.bus.map(0x7400, &[0], true);
                assert!(
                    h.cpu
                        .run_until(
                            &mut h.bus,
                            100000,
                            |_| Inputs::default(),
                            |cpu| cpu.is_instruction_boundary() && cpu.pc() == at
                        )
                        .unwrap()
                );
                let mut registers = h.cpu.registers();
                registers.a = (registers.a & 255) | high;
                h.cpu = Machine::start_at(registers);
                h.run();
                h.guards(0);
                assert_eq!(h.bus.ram[0x7101], if value < 128 { 17 } else { 23 });
            }
        }
    }
}

#[test]
fn long_top_bit_ignores_low_words_and_checks_both_equality_senses() {
    check_type(
        "LONGCARD",
        4,
        vec![
            0, 1, 0xffff, 0x7fff0000, 0x7fffffff, 0x80000000, 0x80000001, 0x8000ffff, 0xffff0000,
            0xffffffff,
        ],
    );
    check_type(
        "LONGINT",
        4,
        vec![0, 1, 0x7fffffff, 0x80000000, 0x8000ffff, 0xffffffff],
    );
}
