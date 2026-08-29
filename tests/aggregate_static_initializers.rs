use actionc::ast::{Decl, ExprKind, Item};
use actionc::codegen::{
    CodegenOutput, CodegenProfile, CodegenSymbolScope, generate_profile_with_origin,
    generate_semir_profile_with_origin,
};
use actionc::compiler::{CompileMode, CompileOptions, compile_file};
use actionc::lexer::tokenize;
use actionc::mir6502::{self, Mir6502Config};
use actionc::nir;
use actionc::parser::parse;
use actionc::semantic::ir::{self, SemItem, SemStaticInitializerValue};
use actionc::semantic::{SemanticModel, analyze};

const ORIGIN: u16 = 0x3000;

fn parse_and_analyze(source: &str) -> (actionc::ast::Program, SemanticModel) {
    let program = parse(&tokenize(source).expect("tokenize aggregate initializer fixture"))
        .expect("parse aggregate initializer fixture");
    let model = analyze(&program).expect("analyze aggregate initializer fixture");
    (program, model)
}

fn classic_output(source: &str) -> CodegenOutput {
    let (program, _) = parse_and_analyze(source);
    generate_profile_with_origin(&program, ORIGIN, CodegenProfile::Compat)
        .expect("generate aggregate initializer characterization fixture")
}

fn global_address(output: &CodegenOutput, name: &str) -> u16 {
    output
        .map
        .storage_symbols
        .iter()
        .find(|symbol| {
            symbol.scope == CodegenSymbolScope::Global && symbol.name.eq_ignore_ascii_case(name)
        })
        .unwrap_or_else(|| panic!("missing global storage symbol `{name}`"))
        .address
}

fn storage_address(output: &CodegenOutput, name: &str) -> u16 {
    output
        .map
        .storage_symbols
        .iter()
        .find(|symbol| symbol.name.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| panic!("missing storage symbol `{name}`"))
        .address
}

fn output_bytes(output: &CodegenOutput, address: u16, len: usize) -> &[u8] {
    let offset = usize::from(address.wrapping_sub(output.origin));
    &output.bytes[offset..offset + len]
}

fn record_array_backing<'a>(
    output: &'a CodegenOutput,
    name: &str,
    byte_len: usize,
) -> &'a [u8] {
    let descriptor = global_address(output, name);
    let pointer = u16::from_le_bytes(
        output_bytes(output, descriptor, 2)
            .try_into()
            .expect("record-array descriptor pointer"),
    );
    output_bytes(output, pointer, byte_len)
}

fn storage_record_array_backing<'a>(
    output: &'a CodegenOutput,
    name: &str,
    byte_len: usize,
) -> &'a [u8] {
    let descriptor = storage_address(output, name);
    let pointer = u16::from_le_bytes(
        output_bytes(output, descriptor, 2)
            .try_into()
            .expect("record-array descriptor pointer"),
    );
    output_bytes(output, pointer, byte_len)
}

fn all_backend_outputs(source: &str) -> [(String, CodegenOutput); 3] {
    let (program, model) = parse_and_analyze(source);
    let semir = ir::lower_program(&program, &model);
    let compatibility =
        generate_semir_profile_with_origin(&semir, ORIGIN, CodegenProfile::Compat)
            .expect("compatibility/classic aggregate initializer");
    let modern = generate_semir_profile_with_origin(&semir, ORIGIN, CodegenProfile::Modern)
        .expect("modern/classic aggregate initializer");
    let nir = nir::lower_program(&semir);
    let nir = nir::optimize_program(&nir).expect("verify aggregate initializer NIR");
    let mir6502 =
        mir6502::generate_output_with_config(&nir, ORIGIN, &Mir6502Config::optimized())
            .expect("MIR6502 aggregate initializer");
    [
        ("compatibility/classic".to_string(), compatibility),
        ("modern/classic".to_string(), modern),
        ("MIR6502".to_string(), mir6502),
    ]
}

#[test]
fn flat_initializer_list_preserves_one_element_per_source_value() {
    let (program, _) = parse_and_analyze(
        "BYTE ARRAY data(6)=[1 -2 'A TRUE FALSE NIL] PROC Main() RETURN",
    );
    let Item::Declaration(Decl::Var(declaration)) = &program.modules[0].items[0] else {
        panic!("expected array declaration");
    };
    let ExprKind::InitializerList(elements) =
        &declaration.entries[0].initializer.as_ref().unwrap().kind
    else {
        panic!("expected structured initializer list");
    };

    assert_eq!(elements.len(), 6);
}

#[test]
fn semantic_record_layout_is_packed_in_recursive_declaration_order() {
    let (_, model) = parse_and_analyze(
        "TYPE Inner=[BYTE flag CARD address] \
         TYPE Outer=[BYTE tag Inner value BYTE tail] \
         Outer ARRAY table(2) PROC Main() RETURN",
    );
    let inner = model
        .layout
        .records
        .iter()
        .find(|record| record.name.eq_ignore_ascii_case("Inner"))
        .expect("Inner semantic layout");
    let outer = model
        .layout
        .records
        .iter()
        .find(|record| record.name.eq_ignore_ascii_case("Outer"))
        .expect("Outer semantic layout");

    assert_eq!(inner.size, 3);
    assert_eq!(
        inner
            .fields
            .iter()
            .map(|field| (field.name.as_str(), field.offset))
            .collect::<Vec<_>>(),
        [("flag", 0), ("address", 1)]
    );
    assert_eq!(outer.size, 5);
    assert_eq!(
        outer
            .fields
            .iter()
            .map(|field| (field.name.as_str(), field.offset))
            .collect::<Vec<_>>(),
        [("tag", 0), ("value", 1), ("tail", 4)]
    );
}

#[test]
fn sized_byte_array_zero_fills_after_the_initialized_extent() {
    let output = classic_output("BYTE ARRAY values(4)=[1 2] PROC Main() RETURN");
    let address = global_address(&output, "values");

    assert_eq!(output_bytes(&output, address, 4), [1, 2, 0, 0]);
}

#[test]
fn initializer_longer_than_declared_byte_array_currently_extends_storage() {
    let output = classic_output(
        "BYTE ARRAY values(2)=[1 2 3] BYTE ARRAY sentinel(1)=[$AA] PROC Main() RETURN",
    );
    let values = global_address(&output, "values");
    let sentinel = global_address(&output, "sentinel");

    assert_eq!(output_bytes(&output, values, 3), [1, 2, 3]);
    assert_eq!(sentinel, values + 3);
}

#[test]
fn sized_card_array_zero_fills_its_descriptor_backing() {
    let output = classic_output("CARD ARRAY values(3)=[1] PROC Main() RETURN");
    let descriptor = global_address(&output, "values");
    let pointer = u16::from_le_bytes(
        output_bytes(&output, descriptor, 2)
            .try_into()
            .expect("descriptor pointer bytes"),
    );

    assert_eq!(output_bytes(&output, pointer, 6), [1, 0, 0, 0, 0, 0]);
}

#[test]
fn semir_plans_mixed_width_record_array_writes_at_leaf_offsets() {
    let (program, model) = parse_and_analyze(
        "TYPE Pair=[BYTE tag CARD word] \
         Pair ARRAY pairs(2)=[1 $2345 2 $6789] PROC Main() RETURN",
    );
    let semir = ir::lower_program(&program, &model);
    let declaration = semir.modules[0]
        .items
        .iter()
        .find_map(|item| match item {
            SemItem::Declaration(declaration)
                if declaration.symbol.name.eq_ignore_ascii_case("pairs") =>
            {
                Some(declaration)
            }
            _ => None,
        })
        .expect("pairs SemIR declaration");
    let plan = declaration
        .static_initializer
        .as_ref()
        .expect("layout-resolved static initializer");

    assert_eq!(plan.initialized_extent, 6);
    assert_eq!(
        plan.writes
            .iter()
            .map(|write| (write.offset, write.width, write.display_path.as_str()))
            .collect::<Vec<_>>(),
        [
            (0, 1, "pairs(0).tag"),
            (1, 2, "pairs(0).word"),
            (3, 1, "pairs(1).tag"),
            (4, 2, "pairs(1).word"),
        ]
    );
}

#[test]
fn semir_rounds_partial_inferred_record_array_to_a_complete_element() {
    let (program, model) = parse_and_analyze(
        "TYPE Pair=[BYTE tag CARD word] \
         Pair ARRAY pairs=[1 $2345 2] PROC Main() RETURN",
    );
    let semir = ir::lower_program(&program, &model);
    let plan = semir.modules[0]
        .items
        .iter()
        .find_map(|item| match item {
            SemItem::Declaration(declaration)
                if declaration.symbol.name.eq_ignore_ascii_case("pairs") =>
            {
                declaration.static_initializer.as_ref()
            }
            _ => None,
        })
        .expect("pairs static initializer plan");

    assert_eq!(plan.initialized_extent, 6);
    assert_eq!(
        plan.writes.iter().map(|write| write.offset).collect::<Vec<_>>(),
        [0, 1, 3]
    );
}

#[test]
fn semantic_address_width_checks_use_the_record_leaf_destination() {
    let (program, model) = parse_and_analyze(
        "TYPE Pair=[BYTE low CARD word] BYTE target \
         Pair ARRAY pairs(1)=[<target @target] PROC Main() RETURN",
    );
    let semir = ir::lower_program(&program, &model);
    let plan = semir.modules[0]
        .items
        .iter()
        .find_map(|item| match item {
            SemItem::Declaration(declaration)
                if declaration.symbol.name.eq_ignore_ascii_case("pairs") =>
            {
                declaration.static_initializer.as_ref()
            }
            _ => None,
        })
        .expect("pairs relocation plan");

    assert!(matches!(
        plan.writes[0].value,
        SemStaticInitializerValue::Address {
            selector: Some(_),
            ..
        }
    ));
    assert!(matches!(
        plan.writes[1].value,
        SemStaticInitializerValue::Address { selector: None, .. }
    ));
    assert_eq!(
        plan.writes.iter().map(|write| write.width).collect::<Vec<_>>(),
        [1, 2]
    );
}

#[test]
fn global_mixed_width_record_array_has_identical_backing_in_all_backends() {
    let source = "TYPE Pair=[BYTE tag CARD word] \
                  Pair ARRAY pairs(2)=[1 $2345 2 $6789] \
                  PROC Main() RETURN";
    let (program, model) = parse_and_analyze(source);
    let semir = ir::lower_program(&program, &model);
    let compatibility =
        generate_semir_profile_with_origin(&semir, ORIGIN, CodegenProfile::Compat)
            .expect("compatibility/classic aggregate initializer");
    let modern = generate_semir_profile_with_origin(&semir, ORIGIN, CodegenProfile::Modern)
        .expect("modern/classic aggregate initializer");
    let nir = nir::lower_program(&semir);
    let nir = nir::optimize_program(&nir).expect("verify aggregate initializer NIR");
    let mir6502 = mir6502::generate_output_with_config(
        &nir,
        ORIGIN,
        &Mir6502Config::optimized(),
    )
    .expect("MIR6502 aggregate initializer");

    for (backend, output) in [
        ("compatibility/classic", compatibility),
        ("modern/classic", modern),
        ("MIR6502", mir6502),
    ] {
        assert_eq!(
            record_array_backing(&output, "pairs", 6),
            [1, 0x45, 0x23, 2, 0x89, 0x67],
            "{backend} record backing"
        );
    }
}

#[test]
fn compiler_api_accepts_global_record_arrays_in_every_mode() {
    let source = "TYPE Pair=[BYTE tag CARD word] \
                  Pair ARRAY pairs(2)=[1 $2345 2 $6789] \
                  PROC Main() RETURN";
    let path = std::env::temp_dir().join(format!(
        "actionc-aggregate-initializer-{}.act",
        std::process::id()
    ));
    std::fs::write(&path, source).expect("write aggregate initializer compiler fixture");

    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        compile_file(&path, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|error| panic!("compile aggregate initializer in {mode:?}: {error}"));
    }

    std::fs::remove_file(path).expect("remove aggregate initializer compiler fixture");
}

#[test]
fn compiler_api_reads_fields_from_initialized_record_arrays_in_every_mode() {
    let source = "TYPE Pair=[BYTE tag CARD word] \
                  Pair ARRAY pairs(2)=[1 $2345 2 $6789] \
                  BYTE tagOut CARD wordOut \
                  PROC Main() tagOut=pairs(1).tag wordOut=pairs(1).word RETURN";
    let path = std::env::temp_dir().join(format!(
        "actionc-aggregate-field-read-{}.act",
        std::process::id()
    ));
    std::fs::write(&path, source).expect("write aggregate field-read compiler fixture");

    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        compile_file(&path, &CompileOptions::for_mode(mode))
            .unwrap_or_else(|error| panic!("compile record field reads in {mode:?}: {error:?}"));
    }

    std::fs::remove_file(path).expect("remove aggregate field-read compiler fixture");
}

#[test]
fn compiler_api_reads_nested_initialized_record_fields_in_every_mode() {
    let source = "TYPE Inner=[BYTE flag CARD address] \
                  TYPE Outer=[BYTE tag Inner value BYTE tail] \
                  Outer ARRAY table(2)=[1 2 $3456 3 4 5 $789A 6] \
                  BYTE flagOut,tailOut CARD addressOut \
                  PROC Main() \
                    flagOut=table(1).value.flag \
                    addressOut=table(1).value.address \
                    tailOut=table(1).tail \
                  RETURN";
    let path = std::env::temp_dir().join(format!(
        "actionc-nested-aggregate-field-read-{}.act",
        std::process::id()
    ));
    std::fs::write(&path, source).expect("write nested aggregate field-read compiler fixture");

    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        compile_file(&path, &CompileOptions::for_mode(mode)).unwrap_or_else(|error| {
            panic!("compile nested record field reads in {mode:?}: {error:?}")
        });
    }

    std::fs::remove_file(path).expect("remove nested aggregate field-read compiler fixture");
}

#[test]
fn direct_record_initializer_has_identical_bytes_in_all_backends() {
    let source = "TYPE Pair=[BYTE tag CARD word] \
                  Pair value=[1 $2345] PROC Main() RETURN";

    for (backend, output) in all_backend_outputs(source) {
        let address = global_address(&output, "value");
        assert_eq!(
            output_bytes(&output, address, 3),
            [1, 0x45, 0x23],
            "{backend} direct record"
        );
    }
}

#[test]
fn local_record_array_has_identical_backing_in_all_backends() {
    let source = "TYPE Pair=[BYTE tag CARD word] \
                  PROC Main() Pair ARRAY pairs(2)=[1 $2345 2 $6789] RETURN";

    for (backend, output) in all_backend_outputs(source) {
        assert_eq!(
            storage_record_array_backing(&output, "pairs", 6),
            [1, 0x45, 0x23, 2, 0x89, 0x67],
            "{backend} local record-array backing"
        );
    }
}

#[test]
fn local_direct_record_has_identical_bytes_in_all_backends() {
    let source = "TYPE Pair=[BYTE tag CARD word] \
                  PROC Main() Pair value=[1 $2345] RETURN";

    for (backend, output) in all_backend_outputs(source) {
        let address = storage_address(&output, "value");
        assert_eq!(
            output_bytes(&output, address, 3),
            [1, 0x45, 0x23],
            "{backend} local direct record"
        );
    }
}

#[test]
fn nested_record_array_uses_recursive_packed_layout_in_all_backends() {
    let source = "TYPE Inner=[BYTE flag CARD address] \
                  TYPE Outer=[BYTE tag Inner value BYTE tail] \
                  Outer ARRAY table(2)=[1 2 $3456 3 4 5 $789A 6] \
                  PROC Main() RETURN";
    let expected = [1, 2, 0x56, 0x34, 3, 4, 5, 0x9A, 0x78, 6];

    for (backend, output) in all_backend_outputs(source) {
        assert_eq!(
            record_array_backing(&output, "table", expected.len()),
            expected,
            "{backend} nested record-array backing"
        );
    }
}

#[test]
fn twelve_byte_record_array_uses_leaf_widths_in_all_backends() {
    let source = "TYPE Wide=[CARD a,b,c,d,e,f] \
                  Wide ARRAY table(1)=[$0102 $0304 $0506 $0708 $090A $0B0C] \
                  PROC Main() RETURN";
    let expected = [
        0x02, 0x01, 0x04, 0x03, 0x06, 0x05, 0x08, 0x07, 0x0A, 0x09, 0x0C, 0x0B,
    ];

    for (backend, output) in all_backend_outputs(source) {
        assert_eq!(
            record_array_backing(&output, "table", expected.len()),
            expected,
            "{backend} twelve-byte record-array backing"
        );
    }
}

#[test]
fn partial_inferred_record_zero_fills_the_final_element_in_all_backends() {
    let source = "TYPE Pair=[BYTE tag CARD word] \
                  Pair ARRAY pairs=[1 $2345 2] PROC Main() RETURN";

    for (backend, output) in all_backend_outputs(source) {
        assert_eq!(
            record_array_backing(&output, "pairs", 6),
            [1, 0x45, 0x23, 2, 0, 0],
            "{backend} partial inferred record"
        );
    }
}

#[test]
fn record_leaf_relocations_resolve_forward_targets_in_all_backends() {
    let source = "TYPE Links=[BYTE low BYTE high CARD full] \
                  Links ARRAY refs(1)=[<target >target @target] \
                  BYTE target=$AA PROC Main() RETURN";

    for (backend, output) in all_backend_outputs(source) {
        let target = global_address(&output, "target");
        assert_eq!(
            record_array_backing(&output, "refs", 4),
            [target as u8, (target >> 8) as u8, target as u8, (target >> 8) as u8],
            "{backend} record leaf relocations"
        );
    }
}

#[test]
fn record_leaf_relocations_resolve_the_own_descriptor_in_all_backends() {
    let source = "TYPE SelfRef=[CARD address] \
                  SelfRef ARRAY refs(1)=[@refs] PROC Main() RETURN";

    for (backend, output) in all_backend_outputs(source) {
        let descriptor = global_address(&output, "refs");
        let backing = u16::from_le_bytes(
            output_bytes(&output, descriptor, 2)
                .try_into()
                .expect("self-reference descriptor pointer"),
        );
        assert_eq!(
            record_array_backing(&output, "refs", 2),
            backing.to_le_bytes(),
            "{backend} self relocation"
        );
    }
}

#[test]
fn qualified_module_relocations_compile_in_every_mode() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock after Unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "actionc-qualified-aggregate-relocation-{}-{unique}",
        std::process::id()
    ));
    let library_dir = root.join("lib");
    std::fs::create_dir_all(&library_dir).expect("create aggregate module fixture directory");
    let application = root.join("app.act");
    std::fs::write(
        &application,
        "MODULE APP\n\
         USE LIB.TARGET AS DATA\n\
         TYPE Link=[CARD address]\n\
         Link ARRAY refs(1)=[@DATA.value]\n\
         PROC Main() RETURN\n\
         ENDMODULE\n",
    )
    .expect("write aggregate module application");
    std::fs::write(
        library_dir.join("target.act"),
        "MODULE LIB.TARGET\nPUBLIC BYTE value=$AA\nENDMODULE\n",
    )
    .expect("write aggregate module library");

    for mode in [
        CompileMode::Compatibility,
        CompileMode::Optimized,
        CompileMode::Mir6502,
    ] {
        let options = CompileOptions::for_mode(mode).with_module_path(&root);
        compile_file(&application, &options).unwrap_or_else(|error| {
            panic!("compile qualified aggregate relocation in {mode:?}: {error:?}")
        });
    }

    std::fs::remove_dir_all(root).expect("remove aggregate module fixture directory");
}
