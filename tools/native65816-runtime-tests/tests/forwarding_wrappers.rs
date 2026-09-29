mod support;
use actionc_vm::native65816::Inputs;
use support::*;

#[test]
fn pointer_wrapper_reuses_the_original_argument_and_return_without_writes() {
    let source = "TYPE A=[BYTE value] TYPE B=[CARD value] BYTE output BYTE FUNC Read(A POINTER p) RETURN(p.value) BYTE FUNC Wrap(B POINTER p) RETURN(Read(A POINTER(p))) PROC Main() output=Wrap(B POINTER($7100)) RETURN";
    for optimize in [false, true] {
        let image = compile(source, optimize);
        let wrapper = image.routines.iter().find(|r| r.name == "Wrap").unwrap();
        let callee = image.routines.iter().find(|r| r.name == "Read").unwrap();
        assert_eq!(
            (
                wrapper.size,
                wrapper.fixed_frame,
                wrapper.spill_bytes,
                wrapper.local_stack_peak
            ),
            (4, 0, 0, 0)
        );
        assert!(wrapper.calls.is_empty() && wrapper.temporaries.is_empty());
        assert_eq!(wrapper.arguments[0].body_displacement, 4);
        for mask in [0, 4] {
            let mut h = Harness::new(&image, &caller(image.entry), mask);
            h.bus.ram[0x7100] = 0x81;
            assert!(
                h.cpu
                    .run_until(
                        &mut h.bus,
                        10000,
                        |_| Inputs::default(),
                        |c| c.is_instruction_boundary() && c.pc() == wrapper.address
                    )
                    .unwrap()
            );
            let before = h.cpu.registers();
            let writes = h.bus.writes.len();
            assert!(
                h.cpu
                    .run_until(
                        &mut h.bus,
                        100,
                        |_| Inputs::default(),
                        |c| c.is_instruction_boundary() && c.pc() == callee.address
                    )
                    .unwrap()
            );
            let mut expected = before;
            expected.pbr = (callee.address >> 16) as u8;
            expected.pc = callee.address as u16;
            assert_eq!(h.cpu.registers(), expected);
            assert_eq!(h.bus.writes.len(), writes);
            h.run();
            h.guards(mask);
            assert_eq!(h.global(&image, "output", 1), 0x81);
        }
    }
}
