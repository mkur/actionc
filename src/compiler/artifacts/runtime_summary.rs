use super::*;
use crate::codegen::is_embedded_runtime_symbol;

pub(super) fn push_summary(output: &CodegenOutput, lines: &mut Vec<String>) {
    let ranges = embedded_ranges(output);
    let bytes: u32 = ranges.iter().map(|(start, end)| end - start).sum();
    match ranges.as_slice() {
        [] => lines.push("; Embedded runtime: none ($0000 bytes)".to_string()),
        [(start, end)] => lines.push(format!(
            "; Embedded runtime: ${start:04X} -> ${:04X} (${bytes:04X} bytes, {bytes} decimal; includes transitive dependencies)",
            end - 1,
        )),
        _ => {
            lines.push(format!(
                "; Embedded runtime: ${bytes:04X} bytes ({bytes} decimal) in {} ranges; includes transitive dependencies",
                ranges.len(),
            ));
            for (start, end) in ranges {
                lines.push(format!(
                    "; Runtime range: ${start:04X} -> ${:04X} (${:04X} bytes)",
                    end - 1,
                    end - start,
                ));
            }
        }
    }
}

fn embedded_ranges(output: &CodegenOutput) -> Vec<(u32, u32)> {
    // These are the final linked providers, so dependencies are present even
    // when they do not have a direct runtime-binding entry. Initializer ranges
    // include physical backing data and parameter homes, unlike symbol sizes.
    let routines = output
        .map
        .routine_ranges
        .iter()
        .filter(|range| is_embedded_runtime_symbol(&range.name))
        .map(|range| (range.start, range.end));
    let data = output
        .map
        .source_ranges
        .iter()
        .filter(|range| range.kind == CodegenSourceRangeKind::StorageInitializer)
        .filter(|range| {
            range
                .name
                .as_deref()
                .is_some_and(is_embedded_runtime_symbol)
        })
        .map(|range| (range.start, range.end));
    let image_start = u32::from(output.origin);
    let image_end = (image_start + output.bytes.len() as u32).min(0x10000);
    let mut ranges = routines
        .chain(data)
        .filter_map(|(start, end)| {
            if start == end {
                return None;
            }
            let start = u32::from(start).max(image_start);
            let end = if end == 0 { 0x10000 } else { u32::from(end) }.min(image_end);
            (start < end).then_some((start, end))
        })
        .collect::<Vec<_>>();
    ranges.sort_unstable();
    let mut merged = Vec::<(u32, u32)>::new();
    for (start, end) in ranges {
        if let Some((_, previous_end)) = merged.last_mut()
            && start <= *previous_end
        {
            *previous_end = (*previous_end).max(end);
        } else {
            merged.push((start, end));
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::{CodegenSourceRange, RoutineRange};
    use crate::compiler::{CompileMode, CompileOptions, compile_file};
    use crate::runtime::Runtime;
    use crate::source::Span;

    fn compile(mode: CompileMode, runtime: Runtime, origin: u16) -> CodegenOutput {
        compile_file(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/listing/mads_runtime_summary.act"),
            &CompileOptions::for_mode(mode)
                .with_runtime(runtime)
                .with_origin(origin),
        )
        .unwrap()
        .output
    }

    #[test]
    fn summary_counts_loaded_code_and_data_once_without_counting_gaps() {
        let mut output = compile(CompileMode::Compatibility, Runtime::ActionCart, 0x3000);
        output.bytes = vec![0; 64];
        output.map.routine_ranges = [
            ("ACTION.RUNTIME.SYSLIB::MultI", 0x3010, 0x3020),
            ("ACTION.RUNTIME.SYSLIB::MultB", 0x3018, 0x3028),
            ("Application", 0x3028, 0x3030),
            ("ACTION.RUNTIME.SYSLIB::Empty", 0, 0),
        ]
        .into_iter()
        .map(|(name, start, end)| RoutineRange {
            name: name.into(),
            start,
            end,
        })
        .collect();
        output.map.source_ranges = [
            ("M_ACTION_RUNTIME_RESIDENT_DATA_DEADBEEF", 0x3004, 0x3008),
            ("__nir_str_ACTION_RUNTIME_RESIDENT__Log_1", 0x3006, 0x300A),
            ("ACTION.RUNTIME.RESIDENT::Tail", 0x3038, 0x3060),
            ("ACTION.RUNTIME.RESIDENT::Deferred", 0x3060, 0x3070),
            ("__nir_str_Main_1", 0x300A, 0x3010),
            ("ACTION.RUNTIME.RESIDENT::ZeroPage", 0, 0x0010),
        ]
        .into_iter()
        .map(|(name, start, end)| CodegenSourceRange {
            name: Some(name.into()),
            kind: CodegenSourceRangeKind::StorageInitializer,
            source_span: Span::new(0, 0),
            start,
            end,
        })
        .collect();
        let mut lines = Vec::new();
        push_summary(&output, &mut lines);
        assert_eq!(
            lines,
            [
                "; Embedded runtime: $0026 bytes (38 decimal) in 3 ranges; includes transitive dependencies",
                "; Runtime range: $3004 -> $3009 ($0006 bytes)",
                "; Runtime range: $3010 -> $3027 ($0018 bytes)",
                "; Runtime range: $3038 -> $303F ($0008 bytes)",
            ]
        );
    }

    #[test]
    fn summary_includes_transitive_helpers_and_resident_storage_in_every_mode() {
        for mode in [
            CompileMode::Compatibility,
            CompileMode::Optimized,
            CompileMode::Mir6502,
        ] {
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                let mut previous = None;
                for origin in [0x3000, 0x41C7] {
                    let output = compile(mode, runtime, origin);
                    let ranges = embedded_ranges(&output);
                    let covered = |start: u16, end: u16| {
                        ranges
                            .iter()
                            .any(|&(lo, hi)| lo <= u32::from(start) && hi >= u32::from(end))
                    };
                    let owned = output
                        .map
                        .routine_ranges
                        .iter()
                        .filter(|range| {
                            is_embedded_runtime_symbol(&range.name) && range.start < range.end
                        })
                        .collect::<Vec<_>>();
                    assert!(!owned.is_empty(), "division must link owned code");
                    for routine in owned {
                        assert!(covered(routine.start, routine.end), "{}", routine.name);
                    }
                    let main = output
                        .map
                        .routine_ranges
                        .iter()
                        .find(|range| range.name == "Main")
                        .unwrap();
                    assert!(!covered(main.start, main.end));
                    if runtime == Runtime::Standalone {
                        assert!(output.map.routine_ranges.iter().any(|range| {
                            runtime_symbol_component(&range.name).as_deref() == Some("syslib_multb")
                        }));
                        assert!(
                            !output
                                .map
                                .runtime_bindings
                                .iter()
                                .any(|binding| { binding.helper.eq_ignore_ascii_case("MultB") }),
                            "MultB is a transitive dependency, not a direct request"
                        );
                        // DevS contains two backing bytes and a four-byte
                        // descriptor. Counting only its public symbol misses
                        // the backing. Its initializer covers all six bytes.
                        let data = output
                            .map
                            .source_ranges
                            .iter()
                            .find(|range| {
                                range.kind == CodegenSourceRangeKind::StorageInitializer
                                    && range
                                        .name
                                        .as_deref()
                                        .and_then(runtime_symbol_component)
                                        .as_deref()
                                        == Some("resident_dev_s")
                            })
                            .unwrap_or_else(|| {
                                panic!(
                                    "{mode:?}: missing physical runtime data range: {:?}",
                                    output
                                        .map
                                        .source_ranges
                                        .iter()
                                        .filter(|range| range.kind
                                            == CodegenSourceRangeKind::StorageInitializer)
                                        .collect::<Vec<_>>()
                                )
                            });
                        assert_eq!(data.end - data.start, 6);
                        assert!(covered(data.start, data.end));
                    }
                    let relative = ranges
                        .iter()
                        .map(|&(start, end)| (start - u32::from(origin), end - u32::from(origin)))
                        .collect::<Vec<_>>();
                    if let Some(previous) = previous.replace(relative.clone()) {
                        assert_eq!(previous, relative, "summary must rebase with the image");
                    }
                    let mut without_bindings = output.clone();
                    without_bindings.map.runtime_bindings.clear();
                    assert_eq!(ranges, embedded_ranges(&without_bindings));
                }
            }
        }
    }

    #[test]
    fn cartridge_services_and_application_helper_overrides_add_no_embedded_bytes() {
        for mode in [
            CompileMode::Compatibility,
            CompileMode::Optimized,
            CompileMode::Mir6502,
        ] {
            let output = compile_file(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("fixtures/listing/mads_sargs_override.act"),
                &CompileOptions::for_mode(mode),
            )
            .unwrap()
            .output;
            let mut lines = Vec::new();
            push_summary(&output, &mut lines);
            assert_eq!(lines, ["; Embedded runtime: none ($0000 bytes)"]);
        }
    }
}
