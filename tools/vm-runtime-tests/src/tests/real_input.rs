use super::*;
use actionc::codegen::{
    CodegenArrayStorage, CodegenOutput, CodegenProfile, CodegenSourceRangeKind,
};
use actionc::map_query::MapQuery;

const CONSOLE: [u8; 6] = [0x40, 0x03, 0x75, 0, 0, 0]; // 3.75
const CURRENT: [u8; 6] = [0x40, 0x01, 0x25, 0, 0, 0]; // 1.25
const EXPLICIT: [u8; 6] = [0xc0, 0x02, 0x50, 0, 0, 0]; // -2.5
const PARTIAL: [u8; 6] = [0xc0, 0x07, 0x25, 0, 0, 0]; // -7.25

pub(super) fn qualify(fixture: &str) {
    let loaded =
        actionc::includes::load_compilation(runtime_fixture(fixture), &Default::default()).unwrap();
    let model = actionc::semantic::analyze_compilation_with_options(
        &loaded,
        actionc::semantic::SemanticOptions::modern(),
    )
    .unwrap();
    let semir = actionc::semantic::ir::lower_compilation(&loaded, &model);
    let nir = actionc::nir::optimize_program(&actionc::nir::lower_program(&semir)).unwrap();
    for runtime in [Runtime::ActionCart, Runtime::Standalone] {
        let classic = actionc::codegen::generate_semir_profile_at_origin_with_runtime(
            &semir,
            0x3000,
            CodegenProfile::Modern,
            runtime,
        )
        .unwrap();
        check(&classic, CompileMode::Optimized, runtime);
        let mir = actionc::mir6502::generate_output_with_config_and_runtime(
            &nir,
            0x3000,
            &actionc::mir6502::Mir6502Config::optimized(),
            runtime,
        )
        .unwrap();
        check(&mir, CompileMode::Mir6502, runtime);
    }
}

fn check(output: &CodegenOutput, mode: CompileMode, runtime: Runtime) {
    let image = actionc::codegen::format_load_file(output);
    let pristine = load_runtime_object(&image, runtime, true);
    let (default_channel, default_capacity, default_value) = match runtime {
        Runtime::ActionCart => (2, 255, CURRENT),
        Runtime::Standalone => (0, 126, CONSOLE),
    };
    let channels = [default_channel, 3];
    let capacities = [default_capacity, 126];
    let context = format!("{mode:?}/{runtime:?}");
    let mut buffers = [0; 2];

    // First match each real CIO request to the allocation that owns its header.
    // Exercise exact-capacity records only after checking those physical spans.
    for full in [false, true] {
        let vm = with_records(pristine.clone(), full);
        let outcome = VmRunner::new(vm).run(RunRequest {
            max_steps: 150_000,
            ..Default::default()
        });
        assert_eq!(
            outcome.stop_reason(),
            StopReason::StepLimit { max_steps: 150_000 },
            "{context}"
        );
        check_results(&outcome, default_value, EXPLICIT, &context);
        let reads: Vec<_> = outcome
            .vm
            .bus()
            .cio_observations()
            .iter()
            .filter(|call| call.command == 5)
            .collect();
        assert_eq!(reads.len(), 2, "{context}");
        for (i, call) in reads.into_iter().enumerate() {
            assert_eq!(
                (call.channel, call.length, call.bytes_read),
                (
                    Some(channels[i]),
                    capacities[i],
                    Some(if full { capacities[i] } else { 5 })
                ),
                "{context}"
            );
            assert_receive_space(output, call.buffer, call.length, &context);
            buffers[i] = call.buffer;
        }
    }

    for i in 0..2 {
        // InS subtracts one from the returned count even without an EOL.
        // Keeping the final '8' would produce a different packed REAL.
        let record = PartialRecord {
            expected_buffer: Some(buffers[i]),
            capacity: capacities[i],
            bytes: b"-7.258",
        };
        let outcome = run_failed_cio(
            with_records(pristine.clone(), false),
            mode,
            runtime,
            channels[i],
            5,
            Some(record),
            &[0x82],
        );
        let (default, explicit) = if i == 0 {
            (PARTIAL, EXPLICIT)
        } else {
            (default_value, PARTIAL)
        };
        check_results(&outcome, default, explicit, &context);
    }
}

fn assert_receive_space(output: &CodegenOutput, payload: u16, length: u16, context: &str) {
    let header = payload.checked_sub(1).unwrap();
    if let Some(backing) = output.skipped_ranges.iter().find(|r| r.start == header) {
        assert!(
            backing.len > length,
            "{context}: {backing:?} needs a header and {length} bytes"
        );
        return;
    }
    // Classic inline array symbols report their element width. The next symbol
    // or the end of initialized frame storage bounds the actual allocation.
    let owner = MapQuery::new(&output.map).storage_owner(header).unwrap();
    assert_eq!(
        (owner.offset, owner.symbol.array),
        (0, Some(CodegenArrayStorage::Inline)),
        "{context}"
    );
    let frame = output
        .map
        .source_ranges
        .iter()
        .find(|r| {
            r.kind == CodegenSourceRangeKind::StorageInitializer
                && r.start <= header
                && header < r.end
        })
        .unwrap();
    let end = output
        .map
        .storage_symbols
        .iter()
        .map(|s| s.address)
        .filter(|&addr| header < addr && addr < frame.end)
        .min()
        .unwrap_or(frame.end);
    assert!(
        end - header > length,
        "{context}: receive buffer at ${header:04X} needs a header and {length} bytes"
    );
}

fn with_records(mut vm: CompilerVm, full: bool) -> CompilerVm {
    let record = |number: &[u8], capacity: usize, eol| {
        let mut bytes = vec![b' '; if full { capacity - 1 - number.len() } else { 0 }];
        bytes.extend_from_slice(number);
        bytes.push(eol);
        bytes
    };
    let bus = vm.bus_mut();
    bus.queue_scripted_cio_input_bytes(&record(b"3.75", 126, 0x9b));
    bus.add_host_file("CURRENT.TXT", record(b"1.25", 255, b'\n'));
    bus.add_host_file("EXPLICIT.TXT", record(b"-2.5", 126, b'\n'));
    for address in [0x0680, 0x0690] {
        bus.ram_mut()
            .map(address, &[0xA5, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0x5A])
            .unwrap();
    }
    vm
}

fn check_results(outcome: &RunOutcome, default: [u8; 6], explicit: [u8; 6], context: &str) {
    assert_eq!(
        outcome.memory().read(0x06a0),
        0xa5,
        "{context}: fixture did not finish"
    );
    for (guard, expected) in [(0x0680, default), (0x0690, explicit)] {
        let actual: Vec<_> = (0..8).map(|i| outcome.memory().read(guard + i)).collect();
        assert_eq!(
            actual,
            [&[0xa5], expected.as_slice(), &[0x5a]].concat(),
            "{context} at ${guard:04X}"
        );
    }
}
