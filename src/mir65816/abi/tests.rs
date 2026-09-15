use super::*;
use crate::nir::{NirCallableKind, NirIntegerType, SignatureId};
use std::collections::BTreeMap;

fn ty(kind: NirTypeKind, width: u32) -> NirType {
    NirType {
        pointer: matches!(
            kind,
            NirTypeKind::Pointer { .. } | NirTypeKind::Callable { .. }
        ),
        kind,
        width: Some(ByteSize::new(width)),
        summary: "display text does not select the ABI".into(),
    }
}

fn scalar_types() -> Vec<NirType> {
    vec![
        ty(NirTypeKind::U8, 1),
        ty(NirTypeKind::I16, 2),
        ty(NirTypeKind::Integer(NirIntegerType::address(24)), 3),
        ty(
            NirTypeKind::Pointer {
                pointee: None,
                address_space: TargetLayout::DATA_ADDRESS_SPACE,
            },
            3,
        ),
        ty(
            NirTypeKind::Callable {
                kind: NirCallableKind::Proc,
                signature: SignatureId(1),
                convention: NirCallConvention::TargetPublic,
                address_space: TargetLayout::CODE_ADDRESS_SPACE,
            },
            3,
        ),
        ty(NirTypeKind::Integer(NirIntegerType::I32), 4),
    ]
}

fn signature(params: Vec<NirType>, result: Option<NirType>) -> NirCallableSignature {
    NirCallableSignature {
        params,
        result,
        ..NirCallableSignature::empty_proc(NirCallConvention::TargetPublic)
    }
}

#[test]
fn scalar_homes_retain_width_alignment_and_unused_result_bits() {
    let types = scalar_types();
    for (ty, size, alignment, result) in [
        (&types[0], 1, 1, ResultLocation::A8ZeroExtended),
        (&types[1], 2, 2, ResultLocation::A16),
        (&types[2], 3, 2, ResultLocation::A16X8ZeroExtended),
        (&types[3], 3, 1, ResultLocation::A16X8ZeroExtended),
        (&types[4], 3, 1, ResultLocation::A16X8ZeroExtended),
        (&types[5], 4, 2, ResultLocation::A16X16),
    ] {
        let actual = classify(ty).unwrap();
        assert_eq!(
            (actual.size.get(), actual.alignment.get(), actual.result),
            (size, alignment, result)
        );
        assert_eq!(
            call_layout(&signature(vec![], Some(ty.clone())))
                .unwrap()
                .result,
            Some(result)
        );
    }
    assert_eq!(
        classify(&ty(NirTypeKind::Bool, 1)).unwrap().class,
        ScalarClass::Byte
    );
    assert_eq!(
        classify(&ty(NirTypeKind::Integer(NirIntegerType::size(24)), 3))
            .unwrap()
            .class,
        ScalarClass::AddressOrSize
    );
    for integer in [NirIntegerType::U16, NirIntegerType::U32] {
        assert!(
            classify(&ty(
                NirTypeKind::Integer(integer),
                integer.storage_width().get()
            ))
            .is_ok()
        );
    }
}

#[test]
fn mixed_call_matches_the_published_bytes_and_stack_offsets() {
    let spec: serde_json::Value =
        serde_json::from_str(include_str!("../../../docs/abi/action65816-native-v1.json")).unwrap();
    let example = &spec["examples"]["mixed_call"];
    let types = scalar_types();
    let layout = call_layout(&signature(
        vec![
            types[0].clone(),
            types[1].clone(),
            types[3].clone(),
            types[5].clone(),
        ],
        None,
    ))
    .unwrap();
    assert_eq!(layout.payload_bytes.get(), 12);
    assert_eq!(layout.outgoing_bytes.get(), 13);
    assert_eq!(
        layout
            .arguments
            .iter()
            .map(|argument| argument.offset.get())
            .collect::<Vec<_>>(),
        vec![0, 2, 4, 8]
    );
    let mut bytes = vec![0; layout.outgoing_bytes.get() as usize];
    for (argument, value) in layout
        .arguments
        .iter()
        .zip(example["values_unsigned_bits"].as_array().unwrap())
    {
        let offset = argument.offset.get() as usize;
        let width = argument.scalar.size.get() as usize;
        bytes[offset..offset + width]
            .copy_from_slice(&value.as_u64().unwrap().to_le_bytes()[..width]);
    }
    let hex = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(hex, example["argument_image_hex"].as_str().unwrap());
    let s_call =
        example["caller_body_s"].as_u64().unwrap() - u64::from(layout.outgoing_bytes.get());
    let s_entry = s_call - u64::from(CALL_RETURN_ADDRESS_BYTES);
    assert_eq!(s_entry, example["s_entry"].as_u64().unwrap());
    assert_eq!(s_entry % 2, 0);
}

#[test]
fn every_short_scalar_signature_preserves_stack_and_argument_alignment() {
    let types = scalar_types();
    for count in 0..=4u32 {
        for mut variant in 0..types.len().pow(count) {
            let mut params = Vec::new();
            for _ in 0..count {
                params.push(types[variant % types.len()].clone());
                variant /= types.len();
            }
            let layout = call_layout(&signature(params, None)).unwrap();
            assert_eq!(layout.outgoing_bytes.get() % 2, 1);
            let s_call = 0x4000 - layout.outgoing_bytes.get();
            assert_eq!((s_call - CALL_RETURN_ADDRESS_BYTES) % 2, 0);
            for argument in &layout.arguments {
                assert_eq!(
                    (s_call + 1 + argument.offset.get()) % argument.scalar.alignment.get(),
                    0
                );
            }
        }
    }
    let empty = call_layout(&signature(vec![], None)).unwrap();
    assert_eq!(empty.payload_bytes, ByteSize::ZERO);
    assert_eq!(empty.outgoing_bytes, ByteSize::ONE);
}

#[test]
fn unsupported_and_inconsistent_physical_signatures_fail_explicitly() {
    assert_eq!(
        classify(&ty(NirTypeKind::Real, 6)),
        Err(AbiError::UnsupportedType)
    );
    assert_eq!(
        classify(&ty(NirTypeKind::Integer(NirIntegerType::I8), 1)),
        Err(AbiError::UnsupportedType)
    );
    assert!(matches!(
        classify(&ty(NirTypeKind::U16, 1)),
        Err(AbiError::WidthMismatch { .. })
    ));
    let mut sig = signature(vec![], None);
    sig.variadic = Some(ty(NirTypeKind::U8, 1));
    assert_eq!(call_layout(&sig), Err(AbiError::VariadicSignature));
    sig.variadic = None;
    sig.convention = NirCallConvention::External(crate::nir::ExternalAbiId(0));
    assert_eq!(call_layout(&sig), Err(AbiError::ExternalConvention));
    assert_eq!(
        call_layout(&signature(vec![scalar_types()[5].clone(); 16384], None)),
        Err(AbiError::ExtentOverflow)
    );
}

#[test]
fn generated_rust_and_assembly_constants_match_the_manifest_on_lf_and_crlf() {
    let source = include_str!("../../../docs/abi/action65816-native-v1.json").replace("\r\n", "\n");
    let assembly =
        include_str!("../../../docs/abi/action65816-native-v1.inc").replace("\r\n", "\n");
    for (source, assembly) in [
        (source.clone(), assembly.clone()),
        (source.replace('\n', "\r\n"), assembly.replace('\n', "\r\n")),
    ] {
        let spec: serde_json::Value = serde_json::from_str(&source).unwrap();
        let equates = assembly
            .lines()
            .filter_map(|line| line.split_once(" = $"))
            .map(|(name, value)| (name, u32::from_str_radix(value, 16).unwrap()))
            .collect::<BTreeMap<_, _>>();
        for &(path, name, value) in generated::MANIFEST_VALUES {
            assert_eq!(
                spec.pointer(path).unwrap().as_u64(),
                Some(u64::from(value)),
                "{path}"
            );
            assert_eq!(equates.get(name), Some(&value), "{name}");
        }
        assert_eq!(equates.len(), generated::MANIFEST_VALUES.len());
        let fingerprint = source
            .replace("\r\n", "\n")
            .bytes()
            .fold(0xcbf29ce484222325u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
            });
        assert_eq!(
            fingerprint, MANIFEST_FINGERPRINT,
            "run python3 tools/generate_abi65816.py"
        );
        assert_eq!(spec["abi"], ABI_NAME);
    }
}
