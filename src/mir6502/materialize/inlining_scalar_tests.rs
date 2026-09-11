fn execute_scalar(image: &Image, entry: RoutineId, input: &[u8]) -> ([u8; 4], [u8; 4]) {
    let mut memory = [0u8; 65536];

    memory[0x2000..0x2000 + image.bytes.len()].copy_from_slice(&image.bytes);
    memory[0x600..0x600 + input.len()].copy_from_slice(input);
    let start = image
        .blocks
        .iter()
        .find(|(r, _, _)| *r == entry)
        .unwrap()
        .2
        .start;
    super::super::leaf_test_cpu::run_memory(&mut memory, 0x2000 + start);
    (
        memory[0x610..0x614].try_into().unwrap(),
        memory[0xA0..0xA4].try_into().unwrap(),
    )
}

fn expand_scalar_control(source: &str) -> (MirProgram, Image) {
    let mut program = lower(source);
    let census = analyze(&program);
    let leaf = census
        .leaves
        .get(&RoutineId(0))
        .unwrap_or_else(|| panic!("{census:?}"));
    expand_group(&mut program.routines[1], leaf).unwrap();
    crate::mir6502::verify_program(&program, MirPhase::PreMaterialization).unwrap();
    let mut config = Mir6502Config::optimized();
    config.enable_small_leaf_inlining = false;
    let program = crate::mir6502::materialize_program_with_origin_and_runtime(
        program,
        &config,
        0x2000,
        Runtime::ActionCart,
    )
    .unwrap();
    let image = Image::build(&program, 0x2000).unwrap();
    (program, image)
}

fn compile_scalar_image(source: &str, inline: bool) -> (MirProgram, Image) {
    let mut config = Mir6502Config::optimized();
    config.enable_small_leaf_inlining = inline;
    let program = crate::mir6502::materialize_program_with_origin_and_runtime(
        lower(source),
        &config,
        0x2000,
        Runtime::ActionCart,
    )
    .unwrap();
    let image = Image::build(&program, 0x2000).unwrap();
    (program, image)
}

#[test]
fn requested_word_and_long_returns_preserve_signed_extension_and_truncation() {
    for (ty, expression) in [
        ("INT", "value+1"),
        ("LONGINT", "LONGINT(value)"),
        ("LONGINT", "LONGINT(value)+65536"),
        ("BYTE", "BYTE(value)"),
    ] {
        let source = format!(
            "INT input=$600 {ty} output=$610 INLINE {ty} FUNC Map(INT value) RETURN({expression}) PROC Main() output=Map(input) RETURN"
        );
        let (before, old) = compile_scalar_image(&source, false);
        let (_, selected) = compile_scalar_image(&source, true);
        let (after, new) = expand_scalar_control(&source);
        assert_eq!(calls_to(&before, RoutineId(0)), 1);
        assert_eq!(calls_to(&after, RoutineId(0)), 0, "{source}");
        for input in [i16::MIN, -32767, -257, -256, -1, 0, 1, 255, 256, i16::MAX] {
            let actual = execute_scalar(&new, RoutineId(1), &input.to_le_bytes());
            assert_eq!(
                actual,
                execute_scalar(&old, RoutineId(1), &input.to_le_bytes()),
                "{source}: {input}"
            );
            assert_eq!(
                actual,
                execute_scalar(&selected, RoutineId(1), &input.to_le_bytes())
            );
            let expected = match expression {
                "value+1" => (input.wrapping_add(1) as u16).to_le_bytes().to_vec(),
                "LONGINT(value)" => i32::from(input).to_le_bytes().to_vec(),
                "LONGINT(value)+65536" => (i32::from(input) + 65536).to_le_bytes().to_vec(),
                _ => vec![input as u8],
            };
            assert_eq!(&actual.0[..expected.len()], &expected);
        }
    }
}

#[test]
fn requested_long_arguments_capture_both_lanes_and_keep_results_across_calls() {
    for (ty, op) in [("LONGCARD", "+"), ("LONGINT", "-")] {
        let source = format!(
            "{ty} a=$600,b=$604,output=$610 INLINE {ty} FUNC Combine({ty} left,right) RETURN(left{op}right) PROC Main() output=Combine(a,b)+Combine(b,a) RETURN"
        );
        let (before, _) = compile_scalar_image(&source, false);
        let (selected, selected_image) = compile_scalar_image(&source, true);
        let (after, new) = expand_scalar_control(&source);
        assert_eq!(calls_to(&before, RoutineId(0)), 2);
        assert_eq!(calls_to(&after, RoutineId(0)), 0, "{source}");
        for a in [
            0u32,
            1,
            255,
            65535,
            65536,
            0x7fff_ffff,
            0x8000_0000,
            u32::MAX,
        ] {
            for b in [0u32, 1, 256, 65535, 0x8000_0001, u32::MAX] {
                let input: Vec<_> = a.to_le_bytes().into_iter().chain(b.to_le_bytes()).collect();
                let actual = execute_scalar(&new, RoutineId(1), &input);
                // The cartridge SArgs control needs cartridge bank switching;
                // inline_scalar.rs executes the retained calls in the full VM.
                if calls_to(&selected, RoutineId(0)) == 0 {
                    assert_eq!(
                        actual,
                        execute_scalar(&selected_image, RoutineId(1), &input)
                    );
                }
                let expected = if op == "+" {
                    a.wrapping_add(b).wrapping_mul(2)
                } else {
                    0
                };
                assert_eq!(u32::from_le_bytes(actual.0), expected);
            }
        }
    }
}

#[test]
fn wide_call_results_are_explicit_and_malformed_lanes_are_rejected() {
    let source = "LONGCARD input,output INLINE LONGCARD FUNC Map(LONGCARD value) RETURN(value+1) PROC Main() output=Map(input) RETURN";
    let program = lower(source);
    let call = program.routines[1]
        .blocks
        .iter()
        .flat_map(|b| &b.ops)
        .find(|op| matches!(op, MirOp::Call { .. }))
        .unwrap();
    let MirOp::Call {
        result,
        additional_results,
        abi,
        ..
    } = call
    else {
        unreachable!()
    };
    assert_eq!(result.as_ref().unwrap().width, MirWidth::Word);
    assert_eq!(additional_results.len(), 1);
    assert_eq!(abi.additional_results.len(), 1);
    assert!(
        !program.routines[1]
            .blocks
            .iter()
            .flat_map(|b| &b.ops)
            .any(|op| matches!(
                op,
                MirOp::Load {
                    src: MirAddr::Direct(MirMem::FixedZeroPage(MirFixedZpSlot(0xA2))),
                    ..
                }
            ))
    );
    for kind in 0..5 {
        let mut broken = program.clone();
        let MirOp::Call {
            result,
            additional_results,
            abi,
            ..
        } = broken.routines[1]
            .blocks
            .iter_mut()
            .flat_map(|b| &mut b.ops)
            .find(|op| matches!(op, MirOp::Call { .. }))
            .unwrap()
        else {
            unreachable!()
        };
        match kind {
            0 => abi.additional_results.clear(),
            1 => additional_results[0].dst = result.as_ref().unwrap().dst.clone(),
            2 => additional_results[0].width = MirWidth::Byte,
            3 => abi.additional_results[0].home = MirResultHome::ReturnSlot { offset: 1 },
            _ => additional_results.push(additional_results[0].clone()),
        }
        assert!(
            crate::mir6502::verify_program(&broken, MirPhase::PreMaterialization).is_err(),
            "mutation {kind}"
        );
    }
}

#[test]
fn mixed_width_multiple_returns_discarded_and_high_only_results_preserve_public_slots() {
    for consumer in [
        "output=Mix(input,bump)",
        "Mix(input,bump)",
        "output=LONGINT(LONGCARD(Mix(input,bump)) RSH 16)",
    ] {
        let source = format!(
            "INT input=$600 BYTE bump=$602 LONGINT output=$610 INLINE LONGINT FUNC Mix(INT left,BYTE right) IF left<0 THEN RETURN(LONGINT(left)-LONGINT(right)) FI RETURN(LONGINT(left)+LONGINT(right)) PROC Main() {consumer} RETURN"
        );
        let (_, old) = compile_scalar_image(&source, false);
        let (_, selected) = compile_scalar_image(&source, true);
        let (expanded, new) = expand_scalar_control(&source);
        assert_eq!(calls_to(&expanded, RoutineId(0)), 0);
        for a in [i16::MIN, -1, 0, 1, i16::MAX] {
            for b in [0u8, 1, 255] {
                let input: Vec<_> = a.to_le_bytes().into_iter().chain([b]).collect();
                let actual = execute_scalar(&new, RoutineId(1), &input);
                assert_eq!(
                    actual,
                    execute_scalar(&old, RoutineId(1), &input),
                    "{consumer}: {a},{b}"
                );
                assert_eq!(actual, execute_scalar(&selected, RoutineId(1), &input));
                let value = i32::from(a) + if a < 0 { -i32::from(b) } else { i32::from(b) };
                let expected = if consumer.starts_with("Mix") {
                    0
                } else if consumer.contains("RSH") {
                    ((value as u32) >> 16) as i32
                } else {
                    value
                };
                assert_eq!(i32::from_le_bytes(actual.0), expected);
                assert_eq!(i32::from_le_bytes(actual.1), value);
            }
        }
    }
}

#[test]
fn pointer_parameters_do_not_gain_integer_inline_eligibility() {
    let source = "BYTE POINTER input BYTE output INLINE BYTE FUNC Map(BYTE POINTER value) RETURN(1) PROC Main() output=Map(input) RETURN";
    let program = lower(source);
    assert!(program.routines[0].scalar_signature.is_none());
    assert_eq!(
        analyze(&program).rejected[&RoutineId(0)],
        "non-scalar-signature"
    );
}

#[test]
fn promoted_branch_scratch_and_wide_relays_execute_without_private_frames() {
    let source = "INT input=$600 LONGINT output=$610 INLINE LONGINT FUNC Map(INT value) LONGINT scratch,relay IF value<0 THEN scratch=LONGINT(value)-1 ELSE scratch=LONGINT(value)+1 FI relay=scratch RETURN(relay) PROC Main() output=Map(input) RETURN";
    assert!(lower(source).routines[0].frame.locals.is_empty());
    let (_, old) = compile_scalar_image(source, false);
    let (_, new) = expand_scalar_control(source);
    for input in [i16::MIN, -1, 0, 1, i16::MAX] {
        let actual = execute_scalar(&new, RoutineId(1), &input.to_le_bytes());
        assert_eq!(
            actual,
            execute_scalar(&old, RoutineId(1), &input.to_le_bytes())
        );
        assert_eq!(
            i32::from_le_bytes(actual.0),
            i32::from(input) + if input < 0 { -1 } else { 1 }
        );
    }
}

#[test]
fn retained_persistent_counters_and_omitted_arguments_execute_with_static_storage() {
    for (body, calls, expected) in [
        (
            "BYTE counter=[0] counter==+1 RETURN(counter)",
            "output=Map(input) output=Map(input)",
            2,
        ),
        ("RETURN(value XOR $55)", "output=Map(input) output=Map()", 7),
    ] {
        let source = format!(
            "BYTE input=$600,output=$610 INLINE BYTE FUNC Map(BYTE value) {body} PROC Main() {calls} RETURN"
        );
        let (selected, new) = compile_scalar_image(&source, true);
        let (_, old) = compile_scalar_image(&source, false);
        assert_eq!(calls_to(&selected, RoutineId(0)), 2);
        // Omitted arguments retain the existing Action ABI capture behavior,
        // including register values left by the preceding call.
        assert_eq!(
            execute_scalar(&new, RoutineId(1), &[7]),
            execute_scalar(&old, RoutineId(1), &[7])
        );
        assert_eq!(execute_scalar(&new, RoutineId(1), &[7]).0[0], expected);
    }
}
