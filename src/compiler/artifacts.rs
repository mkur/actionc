use std::collections::BTreeMap;

use crate::codegen::{
    AddressingMode, CodegenOutput, CodegenSourceRangeKind, DisassembledInstruction,
    disassemble_with_origin_and_inline_jsr_data,
};
use crate::map_query::MapQuery;

pub(crate) fn format_listing_with_source(output: &CodegenOutput, source_text: &str) -> String {
    let query = MapQuery::with_source(&output.map, source_text);
    let mut lines = Vec::new();
    let mut last_source = None;
    let boundary_comments = routine_boundary_comments(output);
    let instructions = disassemble_code_ranges(output);
    let generated_labels = generated_code_labels(&instructions);
    let routine_labels = routine_address_labels(output);

    for item in listing_items(output, &instructions) {
        match item {
            ListingItem::Instruction(instruction) => {
                if let Some(comments) = boundary_comments.get(&instruction.address) {
                    lines.extend(comments.iter().cloned());
                } else if let Some(label) = generated_labels.get(&instruction.address) {
                    lines.push(format!("{label}:"));
                }
                push_source_comment(&query, instruction.address, &mut last_source, &mut lines);
                lines.push(format_instruction_listing(&instruction, &routine_labels));
            }
            ListingItem::Data {
                address,
                bytes,
                name,
            } => {
                if let Some(comments) = boundary_comments.get(&address) {
                    lines.extend(comments.iter().cloned());
                }
                push_source_comment(&query, address, &mut last_source, &mut lines);
                if let Some(name) = name {
                    lines.push(format!("; ===== DATA {name} ${address:04X} ====="));
                }
                lines.push(format_data_listing(address, &bytes));
            }
        }
    }

    append_trailing_boundary_comments(output, &boundary_comments, &mut lines);
    lines.join("\n")
}

pub(crate) fn format_listing_with_boundaries(output: &CodegenOutput) -> String {
    let boundary_comments = routine_boundary_comments(output);
    let mut lines = Vec::new();
    let instructions = disassemble_code_ranges(output);
    let generated_labels = generated_code_labels(&instructions);
    let routine_labels = routine_address_labels(output);

    for item in listing_items(output, &instructions) {
        match item {
            ListingItem::Instruction(instruction) => {
                if let Some(comments) = boundary_comments.get(&instruction.address) {
                    lines.extend(comments.iter().cloned());
                } else if let Some(label) = generated_labels.get(&instruction.address) {
                    lines.push(format!("{label}:"));
                }
                lines.push(format_instruction_listing(&instruction, &routine_labels));
            }
            ListingItem::Data {
                address,
                bytes,
                name,
            } => {
                if let Some(comments) = boundary_comments.get(&address) {
                    lines.extend(comments.iter().cloned());
                }
                if let Some(name) = name {
                    lines.push(format!("; ===== DATA {name} ${address:04X} ====="));
                }
                lines.push(format_data_listing(address, &bytes));
            }
        }
    }

    append_trailing_boundary_comments(output, &boundary_comments, &mut lines);
    lines.join("\n")
}

#[derive(Debug, Clone)]
enum ListingItem {
    Instruction(DisassembledInstruction),
    Data {
        address: u16,
        bytes: Vec<u8>,
        name: Option<String>,
    },
}

fn listing_items(
    output: &CodegenOutput,
    instructions: &[DisassembledInstruction],
) -> Vec<ListingItem> {
    let mut items = Vec::new();
    let mut instruction_index = 0;
    let mut cursor = output.origin;
    let end = output
        .origin
        .wrapping_add(u16::try_from(output.bytes.len()).unwrap_or(u16::MAX));
    let storage = storage_listing_ranges(output);
    let mut storage_index = 0;

    while cursor < end {
        if let Some(symbol) = storage.get(storage_index)
            && symbol.address == cursor
        {
            for (chunk_index, chunk) in symbol.bytes.chunks(8).enumerate() {
                items.push(ListingItem::Data {
                    address: symbol.address.saturating_add((chunk_index * 8) as u16),
                    bytes: chunk.to_vec(),
                    name: (chunk_index == 0).then(|| symbol.name.clone()),
                });
            }
            cursor = symbol.address.saturating_add(symbol.bytes.len() as u16);
            storage_index += 1;
            continue;
        }

        while let Some(instruction) = instructions.get(instruction_index)
            && instruction.address < cursor
        {
            instruction_index += 1;
        }

        if let Some(instruction) = instructions.get(instruction_index)
            && instruction.address == cursor
        {
            items.push(ListingItem::Instruction(instruction.clone()));
            cursor = cursor.saturating_add(instruction.bytes.len() as u16);
            instruction_index += 1;
            continue;
        }

        let Some(offset) = output_offset(output, cursor) else {
            break;
        };
        items.push(ListingItem::Data {
            address: cursor,
            bytes: vec![output.bytes[offset]],
            name: None,
        });
        cursor = cursor.saturating_add(1);
    }

    items
}

#[derive(Debug, Clone)]
struct StorageListingRange {
    address: u16,
    bytes: Vec<u8>,
    name: String,
}

fn storage_listing_ranges(output: &CodegenOutput) -> Vec<StorageListingRange> {
    let mut ranges = output
        .map
        .storage_symbols
        .iter()
        .filter(|symbol| !address_in_routine(output, symbol.address))
        .filter_map(|symbol| {
            let start = output_offset(output, symbol.address)?;
            let end = start.saturating_add(symbol.size as usize);
            let bytes = output.bytes.get(start..end)?.to_vec();
            Some(StorageListingRange {
                address: symbol.address,
                bytes,
                name: symbol.name.clone(),
            })
        })
        .collect::<Vec<_>>();
    ranges.extend(storage_source_listing_ranges(output));
    ranges.sort_by_key(|range| range.address);
    ranges.dedup_by_key(|range| range.address);
    ranges
}

fn storage_source_listing_ranges(output: &CodegenOutput) -> Vec<StorageListingRange> {
    output
        .map
        .source_ranges
        .iter()
        .filter(|range| range.kind == CodegenSourceRangeKind::StorageInitializer)
        .filter_map(|range| {
            let start = output_offset(output, range.start)?;
            let end = output_end_offset(output, range.end)?;
            let bytes = output.bytes.get(start..end)?.to_vec();
            (!bytes.is_empty()).then(|| StorageListingRange {
                address: range.start,
                bytes,
                name: range.name.clone().unwrap_or_else(|| "storage".to_string()),
            })
        })
        .collect()
}

fn disassemble_code_ranges(output: &CodegenOutput) -> Vec<DisassembledInstruction> {
    let mut ranges = output.map.routine_ranges.clone();
    ranges.sort_by_key(|range| range.start);
    let mut storage = storage_source_listing_ranges(output);
    storage.sort_by_key(|range| range.address);
    let inline_jsr_data_lengths = inline_jsr_data_lengths(output);
    let mut instructions = Vec::new();
    for range in ranges {
        let mut cursor = range.start;
        for data in storage.iter().filter(|data| {
            let data_end = data.address.saturating_add(data.bytes.len() as u16);
            data.address < range.end && data_end > range.start
        }) {
            if data.address > cursor {
                push_disassembled_range(
                    output,
                    cursor,
                    data.address,
                    &inline_jsr_data_lengths,
                    &mut instructions,
                );
            }
            cursor = cursor.max(data.address.saturating_add(data.bytes.len() as u16));
        }
        if cursor < range.end {
            push_disassembled_range(
                output,
                cursor,
                range.end,
                &inline_jsr_data_lengths,
                &mut instructions,
            );
        }
    }
    instructions
}

fn push_disassembled_range(
    output: &CodegenOutput,
    start_address: u16,
    end_address: u16,
    inline_jsr_data_lengths: &BTreeMap<u16, usize>,
    instructions: &mut Vec<DisassembledInstruction>,
) {
    if end_address <= start_address {
        return;
    }
    let Some(start) = output_offset(output, start_address) else {
        return;
    };
    let Some(end) = output_end_offset(output, end_address) else {
        return;
    };
    instructions.extend(disassemble_with_origin_and_inline_jsr_data(
        output.bytes.get(start..end).unwrap_or_default(),
        start_address,
        |target| inline_jsr_data_lengths.get(&target).copied(),
    ));
}

fn inline_jsr_data_lengths(output: &CodegenOutput) -> BTreeMap<u16, usize> {
    output
        .map
        .routine_addresses
        .iter()
        // Action! r_Par consumes three inline parameter bytes after the JSR.
        .filter(|routine| routine.name.eq_ignore_ascii_case("r_Par"))
        .map(|routine| (routine.address, 3))
        .collect()
}

fn address_in_routine(output: &CodegenOutput, address: u16) -> bool {
    output
        .map
        .routine_ranges
        .iter()
        .any(|range| address >= range.start && address < range.end)
}

fn output_offset(output: &CodegenOutput, address: u16) -> Option<usize> {
    if address < output.origin {
        return None;
    }
    let offset = address.wrapping_sub(output.origin) as usize;
    (offset < output.bytes.len()).then_some(offset)
}

fn output_end_offset(output: &CodegenOutput, address: u16) -> Option<usize> {
    if address < output.origin {
        return None;
    }
    let offset = address.wrapping_sub(output.origin) as usize;
    (offset <= output.bytes.len()).then_some(offset)
}

fn push_source_comment(
    query: &MapQuery<'_>,
    address: u16,
    last_source: &mut Option<(usize, usize, u16, u16)>,
    lines: &mut Vec<String>,
) {
    let Some(source) = query.source_at(address) else {
        return;
    };
    let range = source.range;
    let key = (
        range.source_span.start,
        range.source_span.end,
        range.start,
        range.end,
    );
    if *last_source == Some(key) {
        return;
    }
    *last_source = Some(key);
    if let Some(location) = source.location {
        lines.push(format!(
            "; {}:{} {}{} | {}",
            location.line,
            location.column,
            format_source_range_kind(range.kind),
            range
                .name
                .as_ref()
                .map(|name| format!(" {name}"))
                .unwrap_or_default(),
            location.excerpt
        ));
    }
}

fn format_instruction_listing(
    instruction: &DisassembledInstruction,
    routine_labels: &BTreeMap<u16, String>,
) -> String {
    let raw = instruction
        .bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut line = format!(
        "{:04X}  {raw:<8}  {}",
        instruction.address, instruction.text
    );
    if instruction.mnemonic == "JSR"
        && let Some(target) = le_u16_from_slice(&instruction.operands)
        && let Some(label) = routine_labels.get(&target)
    {
        line.push_str(&format!("  ; {label}"));
    }
    line
}

fn format_data_listing(address: u16, bytes: &[u8]) -> String {
    let raw = bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    let values = bytes
        .iter()
        .map(|byte| format!("${byte:02X}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{address:04X}  {raw:<8}  .BYTE {values}")
}

fn generated_code_labels(items: &[DisassembledInstruction]) -> BTreeMap<u16, String> {
    let mut labels = BTreeMap::new();
    for item in items {
        if let Some(target) = instruction_target(item) {
            labels
                .entry(target)
                .or_insert_with(|| format!("L{target:04X}"));
        }
    }
    labels
}

fn routine_address_labels(output: &CodegenOutput) -> BTreeMap<u16, String> {
    output
        .map
        .routine_addresses
        .iter()
        .map(|routine| (routine.address, routine.name.clone()))
        .collect()
}

fn instruction_target(item: &DisassembledInstruction) -> Option<u16> {
    match item.mode? {
        AddressingMode::Relative => {
            let offset = *item.operands.first()? as i8;
            Some(
                item.address
                    .wrapping_add(2)
                    .wrapping_add_signed(i16::from(offset)),
            )
        }
        AddressingMode::Absolute | AddressingMode::AbsoluteX => {
            if matches!(item.mnemonic, "JMP" | "JSR") {
                le_u16_from_slice(&item.operands)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn le_u16_from_slice(bytes: &[u8]) -> Option<u16> {
    Some(u16::from(*bytes.first()?) | (u16::from(*bytes.get(1)?) << 8))
}

fn routine_boundary_comments(output: &CodegenOutput) -> BTreeMap<u16, Vec<String>> {
    let mut comments: BTreeMap<u16, Vec<String>> = BTreeMap::new();
    for routine in &output.map.routine_ranges {
        let entry = output
            .map
            .routine_addresses
            .iter()
            .find(|address| address.name.eq_ignore_ascii_case(&routine.name))
            .map(|address| address.address);
        comments
            .entry(routine.start)
            .or_default()
            .push(format_routine_start_comment(
                &routine.name,
                routine.start,
                routine.end,
                entry,
            ));
        comments
            .entry(routine.end)
            .or_default()
            .push(format!("; ===== END PROC {} =====", routine.name));
    }
    comments
}

fn format_routine_start_comment(name: &str, start: u16, end: u16, entry: Option<u16>) -> String {
    match entry {
        Some(entry) if entry != start => {
            format!("; ===== PROC {name} ${start:04X}..${end:04X} entry ${entry:04X} =====")
        }
        _ => format!("; ===== PROC {name} ${start:04X}..${end:04X} ====="),
    }
}

fn append_trailing_boundary_comments(
    output: &CodegenOutput,
    boundary_comments: &BTreeMap<u16, Vec<String>>,
    lines: &mut Vec<String>,
) {
    let end = output
        .origin
        .wrapping_add(u16::try_from(output.bytes.len()).unwrap_or(u16::MAX));
    if let Some(comments) = boundary_comments.get(&end) {
        lines.extend(comments.iter().cloned());
    }
}

fn format_source_range_kind(kind: CodegenSourceRangeKind) -> &'static str {
    match kind {
        CodegenSourceRangeKind::Routine => "routine",
        CodegenSourceRangeKind::Statement => "statement",
        CodegenSourceRangeKind::Expression => "expression",
        CodegenSourceRangeKind::Declaration => "declaration",
        CodegenSourceRangeKind::StorageInitializer => "storage",
        CodegenSourceRangeKind::MachineBlock => "machine",
    }
}
