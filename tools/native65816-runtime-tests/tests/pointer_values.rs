mod support;
use support::*;

#[test]
fn captured_pointer_casts_preserve_every_lane_and_mutable_parameter_homes() {
    let source = "ADDRESS input=$7100 ADDRESS ARRAY output=$7200 \
        ADDRESS FUNC Cast(ADDRESS value) BYTE POINTER p \
        p=BYTE POINTER(value) RETURN(ADDRESS(p)) \
        ADDRESS FUNC Mutate(ADDRESS value) BYTE POINTER p \
        value=ADDRESS(BYTE POINTER(value)) p=BYTE POINTER(value) RETURN(ADDRESS(p)) \
        PROC Main() output(0)=Cast(input) output(1)=Mutate(input) RETURN";
    for optimize in [false, true] {
        let image = compile(source, optimize);
        let caller = caller(image.entry);
        for value in [0u32, 0xffffff, 0x12ffff, 0x130000, 0xabcdef]
            .into_iter()
            .chain((0..24).map(|bit| 1 << bit))
        {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                h.bus.ram[0x7100..0x7103].copy_from_slice(&value.to_le_bytes()[..3]);
                h.bus.ram[0x71ff..0x7207].fill(0xa5);
                h.run();
                h.guards(mask);
                assert_eq!(h.bus.value(0x7200, 3), value);
                assert_eq!(h.bus.value(0x7203, 3), value);
                assert_eq!((h.bus.ram[0x71ff], h.bus.ram[0x7206]), (0xa5, 0xa5));
            }
        }
    }
}

#[test]
fn zero_offset_field_addresses_preserve_all_bits_without_dereferencing() {
    let source = "TYPE Box=[BYTE first CARD second] \
        Box POINTER input=$7100 ADDRESS output=$7200 \
        ADDRESS FUNC Work(Box POINTER p) RETURN(ADDRESS(@p.first)) \
        PROC Main() output=Work(input) RETURN";
    for optimize in [false, true] {
        let image = compile(source, optimize);
        let caller = caller(image.entry);
        for value in [0u32, 0xffffff, 0x12ffff, 0x130000, 0xabcdef]
            .into_iter()
            .chain((0..24).map(|bit| 1 << bit))
        {
            for mask in [0, 4] {
                let mut h = Harness::new(&image, &caller, mask);
                h.bus.ram[0x7100..0x7103].copy_from_slice(&value.to_le_bytes()[..3]);
                h.bus.ram[0x71ff..0x7204].fill(0xa5);
                h.run();
                h.guards(mask);
                assert_eq!(h.bus.value(0x7200, 3), value);
                assert_eq!((h.bus.ram[0x71ff], h.bus.ram[0x7203]), (0xa5, 0xa5));
            }
        }
    }
}

#[test]
fn constant_field_addresses_carry_through_the_bank_and_wrap_at_24_bits() {
    for offset in [1u32, 3, 255, 256, 65535, 65536] {
        let mut padding = String::new();
        let mut remaining = offset;
        let mut index = 0;
        while remaining != 0 {
            let size = remaining.min(16384);
            padding.push_str(&format!("BYTE ARRAY padding{index}({size}) "));
            remaining -= size;
            index += 1;
        }
        let source = format!(
            "TYPE Box=[{padding}BYTE last] \
            Box POINTER input=$7100 ADDRESS output=$7200 \
            ADDRESS FUNC Work(Box POINTER p) RETURN(ADDRESS(@p.last)) \
            PROC Main() output=Work(input) RETURN"
        );
        for optimize in [false, true] {
            let image = compile(&source, optimize);
            let caller = caller(image.entry);
            for value in [
                0u32, 1, 0xff, 0xffff, 0x10000, 0x12fffd, 0x12ffff, 0xfffffe, 0xffffff,
            ] {
                for mask in [0, 4] {
                    let mut h = Harness::new(&image, &caller, mask);
                    h.bus.ram[0x7100..0x7103].copy_from_slice(&value.to_le_bytes()[..3]);
                    h.bus.ram[0x71ff..0x7204].fill(0xa5);
                    h.run();
                    h.guards(mask);
                    assert_eq!(
                        h.bus.value(0x7200, 3),
                        (value + offset) & 0xffffff,
                        "{optimize}/{value:x}/{offset}"
                    );
                    assert_eq!((h.bus.ram[0x71ff], h.bus.ram[0x7203]), (0xa5, 0xa5));
                }
            }
        }
    }
}

fn check_pointer_computation_irqs(expression: &str, inputs: [u32; 3], expected: [u32; 3]) {
    use actionc_vm::native65816::Inputs;
    use std::collections::BTreeSet;
    use support::context::*;
    let source = format!(
        "MODULE TEST PUBLIC EXTERNAL PROC Yield() \
        VOLATILE BYTE irqAck=$7800 CARD taskA=$7000,taskB=$7002 BYTE current \
        TYPE Box=[BYTE ARRAY padding(3) BYTE last] \
        TYPE Job=[Box POINTER item BYTE done ADDRESS result BYTE POINTER peer] \
        ADDRESS irqResult \
        ADDRESS FUNC Form(Box POINTER p) RETURN({expression}) \
        CARD FUNC Dispatch(CARD saved BYTE reason) \
        irqAck=1 irqResult=Form(Box POINTER({} )) \
        IF current=0 THEN taskA=saved current=1 RETURN(taskB) FI \
        taskB=saved current=0 RETURN(taskA) \
        PROC Task(Job POINTER work) work.result=Form(work.item) work.done=1 \
        WHILE work.peer^=0 DO Yield() OD RETURN \
        PROC Main() RETURN ENDMODULE",
        inputs[2]
    );
    let check = |h: &ContextHarness| {
        h.guards();
        assert_eq!(h.bus.value(DONE, 2), 1);
        assert_eq!(h.bus.value(0x7103, 1), 1);
        assert_eq!(h.bus.value(0x7123, 1), 1);
        assert_eq!(h.bus.value(0x7104, 3), expected[0]);
        assert_eq!(h.bus.value(0x7124, 3), expected[1]);
        assert_eq!(
            h.bus.value(context::symbol(&h.image, "irqResult"), 3),
            expected[2]
        );
    };
    for optimize in [false, true] {
        let mut h = ContextHarness::new(&source, optimize, "Task", &[0x7100, 0x7120]);
        for (job, value, peer) in [
            (0x7100usize, inputs[0], 0x7123u32),
            (0x7120, inputs[1], 0x7103),
        ] {
            h.bus.ram[job..job + 3].copy_from_slice(&value.to_le_bytes()[..3]);
            h.bus.ram[job + 7..job + 10].copy_from_slice(&peer.to_le_bytes()[..3]);
        }
        let start = context::routine(&h.image, "Form");
        let end = start
            + h.image
                .routines
                .iter()
                .find(|r| r.address == start)
                .unwrap()
                .size;
        let mut seen = BTreeSet::new();
        for _ in 0..2_000_000 {
            if h.cpu.is_stopped() {
                break;
            }
            let r = h.cpu.registers();
            if h.cpu.is_instruction_boundary()
                && r.p & 4 == 0
                && [0x2000, 0x2100].contains(&r.d)
                && (start..end).contains(&h.cpu.pc())
                && seen.insert((r.d, h.cpu.pc()))
            {
                let saved_cpu = h.cpu.clone();
                let saved_bus = h.bus.clone();
                let mut pending = true;
                for tick in 0..2_000_000 {
                    if h.cpu.is_stopped() {
                        break;
                    }
                    let writes = h.bus.writes.len();
                    // NMI also interrupts the IRQ/dispatcher path while the
                    // dispatcher evaluates the same address computation.
                    h.tick(Inputs {
                        irq: pending,
                        nmi: tick == 40,
                        ..Default::default()
                    });
                    if h.bus.writes[writes..].iter().any(|&(at, _)| at == IRQ_ACK) {
                        pending = false;
                    }
                }
                check(&h);
                h.cpu = saved_cpu;
                h.bus = saved_bus;
            }
            h.tick(Inputs::default());
        }
        check(&h);
        assert!(
            seen.len() >= 40,
            "only {} address-forming interrupt sites",
            seen.len()
        );
    }
}

#[test]
fn constant_address_carry_survives_irq_and_nmi_with_reentrant_computation() {
    check_pointer_computation_irqs(
        "ADDRESS(@p.last)",
        [0xfffffe, 0x12ffff, 0xffffff],
        [1, 0x130002, 2],
    );
}
#[test]
fn pointer_step_carry_and_borrow_survive_irq_nmi_and_reentrant_computation() {
    check_pointer_computation_irqs(
        "ADDRESS(p)+SIZE(1)",
        [0xffffff, 0x12ffff, 0xffff],
        [0, 0x130000, 0x10000],
    );
    check_pointer_computation_irqs(
        "ADDRESS(p)-SIZE(1)",
        [0, 0x130000, 0x10000],
        [0xffffff, 0x12ffff, 0xffff],
    );
}

#[test]
fn pointer_steps_wrap_in_both_directions_with_exact_external_capture_traces() {
    use actionc_vm::native65816::Access;
    let source = "VOLATILE ADDRESS input=$7100 ADDRESS ARRAY output=$7200 \
        ADDRESS FUNC Up(ADDRESS p) RETURN(p+SIZE(1)) \
        ADDRESS FUNC Down(ADDRESS p) RETURN(p-SIZE(1)) \
        ADDRESS FUNC Mutable(ADDRESS p) p=p+SIZE(1) RETURN(p-SIZE(1)) \
        ADDRESS FUNC Fallback(ADDRESS p) RETURN(p+SIZE(2)) \
        BYTE POINTER FUNC Inc(BYTE POINTER p) p==+1 RETURN(p) \
        BYTE POINTER FUNC Dec(BYTE POINTER p) p==-1 RETURN(p) \
        PROC Main() output(0)=Up(input) output(1)=Down(input) \
        output(2)=Mutable(input) output(3)=Fallback(input) \
        output(4)=ADDRESS(Inc(BYTE POINTER(input))) \
        output(5)=ADDRESS(Dec(BYTE POINTER(input))) RETURN";
    for optimize in [false, true] {
        let image = compile(source, optimize);
        let object = o65::compile(source, optimize, vec![]);
        let mut relocated = vec![];
        for variant in 0..2 {
            relocated.push(
                actionc::mir65816::o65::relocate(
                    &object,
                    &o65::placement(&object, variant, vec![o65::fault(variant)]),
                )
                .unwrap(),
            );
        }
        for value in [
            0u32, 1, 0xff, 0x100, 0xffff, 0x10000, 0x7fffff, 0x800000, 0xfffffe, 0xffffff,
        ]
        .into_iter()
        .chain((0..24).map(|n| 1 << n))
        {
            for mask in [0, 4] {
                for h in std::iter::once(Harness::new(&image, &caller(image.entry), mask)).chain(
                    relocated
                        .iter()
                        .map(|r| Harness::new_o65(r, &caller(r.entry()), mask)),
                ) {
                    let mut h = h;
                    h.bus.ram[0x7100..0x7103].copy_from_slice(&value.to_le_bytes()[..3]);
                    h.bus.ram[0x71ff..0x7213].fill(0xa5);
                    h.bus.watched = (0x7100..0x7103).collect();
                    h.run();
                    h.guards(mask);
                    let up = (value + 1) & 0xffffff;
                    let down = value.wrapping_sub(1) & 0xffffff;
                    for (i, expected) in [up, down, value, (value + 2) & 0xffffff, up, down]
                        .into_iter()
                        .enumerate()
                    {
                        assert_eq!(h.bus.value(0x7200 + 3 * i as u32, 3), expected);
                    }
                    assert_eq!((h.bus.ram[0x71ff], h.bus.ram[0x7212]), (0xa5, 0xa5));
                    assert_eq!(
                        h.bus
                            .trace
                            .iter()
                            .map(|&(_, at, op)| (at, op))
                            .collect::<Vec<_>>(),
                        (0..6)
                            .flat_map(|_| (0x7100..0x7103).map(|at| (at, Access::Read)))
                            .collect::<Vec<_>>()
                    );
                }
            }
        }
    }
}
