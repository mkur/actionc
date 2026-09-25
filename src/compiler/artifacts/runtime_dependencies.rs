use super::*;
use crate::codegen::is_embedded_runtime_symbol;

pub(super) fn push_dependencies(
    output: &CodegenOutput,
    instructions: &[DisassembledInstruction],
    names: &ListingNames,
    lines: &mut Vec<String>,
) {
    let bound = output
        .map
        .runtime_bindings
        .iter()
        .filter_map(|binding| binding.address)
        .collect::<BTreeSet<_>>();
    let dependencies = output
        .map
        .routine_addresses
        .iter()
        .filter(|routine| {
            is_embedded_runtime_symbol(&routine.name)
                && output_offset(output, routine.address).is_some()
                && !bound.contains(&routine.address)
        })
        .map(|routine| (routine.address, routine))
        .collect::<BTreeMap<_, _>>();
    if dependencies.is_empty() {
        return;
    }

    // Short names are convenient for searching. Keep module qualifiers when
    // more than one provider (or application routine) has the same spelling.
    let mut spellings = BTreeMap::<String, BTreeSet<String>>::new();
    for routine in &output.map.routine_addresses {
        let display = names.display(&routine.name);
        let short = display
            .rsplit_once("::")
            .map_or(display.as_str(), |(_, name)| name);
        spellings
            .entry(short.to_ascii_uppercase())
            .or_default()
            .insert(display);
    }
    let display_name = |name: &str| {
        let display = names.display(name);
        let short = display
            .rsplit_once("::")
            .map_or(display.as_str(), |(_, name)| name);
        if spellings
            .get(&short.to_ascii_uppercase())
            .is_some_and(|names| names.len() == 1)
        {
            short.to_string()
        } else {
            display
        }
    };
    let owner_at = |address: u16| {
        output.map.routine_ranges.iter().find(|range| {
            range.start != range.end
                && address >= range.start
                && (address < range.end || range.end == 0)
        })
    };
    let mut parents = BTreeMap::<u16, BTreeSet<String>>::new();
    let mut record_reference = |source: u16, target: u16| {
        let Some(parent) = owner_at(source) else {
            return;
        };
        let dependency = dependencies.get(&target).copied().or_else(|| {
            // Some runtime jumps enter another routine after its prologue.
            let owner = owner_at(target)?;
            dependencies
                .values()
                .copied()
                .find(|routine| routine.name == owner.name)
        });
        let Some(dependency) = dependency else { return };
        if !parent.name.eq_ignore_ascii_case(&dependency.name) {
            parents
                .entry(dependency.address)
                .or_default()
                .insert(display_name(&parent.name));
        }
    };
    for instruction in instructions {
        if let Some(target) = instruction_target(instruction) {
            record_reference(instruction.address, target);
        }
        // Handwritten runtime entry points can fall through into the next
        // routine, e.g. SetSign into SS1. Inline data is never executable here.
        if instruction.mode.is_some()
            && !matches!(instruction.mnemonic, "JMP" | "RTS" | "RTI" | "BRK")
            && let Some(next) = instruction
                .address
                .checked_add(instruction.bytes.len() as u16)
            && dependencies.contains_key(&next)
        {
            record_reference(instruction.address, next);
        }
    }
    // Address-taken helpers need not have a direct JSR/JMP. Use explicit
    // relocation facts rather than treating arbitrary data bytes as pointers.
    for relocation in &output.relocations {
        let source = u32::from(output.origin) + u32::from(relocation.value_offset);
        let target = i64::from(output.origin)
            + i64::from(relocation.target_offset)
            + i64::from(relocation.addend);
        if let (Ok(source), Ok(target)) = (u16::try_from(source), u16::try_from(target)) {
            record_reference(source, target);
        }
    }
    for (address, routine) in dependencies {
        let reason = match parents.get(&address) {
            Some(parents) => format!(
                "required by {}",
                parents.iter().cloned().collect::<Vec<_>>().join(", ")
            ),
            None => "linked runtime dependency".to_string(),
        };
        lines.push(sanitize_assembly_comment(&format!(
            "; Runtime dependency: {} -> ${address:04X} ({reason})",
            display_name(&routine.name),
        )));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::{CompileMode, CompileOptions, compile_file};
    use crate::runtime::Runtime;

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

    fn dependency_lines(output: &CodegenOutput) -> Vec<String> {
        let mut lines = Vec::new();
        push_dependencies(
            output,
            &disassemble_code_ranges(output),
            &ListingNames::from_output(output),
            &mut lines,
        );
        lines
    }

    #[test]
    fn address_taken_dependencies_are_qualified_without_guessing_data_references() {
        use crate::codegen::{RoutineAddress, RoutineRange};

        let mut output = compile(CompileMode::Compatibility, Runtime::ActionCart, 0x3000);
        output.bytes = vec![0; 0x20];
        output.bytes[..5].copy_from_slice(&[0xA9, 0x10, 0xA2, 0x30, 0x60]);
        output.bytes[8..10].copy_from_slice(&[0x12, 0x30]); // Unrelated data, not a relocation.
        output.bytes[0x10] = 0x60;
        output.bytes[0x12] = 0x60;
        output.map.runtime_bindings.clear();
        output.map.source_ranges.clear();
        output.map.routine_ranges = [
            ("Main", 0x3000, 0x3005),
            ("ACTION.RUNTIME.SYSLIB::Helper", 0x3010, 0x3011),
            ("ACTION.RUNTIME.RESIDENT::Helper", 0x3012, 0x3013),
        ]
        .into_iter()
        .map(|(name, start, end)| RoutineRange {
            name: name.into(),
            start,
            end,
        })
        .collect();
        output.map.routine_addresses = output
            .map
            .routine_ranges
            .iter()
            .map(|range| RoutineAddress {
                name: range.name.clone(),
                address: range.start,
            })
            .collect();
        output.relocations = [
            (1, CodegenRelocationKind::Low8),
            (3, CodegenRelocationKind::High8),
        ]
        .into_iter()
        .map(|(value_offset, kind)| CodegenRelocation {
            value_offset,
            target_offset: 0x10,
            addend: 0,
            kind,
        })
        .collect();
        assert_eq!(
            dependency_lines(&output),
            [
                "; Runtime dependency: ACTION.RUNTIME.SYSLIB::Helper -> $3010 (required by Main)",
                "; Runtime dependency: ACTION.RUNTIME.RESIDENT::Helper -> $3012 (linked runtime dependency)",
            ]
        );
    }

    #[test]
    fn transitive_runtime_imports_include_addresses_and_callers_in_every_mode() {
        for mode in [
            CompileMode::Compatibility,
            CompileMode::Optimized,
            CompileMode::Mir6502,
        ] {
            for runtime in [Runtime::ActionCart, Runtime::Standalone] {
                for origin in [0x3000, 0x41C7] {
                    let output = compile(mode, runtime, origin);
                    let lines = dependency_lines(&output);
                    if runtime == Runtime::ActionCart {
                        assert!(lines.is_empty(), "{mode:?}: {lines:?}");
                        continue;
                    }
                    for (component, name, parents) in [
                        ("syslib_multb", "MultB", "MultI"),
                        ("syslib_smops", "SMOps", "MultI"),
                        ("syslib_ss1", "SS1", "SMOps, SetSign"),
                        ("resident_open", "Open", "ChkErr, Graphics"),
                    ] {
                        let routine = output
                            .map
                            .routine_addresses
                            .iter()
                            .find(|routine| {
                                runtime_symbol_component(&routine.name).as_deref()
                                    == Some(component)
                            })
                            .expect(component);
                        let expected = format!(
                            "; Runtime dependency: {name} -> ${:04X} (required by {parents})",
                            routine.address
                        );
                        assert!(
                            lines.contains(&expected),
                            "{mode:?}: missing {expected}\n{lines:#?}"
                        );
                        assert_eq!(
                            lines
                                .iter()
                                .filter(|line| line
                                    .starts_with(&format!("; Runtime dependency: {name} ->")))
                                .count(),
                            1
                        );
                    }
                    assert!(
                        !lines
                            .iter()
                            .any(|line| line.starts_with("; Runtime dependency: MultI ->")),
                        "direct binding must not be repeated: {lines:#?}"
                    );
                    assert!(!lines.iter().any(|line| line.contains("Main ->")));
                    let plain = format_listing_with_boundaries(&output);
                    let source = format_listing_with_source(&output, "");
                    for line in lines {
                        assert!(plain.contains(&line));
                        assert!(source.contains(&line));
                    }
                }
            }
        }
    }
}
