use super::*;

fn program() -> Mir65816Program {
    let source = "ADDRESS FUNC Echo(ADDRESS p) RETURN(p) ADDRESS FUNC Mutate(ADDRESS p) p=ADDRESS(0) RETURN(p) PROC Main() RETURN";
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model = crate::semantic::analyze_with_options(
        &ast,
        crate::semantic::SemanticOptions::modern()
            .with_target(crate::target::TargetId::Wdc65816Native),
    )
    .unwrap();
    let nir = crate::nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model));
    crate::nir::verify_program(&nir).unwrap();
    crate::mir65816::lower_program(&nir).unwrap()
}
fn builder(routine: &Mir65816Routine) -> Builder<'_> {
    let frame = AllocatedFrame::stack(routine).unwrap();
    Builder {
        routine,
        code: TrackedEmitter65816::for_test(&frame),
        frame,
        blocks: BTreeMap::new(),
        next_block: None,
        loop_x: None,
    }
}
fn input(b: &mut Builder<'_>, src: Location, dst: Location) -> Mir65816Value {
    b.frame.temps.insert(TempId(998), src);
    b.frame.temps.insert(TempId(999), dst);
    Mir65816Value::Temp(TempId(998), ByteSize::new(3))
}
fn stack(offset: u16) -> Location {
    Location::Stack(Slot { offset, width: 3 })
}
fn dp(offset: u16) -> Location {
    Location::DirectPage(Slot { offset, width: 3 })
}
fn encoding(memory: Memory, load: bool, byte: u8) -> [u8; 2] {
    match memory {
        Memory::Stack(at) => [if load { 0xa3 } else { 0x83 }, at as u8 + byte],
        Memory::DirectPage(at) => [if load { 0xa5 } else { 0x85 }, at as u8 + byte],
        _ => panic!(),
    }
}
#[test]
fn native_pointer_casts_copy_exact_private_extents_in_both_entry_widths() {
    let p = program();
    for r in &p.routines[..2] {
        for (src, dst) in [
            (stack(250), stack(253)),
            (stack(253), stack(253)),
            (dp(61), dp(0)),
            (dp(0), dp(0)),
            (stack(253), dp(61)),
            (dp(61), stack(253)),
        ] {
            for byte in [false, true] {
                let mut b = builder(r);
                let value = input(&mut b, src, dst);
                if byte {
                    b.code.a8();
                } else {
                    b.code.a16();
                }
                let at = b.code.position();
                let frame = b.frame.clone();
                assert!(b.pointer_cast(TempId(999), &value).unwrap());
                let mut expected = if byte { vec![0xc2, 0x20] } else { vec![] };
                for offset in [0, 1] {
                    expected.extend(encoding(src.into(), true, offset));
                    expected.extend(encoding(dst.into(), false, offset));
                }
                assert_eq!(&b.code.code().bytes[at..], expected);
                assert_eq!(b.frame.temps, frame.temps);
                assert_eq!(b.frame.extent, frame.extent);
            }
        }
        let mut b = builder(r);
        b.frame.temps.insert(TempId(999), dp(0));
        let value = Mir65816Value::Param(r.frame.parameters[0].param);
        let memory = b.value_memory(&value).unwrap().unwrap();
        b.code.a16();
        let at = b.code.position();
        assert!(b.pointer_cast(TempId(999), &value).unwrap());
        let mut expected = Vec::new();
        for offset in [0, 1] {
            expected.extend(encoding(memory, true, offset));
            expected.extend(encoding(dp(0).into(), false, offset));
        }
        assert_eq!(&b.code.code().bytes[at..], expected);
    }
}
#[test]
fn pointer_copy_rejects_incomplete_homes_atomically_and_keeps_overlap_fallback() {
    let p = program();
    for (src, dst, delta, error) in [
        (stack(0), stack(10), 0, true),
        (stack(254), stack(10), 0, true),
        (stack(253), stack(10), 1, true),
        (dp(62), stack(10), 0, true),
        (stack(10), dp(62), 0, true),
        (stack(10), stack(254), 0, true),
        (stack(10), stack(11), 0, false),
        (dp(0), dp(2), 0, false),
    ] {
        let mut b = builder(&p.routines[0]);
        let value = input(&mut b, src, dst);
        b.code.test_delta(delta);
        b.code.a8();
        let before = format!("{:?}", b.code);
        let result = b.pointer_cast(TempId(999), &value);
        if error {
            assert!(result.is_err());
        } else {
            assert_eq!(result, Ok(false));
        }
        assert_eq!(format!("{:?}", b.code), before);
    }
    for value in [
        Mir65816Value::U24(0x123456),
        Mir65816Value::Null(ByteSize::new(3)),
        Mir65816Value::RoutineAddress(0, ByteSize::new(3)),
        Mir65816Value::U16(0xffff),
    ] {
        let mut b = builder(&p.routines[0]);
        b.frame.temps.insert(TempId(999), stack(10));
        assert!(!b.pointer_cast(TempId(999), &value).unwrap());
        assert!(b.code.code().bytes.is_empty());
    }
}
