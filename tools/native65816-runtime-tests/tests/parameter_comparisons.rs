mod support;
use actionc::mir65816::{Mir65816AddressBase, Mir65816Op};
use support::*;

#[test]
fn incoming_comparisons_cover_operands_unsigned_boundaries_and_boolean_forms() {
    let mut omitted = 0;
    for op in ["=", "#", "<", ">=", ">", "<="] {
        for reverse in [false, true] {
            for branch in [false, true] {
                let expression = if reverse {
                    format!("$8000 {op} x")
                } else {
                    format!("x {op} $8000")
                };
                let body = if branch {
                    format!("IF {expression} THEN RETURN(17) FI RETURN(23)")
                } else {
                    format!("RETURN({expression})")
                };
                let source = format!(
                    "CARD input=$7100 BYTE result=$7102\nBYTE FUNC Work(CARD x) {body}\nPROC Main() result=Work(input) RETURN\n"
                );
                for optimize in [false, true] {
                    let p = prepare(&source, optimize);
                    let c = p.compile(&layout()).unwrap();
                    assert_eq!(
                        c.image.to_json().unwrap(),
                        compile(&source.replace('\n', "\r\n"), optimize)
                            .to_json()
                            .unwrap()
                    );
                    let r = c
                        .machine
                        .prepared
                        .routines
                        .iter()
                        .find(|r| r.name == "Work")
                        .unwrap();
                    let m = c.machine.routines.iter().find(|m| m.id == r.id).unwrap();
                    for b in &r.blocks {
                        for (i, op) in b.ops.iter().enumerate() {
                            if matches!(op,Mir65816Op::Load{address,..} if matches!(address.base,Mir65816AddressBase::Parameter(_)))
                            {
                                assert!(m.code.mir_spans[&(b.id, i)].is_empty());
                                omitted += 1;
                            }
                        }
                    }
                    for input in [0u16, 1, 0x7fff, 0x8000, 0x8001, 0xffff] {
                        let (a, b) = if reverse {
                            (0x8000, input)
                        } else {
                            (input, 0x8000)
                        };
                        let yes = match op {
                            "=" => a == b,
                            "#" => a != b,
                            "<" => a < b,
                            ">=" => a >= b,
                            ">" => a > b,
                            _ => a <= b,
                        };
                        let mut h = Harness::new(&c.image, &caller(c.image.entry), 0);
                        h.bus.ram[0x7100..0x7102].copy_from_slice(&input.to_le_bytes());
                        h.run();
                        h.guards(0);
                        assert_eq!(
                            h.bus.ram[0x7102],
                            if branch {
                                if yes { 17 } else { 23 }
                            } else {
                                u8::from(yes)
                            }
                        );
                    }
                }
            }
        }
    }
    assert!(omitted >= 48);
}

#[test]
fn immutable_compare_read_survives_irq_nmi_reentry() {
    let source="MODULE TEST VOLATILE BYTE irqAck=$7800 BYTE scratch
      BYTE FUNC Work(CARD x) IF x<$8000 THEN RETURN(17) FI RETURN(23)
      CARD FUNC Dispatch(CARD saved BYTE reason) scratch=Work($8001) irqAck=1 RETURN(saved)
      PROC Task(CARD POINTER argument) argument^=CARD(Work(argument^)) RETURN PROC Main() RETURN ENDMODULE";
    windows::check_work_interrupts(source);
}
