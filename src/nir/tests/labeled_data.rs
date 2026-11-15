use super::*;

#[test]
fn labeled_data_is_typed_mutable_storage_with_forward_relocations() {
    let source = "DATA first=[\"Yes\" 'Y $1234 <@second >@second @first+1 *]\nDATA second=[$00]\nPROC Main() BYTE POINTER p p=@first RETURN";
    for source in [source.to_owned(), source.replace('\n', "\r\n")] {
        let n = lower_modern_source(&source);
        verify_program(&n).unwrap();
        assert_eq!(n.routines.len(), 1);
        assert!(n.routines[0].entry.program);
        assert_eq!(n.statics.len(), 2);
        let first = &n.statics[0];
        let second = &n.statics[1];
        assert!(first.mutable);
        assert_eq!(first.section, "data");
        assert_eq!(
            first
                .image
                .project_constants(crate::target::Endian::Little)
                .unwrap(),
            [3, b'Y', b'e', b's', b'Y', 0x34, 0x12, 0, 0, 0, 0, 0, 0]
        );
        assert!(first.image.fragments.iter().any(|f| matches!(f,
            NirDataFragment::Address { target: NirDataAddressTarget::Static(id), .. } if *id == second.id)));
        assert!(first.image.fragments.iter().any(|f| matches!(f,
            NirDataFragment::Address { target: NirDataAddressTarget::Static(id), addend: 11, .. } if *id == first.id)));
        assert!(!format_program(&n).contains("foreign"));
        let mut bad = n.clone();
        for fragment in &mut bad.statics[0].image.fragments {
            if let NirDataFragment::Address { target, .. } = fragment {
                *target = NirDataAddressTarget::Static(SymbolId(999));
                break;
            }
        }
        assert!(
            verify_program(&bad)
                .unwrap_err()
                .iter()
                .any(|d| d.message.contains("unknown static id"))
        );
        optimize_program(&n).unwrap();
    }
}

#[test]
fn labeled_data_preserves_contextual_identifier_and_rejects_value_use() {
    verify_program(&lower_modern_source("BYTE DATA PROC Main() DATA=1 RETURN")).unwrap();
    for source in [
        "DATA x=[0] PROC Main() x() RETURN",
        "DATA x=[0] PROC Main() x=1 RETURN",
        "DATA x=[0] PROC Main() BYTE y y=x RETURN",
        "DATA x=[0] DATA x=[1] PROC Main() RETURN",
        "DATA x=[missing] PROC Main() RETURN",
        "DATA x=[$10000] PROC Main() RETURN",
    ] {
        let ast = parse(&tokenize(source).unwrap()).unwrap();
        assert!(
            analyze_with_options(&ast, SemanticOptions::modern()).is_err(),
            "{source}"
        );
    }
    assert!(parse(&tokenize("DATA x=generator() PROC Main() RETURN").unwrap()).is_err());
    assert!(parse(&tokenize("PROC Main() IF 1 THEN DATA x=[0] FI RETURN").unwrap()).is_err());
}

#[test]
fn labeled_data_static_targets_survive_array_initializers_and_raw_code() {
    let n = lower_modern_source(
        "DATA bytes=[1 2] BYTE ARRAY refs(2)=[<@bytes >@bytes] PROC Main() [<@bytes >@bytes] RETURN",
    );
    verify_program(&n).unwrap();
    assert!(
        matches!(&n.globals.iter().find(|g| g.name=="refs").unwrap().init,
        Some(NirGlobalInit::Bytes { image, .. }) if image.fragments.iter().all(|f| matches!(f,
            NirDataFragment::Address { target: NirDataAddressTarget::Static(_), .. })))
    );
}

#[test]
fn labeled_data_uses_target_pointer_width_and_literal_endian() {
    for target in [
        crate::target::TargetId::Atari6502,
        crate::target::TargetId::Wdc65816Native,
        crate::target::TargetId::Motorola68000,
    ] {
        let n =
            lower_modern_source_for_target("DATA bytes=[$1234 @bytes] PROC Main() RETURN", target);
        verify_program(&n).unwrap();
        let width = n.target_layout.data_pointer.size_bytes.get();
        assert_eq!(n.statics[0].image.bytes.len(), 2 + width as usize);
        match target {
            crate::target::TargetId::Wdc65816Native => {
                crate::mir65816::lower_program(&n).unwrap();
            }
            crate::target::TargetId::Motorola68000 => {
                crate::mir68k::lower_program(&n).unwrap();
            }
            _ => {}
        }
    }
}

#[test]
fn labeled_data_named_modules_retain_relocation_dependencies() {
    let root = SourceOrigin::host("project/main.act");
    let provider = InMemorySourceProvider::default()
        .with_source(root.clone(), b"MODULE Main USE Tables PROC Run() BYTE POINTER p p=@Tables.first RETURN ENDMODULE".to_vec())
        .with_source(SourceOrigin::host("project/tables.act"), b"MODULE Tables PUBLIC DATA first=[@second @Later @cell] DATA second=[@first] BYTE cell PROC Later() RETURN ENDMODULE".to_vec());
    let loaded =
        load_compilation_from_provider(root, &provider, &ModuleLoadOptions::default()).unwrap();
    let model =
        crate::semantic::analyze_compilation_with_options(&loaded, SemanticOptions::modern())
            .unwrap();
    let sem = crate::semantic::ir::lower_compilation(&loaded, &model);
    let selected =
        crate::linker::select_semir(&sem, crate::linker::SemLinkPolicy::EntryReachable).unwrap();
    let n = lower_program(&selected);
    verify_program(&n).unwrap();
    assert_eq!(n.statics.len(), 2);
    assert_eq!(n.routines.len(), 2);
    let image = &n.statics[0].image;
    assert!(image.fragments.iter().any(|f| matches!(
        f,
        NirDataFragment::Address {
            target: NirDataAddressTarget::Routine(_),
            ..
        }
    )));
    assert!(image.fragments.iter().any(|f| matches!(
        f,
        NirDataFragment::Address {
            target: NirDataAddressTarget::Storage(NirStorageId::Global(_)),
            ..
        }
    )));
}

#[test]
fn labeled_data_resolves_explicit_numeric_and_resident_addresses() {
    let n =
        lower_modern_source("DATA addresses=[@1 <@COLOR >@COLOR] DATA empty=[] PROC Main() RETURN");
    verify_program(&n).unwrap();
    assert_eq!(n.statics[0].image.bytes.len(), 4);
    assert_eq!(n.statics[0].image.fragments.len(), 3);
    assert!(n.statics[1].image.bytes.is_empty());
    let ast = parse(&tokenize("DATA x=[0] PROC Main() RETURN").unwrap()).unwrap();
    assert!(
        crate::codegen::generate(&ast)
            .unwrap_err()
            .iter()
            .any(|d| d.message.contains("DATA requires a NIR backend"))
    );
}
