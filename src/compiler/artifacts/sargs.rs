use super::*;

const COPYRIGHT_BYTES: [u8; 10] = [0x08, 0x63, 0x09, 0x11, 0x19, 0x18, 0x13, 0x21, 0x23, 0x33];

pub(super) fn copyright_ranges(output: &CodegenOutput) -> Vec<StorageListingRange> {
    let targets = inline_jsr_data_lengths(output);
    output
        .map
        .routine_ranges
        .iter()
        .filter(|range| targets.contains_key(&range.start))
        .filter_map(|range| {
            let start = output_offset(output, range.start)?;
            let end = output_end_offset(output, range.end)?;
            let bytes = output.bytes.get(start..end)?;
            // The original SArgs skips this screen-code message via BNE to
            // RTS or JMP Break. Require both the bytes and that surrounding
            // code so a different helper implementation is not misclassified.
            let offset = bytes.windows(18).position(|window| {
                window[..5] == [0xD0, 0x0F, 0xE6, 0x11, 0x4C]
                    && window[7..17] == COPYRIGHT_BYTES
                    && window[17] == 0x60
            })?;
            Some(StorageListingRange {
                address: range.start.checked_add(u16::try_from(offset + 7).ok()?)?,
                bytes: COPYRIGHT_BYTES.to_vec(),
                name: "SArgs copyright (Atari screen codes)".to_string(),
            })
        })
        .collect()
}

#[derive(Default)]
pub(super) struct SArgsListing {
    descriptors: BTreeMap<u16, Vec<FrameParameter>>,
    parameters: BTreeMap<u16, FrameParameter>,
    copyright_addresses: BTreeSet<u16>,
}

#[derive(Clone)]
struct FrameParameter {
    address: u16,
    width: u16,
    name: String,
    type_name: String,
}

impl SArgsListing {
    pub(super) fn from_output(
        output: &CodegenOutput,
        instructions: &[DisassembledInstruction],
    ) -> Self {
        let mut listing = Self {
            copyright_addresses: copyright_ranges(output)
                .iter()
                .map(|range| range.address)
                .collect(),
            ..Self::default()
        };
        for pair in instructions.windows(2) {
            let [call, data] = pair else { unreachable!() };
            // The disassembler marks the three-byte payload only for a known
            // SArgs ABI target (including cartridge entries and local overrides).
            if call.mnemonic != "JSR"
                || call.address.checked_add(3) != Some(data.address)
                || data.mode.is_some()
                || data.bytes.len() != 3
            {
                continue;
            }
            let params = frame_parameters(output, data).unwrap_or_default();
            for param in &params {
                listing.parameters.insert(param.address, param.clone());
            }
            listing.descriptors.insert(data.address, params);
        }
        listing
    }

    pub(super) fn push_descriptor(
        &self,
        data: &DisassembledInstruction,
        output: &CodegenOutput,
        symbols: &MadsDisplaySymbols,
        relocations: &MadsRelocations<'_>,
        lines: &mut Vec<String>,
    ) -> bool {
        let Some(params) = self.descriptors.get(&data.address) else {
            return false;
        };
        let names = params
            .iter()
            .map(|param| param.name.as_str())
            .collect::<Vec<_>>();
        lines.push(sanitize_assembly_comment(&if names.is_empty() {
            "; SArgs descriptor".to_string()
        } else {
            format!("; SArgs descriptor for parameters: {}", names.join(", "))
        }));
        let mut payload = Vec::new();
        if let Some(word) = word_directive(output, data.address, &data.bytes, symbols, relocations)
        {
            symbols.push_definitions(data.address, &mut payload);
            payload.push(format_assembly_line(
                &word,
                data.address,
                &data.bytes[..2],
                None,
            ));
        } else {
            // Preserve unusual byte relocations or interior labels separately.
            push_data_listing(
                output,
                data.address,
                &data.bytes[..2],
                symbols,
                relocations,
                self,
                &mut payload,
            );
        }
        for line in payload.iter_mut().filter(|line| line.contains(';')) {
            line.push_str(" | parameter frame address");
        }
        let count_address = data.address + 2;
        push_data_listing(
            output,
            count_address,
            &data.bytes[2..],
            symbols,
            relocations,
            self,
            &mut payload,
        );
        if let Some(line) = payload.last_mut() {
            line.push_str(&format!(
                " | copy {} parameter bytes",
                u16::from(data.bytes[2]) + 1
            ));
        }
        // Long parameter labels may exceed the normal assembly column width.
        // Align the address comments within this payload even in that case.
        let column = payload
            .iter()
            .filter_map(|line| line.find(';'))
            .max()
            .unwrap_or(0);
        for line in payload {
            if let Some(at) = line.find(';') {
                lines.push(format!(
                    "{}{:<width$}{}",
                    &line[..at],
                    "",
                    &line[at..],
                    width = column - at
                ));
            } else {
                lines.push(line);
            }
        }
        true
    }

    pub(super) fn push_copyright(
        &self,
        address: u16,
        bytes: &[u8],
        symbols: &MadsDisplaySymbols,
        relocations: &MadsRelocations<'_>,
        lines: &mut Vec<String>,
    ) -> Option<u16> {
        if !self.copyright_addresses.contains(&address) || !bytes.starts_with(&COPYRIGHT_BYTES) {
            return None;
        }
        let width = COPYRIGHT_BYTES.len() as u16;
        let end = address.checked_add(width)?;
        if symbols.next_definition_after(address, end).is_some()
            || relocations.next_at_or_after(address, end).is_some()
        {
            return None;
        }
        lines.push(format_assembly_line(
            "DTA D'(c)1983ACS'",
            address,
            &COPYRIGHT_BYTES,
            None,
        ));
        Some(width)
    }

    pub(super) fn push_parameter(
        &self,
        output: &CodegenOutput,
        address: u16,
        bytes: &[u8],
        symbols: &MadsDisplaySymbols,
        relocations: &MadsRelocations<'_>,
        lines: &mut Vec<String>,
    ) -> Option<u16> {
        let param = self.parameters.get(&address)?;
        let bytes = bytes.get(..usize::from(param.width))?;
        let assembly = match param.width {
            1 if relocations.at(address).is_none() => format_byte_directive(bytes),
            2 => word_directive(output, address, bytes, symbols, relocations)?,
            _ => return None,
        };
        lines.push(format_assembly_line(
            &assembly,
            address,
            bytes,
            Some(&format!("{} {}", param.type_name, param.name)),
        ));
        Some(param.width)
    }
}

fn frame_parameters(
    output: &CodegenOutput,
    data: &DisassembledInstruction,
) -> Option<Vec<FrameParameter>> {
    let owner = output
        .map
        .routine_ranges
        .iter()
        .find(|routine| data.address >= routine.start && data.address < routine.end)?;
    let signature = output
        .map
        .routine_signatures
        .iter()
        .find(|signature| signature.name.eq_ignore_ascii_case(&owner.name))?;
    let base = le_u16_from_slice(&data.bytes)?;
    let count = u16::from(data.bytes[2]) + 1;
    let names = ListingNames::from_output(output);
    let mut cursor = base;
    let mut params = Vec::new();
    for param in &signature.params {
        let name = strip_routine_lexical_prefix(&param.name, &owner.name);
        if param.width == 0 {
            return None;
        }
        let symbol = output.map.storage_symbols.iter().find(|symbol| {
            symbol.kind == CodegenSymbolKind::Parameter
                && matches!(&symbol.scope, CodegenSymbolScope::Routine(routine) if routine.eq_ignore_ascii_case(&owner.name))
                && strip_routine_lexical_prefix(&symbol.name, &owner.name).eq_ignore_ascii_case(name)
                && symbol.address == cursor
                && storage_listing_size(symbol) == param.width
        })?;
        params.push(FrameParameter {
            address: cursor,
            width: param.width,
            name: names.parameter_name(&owner.name, name).to_string(),
            // Array parameters carry an address, not an array element.
            type_name: if symbol.array.is_some() {
                "WORD".to_string()
            } else {
                param.type_name.clone()
            },
        });
        cursor = cursor.checked_add(param.width)?;
    }
    (cursor.checked_sub(base)? == count).then_some(params)
}

fn word_directive(
    output: &CodegenOutput,
    address: u16,
    bytes: &[u8],
    symbols: &MadsDisplaySymbols,
    relocations: &MadsRelocations<'_>,
) -> Option<String> {
    let high_address = address.checked_add(1)?;
    if symbols.definitions.contains_key(&high_address) {
        return None;
    }
    let expression = match (relocations.at(address), relocations.at(high_address)) {
        (Some(word), None) if word.kind == CodegenRelocationKind::Word16 => {
            relocation_expression(output, symbols, word, address)?
        }
        (Some(low), Some(high))
            if low.kind == CodegenRelocationKind::Low8
                && high.kind == CodegenRelocationKind::High8
                && low.target_offset == high.target_offset
                && low.addend == high.addend =>
        {
            relocation_expression(output, symbols, low, address)?
        }
        (None, None) => format!("${:04X}", le_u16_from_slice(bytes)?),
        _ => return None,
    };
    Some(format!(".WORD {expression}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::{CodegenRoutineParam, RoutineAddress};
    use crate::compiler::{CompileMode, CompileOptions, compile_file};

    fn output() -> CodegenOutput {
        compile_file(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/listing/mads_sargs.act"),
            &CompileOptions::for_mode(CompileMode::Compatibility),
        )
        .unwrap()
        .output
    }

    #[test]
    fn missing_or_mismatched_layout_keeps_generic_descriptor_comments() {
        let original = output();
        for missing in [true, false] {
            let mut output = original.clone();
            if missing {
                output.map.routine_signatures.clear();
            } else {
                for signature in &mut output.map.routine_signatures {
                    signature.params.push(CodegenRoutineParam {
                        name: "unknown".into(),
                        type_name: "BYTE".into(),
                        width: 1,
                    });
                }
            }
            let listing = format_listing_with_boundaries(&output);
            assert_eq!(
                listing
                    .lines()
                    .filter(|line| *line == "; SArgs descriptor")
                    .count(),
                3
            );
            assert!(!listing.contains("; SArgs descriptor for parameters:"));
            assert_eq!(listing.matches(".WORD param_").count(), 3);
            assert_eq!(listing.matches("| copy 5 parameter bytes").count(), 2);
        }
    }

    #[test]
    fn descriptor_count_ff_means_256_bytes() {
        let mut output = output();
        let instructions = disassemble_code_ranges(&output);
        let frames = SArgsListing::from_output(&output, &instructions);
        let address = *frames.descriptors.keys().next().unwrap();
        output.bytes[usize::from(address - output.origin) + 2] = 0xFF;
        let listing = format_listing_with_boundaries(&output);
        assert!(listing.contains("| copy 256 parameter bytes"));
        assert!(!listing.contains("; SArgs descriptor for parameters: x, y, col"));
    }

    #[test]
    fn an_unbound_routine_named_sargs_does_not_classify_call_payloads() {
        let mut output = output();
        let instructions = disassemble_code_ranges(&output);
        let frames = SArgsListing::from_output(&output, &instructions);
        let address = *frames.descriptors.keys().next().unwrap();
        let call_operand = usize::from(address - output.origin) - 2;
        output.bytes[call_operand..call_operand + 2].copy_from_slice(&0x9000u16.to_le_bytes());
        output.map.routine_addresses.push(RoutineAddress {
            name: "SArgs".into(),
            address: 0x9000,
        });
        let listing = format_listing_with_boundaries(&output);
        assert_eq!(listing.matches("; SArgs descriptor").count(), 2);
        assert!(!listing.contains("; SArgs descriptor for parameters: x, y, col"));
    }
}
