use super::*;
use crate::mir6502::ir::MirDirectResultAbi;

fn nir(source: &str) -> NirProgram {
    let ast = crate::parser::parse(&crate::lexer::tokenize(source).unwrap()).unwrap();
    let model =
        crate::semantic::analyze_with_options(&ast, crate::semantic::SemanticOptions::modern())
            .unwrap();
    nir::lower_program(&crate::semantic::ir::lower_program(&ast, &model))
}

#[test]
fn direct_results_survive_pointer_parameters_materialization_and_printing() {
    let n = nir("BYTE FUNC ByteInt(CARD p) RETURN(7)
        BYTE FUNC BytePtr(BYTE POINTER p) RETURN(7)
        CARD FUNC WordPtr(BYTE POINTER p) RETURN(256)
        BYTE POINTER FUNC Address(BYTE POINTER p) RETURN(p)
        PROC Nothing(BYTE POINTER p) RETURN
        PROC Main() RETURN");
    let p = crate::mir6502::lower_program(&n).unwrap();
    let materialized =
        crate::mir6502::materialize_program(p.clone(), &crate::mir6502::Mir6502Config::default())
            .unwrap();
    for (name, contract, printed, slots) in [
        (
            "ByteInt",
            MirDirectResultAbi::Scalar(MirWidth::Byte),
            "byte @ $A0",
            &[0xa0][..],
        ),
        (
            "BytePtr",
            MirDirectResultAbi::Scalar(MirWidth::Byte),
            "byte @ $A0",
            &[0xa0][..],
        ),
        (
            "WordPtr",
            MirDirectResultAbi::Scalar(MirWidth::Word),
            "word @ $A0/$A1",
            &[0xa0, 0xa1][..],
        ),
        (
            "Address",
            MirDirectResultAbi::Pointer,
            "pointer @ $A0/$A1",
            &[0xa0, 0xa1][..],
        ),
        ("Nothing", MirDirectResultAbi::Void, "void", &[][..]),
    ] {
        for program in [&p, &materialized] {
            let r = program.routines.iter().find(|r| r.name == name).unwrap();
            assert_eq!(r.direct_result_abi, Some(contract));
            assert_eq!(r.scalar_signature.is_some(), name == "ByteInt");
            assert!(
                crate::mir6502::printer::format_routine(r)
                    .contains(&format!("result-abi {printed}\n"))
            );
            assert_eq!(crate::mir6502::abi::direct_result_slots(contract), slots);
            let mut unknown = r.clone();
            unknown.direct_result_abi = None;
            assert!(
                crate::mir6502::printer::format_routine(&unknown).contains("result-abi unknown\n")
            );
        }
    }
}

#[test]
fn direct_results_are_independent_of_unsupported_parameters_and_variadic_status() {
    let n = nir("BYTE FUNC Known(CARD p) RETURN(1)
        BYTE POINTER FUNC Address(BYTE POINTER p) RETURN(p)");
    let mut signature = n.routines[0].signature.clone();
    signature.params[0].kind = NirTypeKind::Integer(nir::NirIntegerType::U32);
    signature.params[0].width = Some(ByteSize::new(4));
    let expected = Some(MirDirectResultAbi::Scalar(MirWidth::Byte));
    assert_eq!(direct_result_abi(&signature), expected);
    assert!(direct_parameter_abi(&signature).is_none());
    signature.variadic = Some(signature.params[0].clone());
    assert_eq!(direct_result_abi(&signature), expected);
    assert!(direct_parameter_abi(&signature).is_none());
    signature.result = None;
    assert_eq!(
        direct_result_abi(&signature),
        Some(MirDirectResultAbi::Void)
    );

    let mut pointer = n.routines[1].signature.clone();
    assert_eq!(
        direct_result_abi(&pointer),
        Some(MirDirectResultAbi::Pointer)
    );
    pointer.result.as_mut().unwrap().width = Some(ByteSize::ONE);
    assert_eq!(direct_result_abi(&pointer), None);
    pointer = n.routines[1].signature.clone();
    let NirTypeKind::Pointer { address_space, .. } = &mut pointer.result.as_mut().unwrap().kind
    else {
        unreachable!()
    };
    *address_space = crate::target::AddressSpaceId(77);
    assert_eq!(direct_result_abi(&pointer), None);

    signature.variadic = None;
    signature.result = Some(signature.params[0].clone()); // LONG
    assert_eq!(direct_result_abi(&signature), None);
    // Even an inconsistent narrow width cannot reinterpret LONG as a word.
    signature.result.as_mut().unwrap().width = Some(ByteSize::new(2));
    assert_eq!(direct_result_abi(&signature), None);
    signature.result.as_mut().unwrap().kind = NirTypeKind::Record {
        definition: None,
        name: "Pair".into(),
        size: Some(ByteSize::new(2)),
    };
    assert_eq!(direct_result_abi(&signature), None);
    signature.result.as_mut().unwrap().kind = NirTypeKind::Real;
    signature.result.as_mut().unwrap().width = Some(ByteSize::new(6));
    assert_eq!(direct_result_abi(&signature), None);
}
