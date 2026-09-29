mod support;
use actionc_vm::native65816::{Inputs, Machine};
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

#[test]
fn native_result_lanes_return_across_banks_and_indirect_wrapper_entries() {
    for (ty, bytes, mask) in [
        ("BYTE", 1, 0xffu32),
        ("INT", 2, 0xffff),
        ("SIZE", 3, 0xffffff),
        ("LONGCARD", 4, u32::MAX),
        ("BYTE POINTER", 3, 0xffffff),
    ] {
        for optimize in [false, true] {
            let p = prepare(
                &format!(
                    "{ty} input,output {ty} FUNC POINTER cb({ty} arg) {ty} FUNC Echo({ty} value) RETURN(value) {ty} FUNC Wrap({ty} value) RETURN(Echo(value)) PROC Main() cb=@Wrap output=cb(input) RETURN"
                ),
                optimize,
            );
            let machine = actionc::mir65816::emit::materialize(&p.mir).unwrap();
            let mut options = layout();
            // Leave fewer than four bytes after Echo, including linker alignment,
            // so the wrapper must start in the following program bank.
            options.code_origin = 0x020000 - machine.routines[0].code.bytes.len() as u32 - 2;
            let image = p.compile(&options).unwrap().image;
            let wrapper = image.routines.iter().find(|r| r.name == "Wrap").unwrap();
            let leaf = image.routines.iter().find(|r| r.name == "Echo").unwrap();
            assert_eq!(wrapper.size, 4);
            assert_ne!(wrapper.address >> 16, leaf.address >> 16);
            for value in [0u32, 0x80000081, 0x89abcdef, u32::MAX] {
                for irq in [0, 4] {
                    let mut h = Harness::new(&image, &caller(image.entry), irq);
                    let at = context::symbol(&image, "input") as usize;
                    h.bus.ram[at..at + bytes].copy_from_slice(&value.to_le_bytes()[..bytes]);
                    h.run();
                    h.guards(irq);
                    assert_eq!(
                        h.global(&image, "output", bytes),
                        value & mask,
                        "{ty}/{optimize}"
                    );
                    assert_eq!(h.global(&image, "cb", 3), wrapper.address);
                }
            }
        }
    }
}

const MIXED: &str = r#"
BYTE first,last,mark,zero
CARD word
SIZE address
LONGCARD wide
BYTE POINTER ptr
PROC Sink(BYTE a CARD b BYTE c SIZE d LONGCARD e BYTE POINTER p)
  first=a word=b last=c address=d wide=e ptr=p
RETURN
PROC Wrap(BYTE a CARD b BYTE c SIZE d LONGCARD e BYTE POINTER p)
  Sink(a,b,c,d,e,p)
RETURN
PROC Chain(BYTE a CARD b BYTE c SIZE d LONGCARD e BYTE POINTER p)
  Wrap(a,b,c,d,e,p)
RETURN
BYTE FUNC ZeroValue() RETURN(129)
BYTE FUNC WrapZero() RETURN(ZeroValue())
PROC SetMark() mark=1 RETURN
PROC WrapVoid() SetMark() RETURN
PROC Main()
  Chain($81,$BEEF,$A3,SIZE($FEDCBA),LONGCARD($89ABCDEF),BYTE POINTER($123456))
  zero=WrapZero() WrapVoid()
RETURN
"#;

#[test]
fn procedures_padding_and_wrapper_chains_execute_after_o65_relocation() {
    use actionc::mir65816::o65 as format;
    for optimize in [false, true] {
        let p = prepare(MIXED, optimize);
        let image = p.compile(&layout()).unwrap().image;
        for name in ["Wrap", "Chain", "WrapZero", "WrapVoid"] {
            let r = image.routines.iter().find(|r| r.name == name).unwrap();
            assert_eq!((r.size, r.fixed_frame, r.local_stack_peak), (4, 0, 0));
            assert!(r.calls.is_empty());
        }
        let file = p.compile_o65(&Default::default()).unwrap().bytes;
        for variant in 0..2 {
            let placement = o65::placement(&file, variant, vec![o65::fault(variant)]);
            let relocated = format::relocate(&file, &placement).unwrap();
            for name in ["Wrap", "Chain", "WrapZero", "WrapVoid"] {
                let r = relocated
                    .profile()
                    .routines
                    .iter()
                    .find(|r| r.name == name)
                    .unwrap();
                assert_eq!((r.size, r.frame, r.local_peak), (4, 0, 0));
            }
            for irq in [0, 4] {
                let mut h = Harness::new_o65(&relocated, &caller(relocated.entry()), irq);
                h.run();
                h.guards(irq);
                for (name, bytes, expected) in [
                    ("first", 1, 0x81),
                    ("word", 2, 0xbeef),
                    ("last", 1, 0xa3),
                    ("address", 3, 0xfedcba),
                    ("wide", 4, 0x89abcdef),
                    ("ptr", 3, 0x123456),
                    ("zero", 1, 129),
                    ("mark", 1, 1),
                ] {
                    assert_eq!(
                        h.bus.value(o65::object(&relocated, name), bytes),
                        expected,
                        "{name}"
                    );
                }
            }
        }
    }
}

#[test]
fn destination_guard_checks_exact_floor_underflow_and_ceiling_before_writes() {
    let source = "BYTE output PROC Sink(CARD value) BYTE ARRAY scratch(4) scratch(0)=BYTE(value) output=scratch(0) RETURN PROC Wrap(CARD value) Sink(value) RETURN PROC Main() RETURN";
    for optimize in [false, true] {
        let image = compile(source, optimize);
        let wrapper = image.routines.iter().find(|r| r.name == "Wrap").unwrap();
        let leaf = image.routines.iter().find(|r| r.name == "Sink").unwrap();
        assert_eq!(wrapper.size, 4);
        let frame = leaf.fixed_frame;
        assert!(frame >= 4);
        for irq in [0, 4] {
            for (initial_s, fault) in [
                (0x4019 + frame, false),
                (0x4018 + frame, true),
                (2, true),
                (0x6000, true),
            ] {
                let mut h = Harness::new(&image, &caller(image.entry), irq);
                let mut r = h.cpu.registers();
                r.s = initial_s;
                r.pbr = (wrapper.address >> 16) as u8;
                r.pc = wrapper.address as u16;
                h.cpu = Machine::start_at(r);
                assert!(
                    h.cpu
                        .run_until(
                            &mut h.bus,
                            1000,
                            |_| Inputs::default(),
                            |c| c.is_instruction_boundary()
                                && (c.pc() == image.stack_overflow || c.registers().s != initial_s)
                        )
                        .unwrap()
                );
                let r = h.cpu.registers();
                if fault {
                    assert_eq!(h.cpu.pc(), image.stack_overflow);
                    assert_eq!((r.a, r.x, r.s), (frame, initial_s, initial_s));
                } else {
                    assert_ne!(h.cpu.pc(), image.stack_overflow);
                    assert_eq!(r.s, 0x4019);
                }
                assert_eq!((r.d, r.dbr, r.p & 0x3c), (0x2000, 0, irq));
                assert!(h.bus.writes.is_empty());
            }
        }
    }
}
