use actionc::{
    codegen::{self, CodegenOutput},
    compiler::Runtime,
    nir,
    semantic::{self, ir::*},
};
use actionc_vm::{BusAccess, CompilerVm, ExecutionProfile};

fn constructor_capture(body: &[SemStmt]) -> Option<String> {
    let mut capture = None;
    for stmt in body {
        match stmt {
            SemStmt::LexicalBlock { body, .. } => {
                capture = constructor_capture(body).or(capture);
            }
            SemStmt::Assign { target, .. } => {
                if let SemLValueKind::Field { base, field } = &target.kind
                    && field.name == "__variant_tag"
                    && let SemLValueKind::Symbol(symbol) = &base.kind
                {
                    // Nested constructors finish first; the last tag write
                    // identifies the outer constructor capture.
                    capture = Some(symbol.name.clone());
                }
            }
            _ => {}
        }
    }
    capture
}

fn execute(output: &CodegenOutput, capture: &str, expected: &[u8], lane: &str) {
    let slot = output
        .map
        .storage_symbols
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(capture))
        .unwrap_or_else(|| {
            panic!(
                "{lane}: missing {capture}: {:?}",
                output.map.storage_symbols
            )
        });
    let destination = output
        .map
        .storage_symbols
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case("current"))
        .unwrap();
    assert_eq!(usize::from(slot.size), expected.len());
    assert_eq!(slot.size, destination.size);
    let mut vm = CompilerVm::default();
    vm.load_atari_object_for_execution(
        if output.map.runtime == Runtime::ActionCart {
            ExecutionProfile::CartridgeObject
        } else {
            ExecutionProfile::StandaloneObject
        },
        &codegen::format_load_file(output),
    )
    .unwrap();
    // Poison the private temporary as well as the destination, without
    // changing declarations or relying on their load-time zero images.
    for address in slot.address..slot.address + slot.size {
        vm.bus_mut().ram_mut().write(address, 0xCC);
        vm.bus_mut().add_watchpoint(address);
    }
    for address in destination.address..destination.address + destination.size {
        vm.bus_mut().ram_mut().write(address, 0xDD);
    }
    vm.bus_mut().ram_mut().write(0x600, 0);
    vm.bus_mut().clear_events();
    for step in 0..300_000 {
        vm.step_cpu().unwrap();
        if vm.bus().ram().read(0x600) == 0xA5 {
            break;
        }
        assert!(step < 299_999, "{lane}: constructor did not finish");
    }
    let writes: Vec<_> = vm
        .bus()
        .events()
        .iter()
        .filter(|e| e.access == BusAccess::Write)
        .collect();
    assert_eq!(
        writes.len(),
        expected.len(),
        "{lane}: constructor rewrote bytes: {writes:?}"
    );
    let mut counts = vec![0u8; expected.len()];
    for write in &writes {
        let offset = usize::from(write.address - slot.address);
        counts[offset] += 1;
        assert_eq!(write.value, expected[offset], "{lane}: byte {offset}");
    }
    assert!(counts.iter().all(|count| *count == 1), "{lane}: {counts:?}");
    let last = writes.last().unwrap();
    assert_eq!(
        (last.address, last.value),
        (slot.address, expected[0]),
        "{lane}: tag must be last"
    );
    let actual: Vec<_> = (destination.address..destination.address + destination.size)
        .map(|address| vm.bus().ram().read(address))
        .collect();
    assert_eq!(actual, expected, "{lane}: complete destination value");
}

fn check(source: &str, expected: &[u8]) {
    let ast = actionc::parser::parse(&actionc::lexer::tokenize(source).unwrap()).unwrap();
    let model = semantic::analyze_with_options(&ast, semantic::SemanticOptions::modern()).unwrap();
    let semir = semantic::ir::lower_program(&ast, &model);
    let capture = semir
        .modules
        .iter()
        .flat_map(|m| &m.items)
        .find_map(|item| match item {
            SemItem::Routine(r) if r.symbol.name == "Main" => constructor_capture(&r.body),
            _ => None,
        })
        .unwrap();
    let raw = nir::lower_program(&semir);
    nir::verify_program(&raw).unwrap();
    let optimized = nir::optimize_program(&raw).unwrap();
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let output = codegen::generate_semir_profile_at_origin_with_runtime(
            &semir,
            0x3000,
            codegen::CodegenProfile::Modern,
            runtime,
        )
        .unwrap();
        execute(&output, &capture, expected, &format!("classic/{runtime:?}"));
        for (name, program, config) in [
            ("raw", &raw, actionc::mir6502::Mir6502Config::default()),
            (
                "optimized-nir",
                &optimized,
                actionc::mir6502::Mir6502Config::default(),
            ),
            (
                "optimized-mir",
                &optimized,
                actionc::mir6502::Mir6502Config::optimized(),
            ),
        ] {
            let output = actionc::mir6502::generate_output_with_config_and_runtime(
                program, 0x3000, &config, runtime,
            )
            .unwrap();
            execute(
                &output,
                &capture,
                expected,
                &format!("MIR/{name}/{runtime:?}"),
            );
        }
    }
}

#[test]
fn constructor_initializes_each_byte_once_and_commits_tag_last() {
    for (definition, constructor, expected) in [
        (
            "TYPE Value=VARIANT [NONE SOME [BYTE value]]",
            "Value.SOME(42)",
            vec![2, 42],
        ),
        (
            "TYPE Value=VARIANT [NONE SOME [BYTE value]]",
            "Value.NONE",
            vec![1, 0],
        ),
        ("TYPE Value=VARIANT [ONLY]", "Value.ONLY", vec![1]),
        (
            "TYPE Value=VARIANT [DATA [BYTE a CARD b BYTE c]]",
            "Value.DATA(11,$2345,67)",
            vec![1, 11, 0x45, 0x23, 67],
        ),
        (
            "TYPE Inner=VARIANT [NONE SOME [BYTE value]] TYPE Value=VARIANT [NONE DATA [Inner inner]]",
            "Value.DATA(Inner.SOME(42))",
            vec![2, 2, 42],
        ),
    ] {
        check(
            &format!(
                "{definition} Value current BYTE done=$600 PROC Main() current={constructor} done=$A5 DO OD RETURN"
            ),
            &expected,
        );
    }
}

#[test]
fn short_alternatives_clear_poisoned_page_crossing_unused_regions() {
    for constructor in ["Value.SMALL(42)", "Value.EMPTY"] {
        let source = format!(
            "TYPE Payload=[BYTE ARRAY bytes(257)] TYPE Value=VARIANT [EMPTY DATA [Payload payload] SMALL [BYTE code]] Value current BYTE done=$600 PROC Main() current={constructor} done=$A5 DO OD RETURN"
        );
        let mut expected = vec![0; 258];
        expected[0] = if constructor == "Value.EMPTY" { 1 } else { 3 };
        expected[1] = if constructor == "Value.EMPTY" { 0 } else { 42 };
        check(&source, &expected);
    }
}

#[test]
fn copied_union_payload_keeps_its_complete_byte_image() {
    check(
        "TYPE View=UNION [CARD word BYTE ARRAY bytes(3)] TYPE Value=VARIANT [NONE DATA [View view]] View original Value current BYTE done=$600 PROC Main() original.word=$1234 original.bytes(2)=$A5 current=Value.DATA(original) done=$A5 DO OD RETURN",
        &[2, 0x34, 0x12, 0xA5],
    );
}
