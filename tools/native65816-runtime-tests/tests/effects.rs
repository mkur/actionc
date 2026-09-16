mod support;
use actionc_vm::native65816::{Access, Inputs};
use support::context::*;
const SOURCE: &str = r#"
MODULE TEST
PUBLIC EXTERNAL BYTE FUNC SaveIRQ()
PUBLIC EXTERNAL PROC RestoreIRQ(BYTE token)
VOLATILE BYTE irqAck=$7800,poll=$7804,device=$7900
CARD low,high,shared=[7],observed,dispatches
BYTE outerToken,innerToken,torn,before,after
CARD FUNC Dispatch(CARD saved BYTE reason)
 irqAck=1 dispatches==+1
 IF low<>$1234 OR high<>$ABCD THEN torn=1 FI
 shared=55 poll=1
RETURN(saved)
PROC Task(BYTE POINTER unused)
 BYTE outer,inner
 observed=shared
 outer=SaveIRQ() inner=SaveIRQ()
 outerToken=outer innerToken=inner
 device=1 before=device
 low=$1234
 RestoreIRQ(inner)
 high=$ABCD
 device=2 after=device
 RestoreIRQ(outer)
 IF outer=4 THEN RestoreIRQ(0) FI
 WHILE poll=0 DO OD
 observed=shared
RETURN
PROC Main() RETURN
ENDMODULE
"#;
#[test]
fn nested_irq_tokens_hold_pending_irq_and_preserve_optimized_memory_order() {
    for optimize in [false, true] {
        for initial_mask in [0, 4] {
            let mut h = ContextHarness::new(SOURCE, optimize, "Task", &[0x7100]);
            h.bus.ram[usize::from(h.first[0].saved_s) + 10] = initial_mask;
            h.bus.map(0x7900, &[0], true);
            h.bus.watched.extend([0x7900, 0x7804]);
            let low = symbol(&h.image, "low");
            let mut pending = false;
            let mut requested = false;
            for _ in 0..100000 {
                if h.cpu.is_stopped() {
                    break;
                }
                let before = h.bus.writes.len();
                h.tick(Inputs {
                    irq: pending,
                    ..Default::default()
                });
                if h.bus.writes[before..].iter().any(|&(a, _)| a == low) {
                    assert_ne!(h.cpu.registers().p & 4, 0);
                    pending = true;
                    requested = true;
                }
                if h.bus.writes[before..].iter().any(|&(a, _)| a == IRQ_ACK) {
                    pending = false;
                }
            }
            assert!(requested && h.cpu.is_stopped());
            h.guards();
            assert_eq!(h.bus.value(DONE, 2), 1);
            for (name, width, value) in [
                ("outerToken", 1, u32::from(initial_mask)),
                ("innerToken", 1, 4),
                ("torn", 1, 0),
                ("dispatches", 2, 1),
                ("observed", 2, 55),
                ("before", 1, 1),
                ("after", 1, 2),
            ] {
                assert_eq!(
                    h.bus.value(symbol(&h.image, name), width),
                    value,
                    "{name}/{optimize}/{initial_mask}"
                );
            }
            let trace: Vec<_> = h
                .bus
                .trace
                .iter()
                .filter(|(_, a, _)| *a == 0x7900)
                .map(|(_, _, op)| *op)
                .collect();
            assert_eq!(
                trace,
                [
                    Access::Write(1),
                    Access::Read,
                    Access::Write(2),
                    Access::Read
                ]
            );
            assert!(
                h.bus
                    .trace
                    .iter()
                    .any(|(_, a, op)| *a == 0x7804 && *op == Access::Read)
            );
        }
    }
}
