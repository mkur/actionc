use super::*;

fn source(expression: &str) -> String {
    format!(
        "INT result=$0600 BYTE done=$0602,calls=$0603 CARD trace=$0604 \
         BYTE count=$0610 BYTE ARRAY table(4)=[0 1 2 3] \
         BYTE FUNC F(BYTE n) calls==+1 trace=trace*17+n RETURN(n) \
         CARD FUNC W(CARD n) calls==+1 trace=trace*17+n RETURN(n) \
         PROC Main() calls=0 trace=0 result={expression} done=$A5 RETURN"
    )
}

fn execute(output: &CodegenOutput) -> [u8; 65536] {
    let mut memory = [0u8; 65536];
    if output.map.runtime == Runtime::ActionCart {
        let cart = include_bytes!("../../../roms/action.rom");
        memory[0xA000..0xB000].copy_from_slice(&cart[0x1010..0x2010]);
        memory[0xB000..0xC000].copy_from_slice(&cart[0x10..0x1010]);
    }
    let origin = usize::from(output.origin);
    memory[origin..origin + output.bytes.len()].copy_from_slice(&output.bytes);
    memory[0x11] = 0xFF;
    memory[0x610] = 1;
    indexed_test_cpu::run_memory(&mut memory, usize::from(output.run_address));
    memory
}

#[test]
fn compatible_arithmetic_calls_consume_results_in_source_tree_order() {
    // Action! 3.6 ROM probes establish acceptance; results and call traces
    // below are independent arithmetic/evaluation-order expectations.
    for (expression, expected, calls, trace) in [
        ("(F(2)*2-1)*(F(3)+1)", 12i16, 2, 37u16),
        ("(F(0)*2-1)*(F(2)+1)", -3, 2, 2),
        ("(F(2)+1)+(F(3)+1)", 7, 2, 37),
        ("(F(2)+1)-(F(3)+1)", 255, 2, 37),
        ("(F(2)+1)&(F(3)+1)", 0, 2, 37),
        ("(F(2)+1)%(F(3)+1)", 7, 2, 37),
        ("(F(12)+0)/(F(3)+0)", 4, 2, 207),
        ("(F(7)+0) MOD (F(3)+0)", 1, 2, 122),
        ("(F(2)+1)+F(3)", 6, 2, 37),
        ("F(2)+1+F(3)", 6, 2, 37),
        ("F(2)+0+F(3)", 5, 2, 37),
        ("(0+F(2))+F(3)", 5, 2, 37),
        ("(F(2)-0)+F(3)", 5, 2, 37),
        ("F(2)*1+F(3)", 5, 2, 37),
        ("(F(2)+1)*F(3)", 9, 2, 37),
        ("(-F(2))+F(3)", 1, 2, 37),
        ("(F(2)&1)+F(3)", 3, 2, 37),
        ("(F(2) LSH 1)+F(3)", 7, 2, 37),
        ("(W(256) LSH 0)+W(3)", 259, 2, 4355),
        ("(W(256) RSH 0)+W(3)", 259, 2, 4355),
        ("7-F(2)", 5, 1, 2),
        ("F(12)/3", 4, 1, 12),
        ("12/F(3)", 4, 1, 3),
        ("F(7) MOD 3", 1, 1, 7),
        ("7 MOD F(3)", 1, 1, 3),
        ("8 RSH F(2)", 2, 1, 2),
        ("F(8) RSH count", 4, 1, 8),
        ("table(2)+F(3)", 5, 1, 3),
    ] {
        let source = source(expression);
        let ast = parse(&tokenize(&source).unwrap()).unwrap();
        let model = analyze(&ast).unwrap();
        let semir = crate::semantic::ir::lower_program(&ast, &model);
        let profile = CodegenProfile::Compat;
        let outputs = [
            generate_semir_profile_at_origin(&semir, 0x3000, profile)
                .unwrap_or_else(|errors| panic!("{expression}/cart: {errors:?}")),
            generate_semir_standalone_profile_at_origin(&semir, 0x3000, profile)
                .unwrap_or_else(|errors| panic!("{expression}/standalone: {errors:?}")),
            generate_profile_at_origin(&ast, 0x3000, profile)
                .unwrap_or_else(|errors| panic!("{expression}/ast: {errors:?}")),
        ];
        for output in outputs {
            let memory = execute(&output);
            assert_eq!(
                &memory[0x600..0x606],
                &[
                    expected as u8,
                    (expected >> 8) as u8,
                    0xA5,
                    calls,
                    trace as u8,
                    (trace >> 8) as u8
                ],
                "{expression}/{profile:?}/{:?}",
                output.map.runtime
            );
        }
    }
}

#[test]
fn compatible_arithmetic_calls_reject_a_call_before_consuming_prior_result() {
    for expression in [
        "F(2)+F(3)",
        "W(2)+W(3)",
        "F(2)*F(3)",
        "(F(2))+(F(3))",
        "F(2)*(F(3)+1)",
        "F(2)+(F(3)+1)",
        "F(2)+1*F(3)",
        "F(2)+(-F(3))",
        "F(2)&F(3)",
        "F(2)%F(3)",
        "(F(2) LSH 0)+F(3)",
        "(F(2) RSH 0)+F(3)",
        "F(2)+table(F(3))",
    ] {
        let source = source(expression);
        assert_rejected(&source, "earlier function result is still pending");
    }
    for expression in ["F(F(2))", "F(F(2)+1)"] {
        let source = source(expression);
        assert_rejected(&source, "function calls as routine call arguments");
    }
}

fn assert_rejected(source: &str, message: &str) {
    assert_compatible_diagnostic_contains(source, message);
    let ast = parse(&tokenize(source).unwrap()).unwrap();
    let model = analyze(&ast).unwrap();
    let semir = crate::semantic::ir::lower_program(&ast, &model);
    for result in [
        generate_semir_profile_at_origin(&semir, 0x3000, CodegenProfile::Compat),
        generate_semir_standalone_profile_at_origin(&semir, 0x3000, CodegenProfile::Compat),
    ] {
        let errors = result.expect_err("compatibility must reject overlapping calls");
        assert!(
            errors.iter().any(|error| error.message.contains(message)),
            "{source}: {errors:?}"
        );
    }
    generate_semir_profile_at_origin(&semir, 0x3000, CodegenProfile::Modern)
        .unwrap_or_else(|errors| panic!("modern profile should still accept {source}: {errors:?}"));
}
