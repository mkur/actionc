use crate::codegen::{
    CodegenMap, CodegenSourceRange, CodegenStorageSymbol, RoutineRange, SkippedRange,
};

#[derive(Debug, Clone, Copy)]
pub struct MapQuery<'a> {
    map: &'a CodegenMap,
    source_text: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressOwnership<'a> {
    pub address: u16,
    pub storage: Option<StorageOwner<'a>>,
    pub routine: Option<&'a RoutineRange>,
    pub skipped: Option<&'a SkippedRange>,
    pub source: Option<SourceMatch<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageOwner<'a> {
    pub symbol: &'a CodegenStorageSymbol,
    pub offset: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMatch<'a> {
    pub range: &'a CodegenSourceRange,
    pub location: Option<SourceLocation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    pub line: usize,
    pub column: usize,
    pub excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolMatch<'a> {
    Routine(&'a RoutineRange),
    Storage(&'a CodegenStorageSymbol),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeOverlap<'a> {
    pub start: u16,
    pub end: u16,
    pub item: RangeItem<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeItem<'a> {
    Routine(&'a RoutineRange),
    Storage(&'a CodegenStorageSymbol),
    Skipped(&'a SkippedRange),
    Source(&'a CodegenSourceRange),
}

impl<'a> MapQuery<'a> {
    pub fn new(map: &'a CodegenMap) -> Self {
        Self {
            map,
            source_text: None,
        }
    }

    pub fn with_source(map: &'a CodegenMap, source_text: &'a str) -> Self {
        Self {
            map,
            source_text: Some(source_text),
        }
    }

    pub fn owner(&self, address: u16) -> AddressOwnership<'a> {
        AddressOwnership {
            address,
            storage: self.storage_owner(address),
            routine: self.routine_at(address),
            skipped: self.skipped_at(address),
            source: self.source_at(address),
        }
    }

    pub fn storage_owner(&self, address: u16) -> Option<StorageOwner<'a>> {
        self.map
            .storage_symbols
            .iter()
            .filter(|symbol| contains_address(symbol.address, symbol.size, address))
            .min_by_key(|symbol| symbol.size)
            .map(|symbol| StorageOwner {
                symbol,
                offset: address.wrapping_sub(symbol.address),
            })
    }

    pub fn routine_at(&self, address: u16) -> Option<&'a RoutineRange> {
        self.map
            .routine_ranges
            .iter()
            .filter(|routine| routine.start <= address && address < routine.end)
            .min_by_key(|routine| routine.end.wrapping_sub(routine.start))
    }

    pub fn skipped_at(&self, address: u16) -> Option<&'a SkippedRange> {
        self.map
            .skipped_ranges
            .iter()
            .find(|range| contains_address(range.start, range.len, address))
    }

    pub fn source_at(&self, address: u16) -> Option<SourceMatch<'a>> {
        let range = self
            .map
            .source_ranges
            .iter()
            .filter(|range| range.start <= address && address < range.end)
            .min_by_key(|range| range.end.wrapping_sub(range.start))?;
        Some(SourceMatch {
            range,
            location: self
                .source_text
                .map(|source_text| source_location(source_text, range.source_span.start)),
        })
    }

    pub fn symbol(&self, name: &str) -> Vec<SymbolMatch<'a>> {
        let mut matches = Vec::new();
        matches.extend(
            self.map
                .routine_ranges
                .iter()
                .filter(|routine| names_match(&routine.name, name))
                .map(SymbolMatch::Routine),
        );
        matches.extend(
            self.map
                .storage_symbols
                .iter()
                .filter(|symbol| names_match(&symbol.name, name))
                .map(SymbolMatch::Storage),
        );
        matches
    }

    pub fn routine(&self, name: &str) -> Option<&'a RoutineRange> {
        self.map
            .routine_ranges
            .iter()
            .find(|routine| names_match(&routine.name, name))
    }

    pub fn range(&self, start: u16, end: u16) -> Vec<RangeOverlap<'a>> {
        if end <= start {
            return Vec::new();
        }

        let mut overlaps = Vec::new();
        overlaps.extend(
            self.map
                .routine_ranges
                .iter()
                .filter(|routine| ranges_overlap(start, end, routine.start, routine.end))
                .map(|routine| RangeOverlap {
                    start: routine.start,
                    end: routine.end,
                    item: RangeItem::Routine(routine),
                }),
        );
        overlaps.extend(
            self.map
                .storage_symbols
                .iter()
                .filter(|symbol| ranges_overlap_sized(start, end, symbol.address, symbol.size))
                .map(|symbol| RangeOverlap {
                    start: symbol.address,
                    end: sized_end(symbol.address, symbol.size),
                    item: RangeItem::Storage(symbol),
                }),
        );
        overlaps.extend(
            self.map
                .skipped_ranges
                .iter()
                .filter(|range| ranges_overlap_sized(start, end, range.start, range.len))
                .map(|range| RangeOverlap {
                    start: range.start,
                    end: sized_end(range.start, range.len),
                    item: RangeItem::Skipped(range),
                }),
        );
        overlaps.extend(
            self.map
                .source_ranges
                .iter()
                .filter(|range| ranges_overlap(start, end, range.start, range.end))
                .map(|range| RangeOverlap {
                    start: range.start,
                    end: range.end,
                    item: RangeItem::Source(range),
                }),
        );
        overlaps.sort_by(|left, right| {
            left.start
                .cmp(&right.start)
                .then_with(|| range_item_order(&left.item).cmp(&range_item_order(&right.item)))
                .then_with(|| left.end.cmp(&right.end))
        });
        overlaps
    }
}

pub fn source_location(source: &str, offset: usize) -> SourceLocation {
    let offset = floor_char_boundary(source, offset.min(source.len()));
    let mut line = 1usize;
    let mut line_start = 0usize;
    for (index, ch) in source.char_indices() {
        if index >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            line_start = index + ch.len_utf8();
        }
    }
    let line_end = source[line_start..]
        .find('\n')
        .map(|relative| line_start + relative)
        .unwrap_or(source.len());
    let column = source[line_start..offset].chars().count() + 1;
    let excerpt = source[line_start..line_end].trim().to_string();
    SourceLocation {
        line,
        column,
        excerpt,
    }
}

fn contains_address(start: u16, len: u16, address: u16) -> bool {
    let start = u32::from(start);
    let end = start + u32::from(len);
    let address = u32::from(address);
    start <= address && address < end
}

fn ranges_overlap_sized(left_start: u16, left_end: u16, right_start: u16, right_len: u16) -> bool {
    ranges_overlap(
        left_start,
        left_end,
        right_start,
        sized_end(right_start, right_len),
    )
}

fn ranges_overlap(left_start: u16, left_end: u16, right_start: u16, right_end: u16) -> bool {
    u32::from(left_start) < u32::from(right_end) && u32::from(right_start) < u32::from(left_end)
}

fn sized_end(start: u16, len: u16) -> u16 {
    start.wrapping_add(len)
}

fn names_match(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn range_item_order(item: &RangeItem<'_>) -> u8 {
    match item {
        RangeItem::Storage(_) => 0,
        RangeItem::Routine(_) => 1,
        RangeItem::Skipped(_) => 2,
        RangeItem::Source(_) => 3,
    }
}

fn floor_char_boundary(source: &str, offset: usize) -> usize {
    if source.is_char_boundary(offset) {
        return offset;
    }
    source
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index < offset)
        .last()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::{
        CodegenAddressSpace, CodegenSourceRangeKind, CodegenSymbolKind, CodegenSymbolScope,
    };
    use crate::source::Span;

    fn sample_map() -> CodegenMap {
        CodegenMap {
            origin: 0x3000,
            run_address: 0x3004,
            skipped_ranges: vec![SkippedRange {
                start: 0x3020,
                len: 0x0010,
            }],
            routine_addresses: Vec::new(),
            routine_ranges: vec![RoutineRange {
                name: "Main".to_string(),
                start: 0x3004,
                end: 0x3010,
            }],
            routine_signatures: Vec::new(),
            storage_symbols: vec![
                storage_symbol(
                    "A",
                    CodegenSymbolScope::Global,
                    CodegenSymbolKind::Storage,
                    0x3000,
                    1,
                ),
                storage_symbol(
                    "L",
                    CodegenSymbolScope::Routine("Main".to_string()),
                    CodegenSymbolKind::Local,
                    0x3004,
                    2,
                ),
            ],
            source_ranges: vec![
                CodegenSourceRange {
                    kind: CodegenSourceRangeKind::Routine,
                    name: Some("Main".to_string()),
                    source_span: Span::new(7, 28),
                    start: 0x3004,
                    end: 0x3010,
                },
                CodegenSourceRange {
                    kind: CodegenSourceRangeKind::Statement,
                    name: Some("assignment".to_string()),
                    source_span: Span::new(19, 22),
                    start: 0x3008,
                    end: 0x300B,
                },
            ],
            routine_effects: Vec::new(),
            machine_blocks: Vec::new(),
            optimizations: Vec::new(),
            proofs: Vec::new(),
            proof_attempts: Vec::new(),
        }
    }

    fn storage_symbol(
        name: &str,
        scope: CodegenSymbolScope,
        kind: CodegenSymbolKind,
        address: u16,
        size: u16,
    ) -> CodegenStorageSymbol {
        CodegenStorageSymbol {
            name: name.to_string(),
            scope,
            kind,
            address,
            size,
            address_space: CodegenAddressSpace::Absolute,
            pointee_size: None,
            array: None,
            signed: false,
        }
    }

    #[test]
    fn owner_reports_storage_routine_and_source() {
        let map = sample_map();
        let query = MapQuery::with_source(&map, "BYTE A\nPROC Main()\nA=1\nRETURN");

        let owner = query.owner(0x3008);

        assert!(matches!(owner.routine, Some(routine) if routine.name == "Main"));
        assert!(matches!(
            owner.source,
            Some(SourceMatch {
                range: CodegenSourceRange {
                    kind: CodegenSourceRangeKind::Statement,
                    ..
                },
                ..
            })
        ));
        assert!(owner.storage.is_none());
    }

    #[test]
    fn storage_owner_reports_offset() {
        let map = sample_map();
        let query = MapQuery::new(&map);

        let owner = query.storage_owner(0x3005).unwrap();

        assert_eq!(owner.symbol.name, "L");
        assert_eq!(owner.offset, 1);
    }

    #[test]
    fn symbol_lookup_is_case_insensitive() {
        let map = sample_map();
        let query = MapQuery::new(&map);

        assert!(matches!(query.symbol("main")[0], SymbolMatch::Routine(_)));
        assert!(matches!(query.symbol("a")[0], SymbolMatch::Storage(_)));
    }

    #[test]
    fn range_reports_all_overlaps_sorted_by_address() {
        let map = sample_map();
        let query = MapQuery::new(&map);

        let overlaps = query.range(0x3000, 0x3009);

        assert!(matches!(overlaps[0].item, RangeItem::Storage(symbol) if symbol.name == "A"));
        assert!(overlaps.iter().any(|overlap| matches!(
            overlap.item,
            RangeItem::Routine(routine) if routine.name == "Main"
        )));
        assert!(overlaps.iter().any(|overlap| matches!(
            overlap.item,
            RangeItem::Source(range) if range.kind == CodegenSourceRangeKind::Statement
        )));
    }

    #[test]
    fn source_location_reports_line_column_and_excerpt() {
        let location = source_location("ONE\nTWO\nTHREE", 4);

        assert_eq!(location.line, 2);
        assert_eq!(location.column, 1);
        assert_eq!(location.excerpt, "TWO");
    }
}
