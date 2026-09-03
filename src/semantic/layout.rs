use std::collections::{HashMap, HashSet};

use crate::source::Span;
use crate::target::{RecordLayoutPolicy, TargetLayout};

use super::{
    FieldId, RecordFieldType, RecordType, SemanticField, SymbolClass, SymbolId, SymbolTable,
    ValueType,
};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SemanticLayoutFacts {
    pub records: Vec<SemanticRecordLayout>,
    pub record_lookup: HashMap<SymbolId, RecordLayoutId>,
    pub arrays: Vec<SemanticArrayLayout>,
    pub array_lookup: HashMap<SymbolId, ArrayLayoutId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RecordLayoutId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArrayLayoutId(pub usize);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticRecordLayout {
    pub id: RecordLayoutId,
    pub owner: SymbolId,
    pub name: String,
    pub record_type: RecordType,
    pub fields: Vec<SemanticRecordFieldLayout>,
    pub size: u16,
    pub alignment: u16,
    pub tail_padding: u16,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticRecordFieldLayout {
    pub id: FieldId,
    pub name: String,
    pub ty: ValueType,
    pub offset: u16,
    pub alignment: u16,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticArrayLayout {
    pub id: ArrayLayoutId,
    pub symbol: SymbolId,
    pub name: String,
    pub element_type: ValueType,
    pub length: Option<u16>,
    pub pointer_type: ValueType,
    pub element_size: u16,
    pub element_alignment: u16,
    pub stride: u16,
    pub storage_size: Option<u32>,
    pub origin: SemanticArrayOrigin,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SemanticArrayOrigin {
    Global,
    Local,
    Parameter,
    Unknown,
}

impl SemanticLayoutFacts {
    pub fn build(
        symbols: &SymbolTable,
        array_symbols: &HashSet<SymbolId>,
        array_lengths: &HashMap<SymbolId, u16>,
        fields: &[SemanticField],
        target_layout: TargetLayout,
    ) -> Self {
        let mut facts = Self::default();
        facts.collect_records(symbols, fields, target_layout);
        facts.collect_arrays(symbols, array_symbols, array_lengths, target_layout);
        facts
    }

    pub fn record_for_owner(&self, owner: SymbolId) -> Option<&SemanticRecordLayout> {
        self.record_lookup
            .get(&owner)
            .and_then(|id| self.records.get(id.0))
    }

    pub fn record_for_name(&self, name: &str) -> Option<&SemanticRecordLayout> {
        self.records
            .iter()
            .find(|record| record.name.eq_ignore_ascii_case(name))
    }

    pub fn array_for_symbol(&self, symbol: SymbolId) -> Option<&SemanticArrayLayout> {
        self.array_lookup
            .get(&symbol)
            .and_then(|id| self.arrays.get(id.0))
    }

    fn collect_records(
        &mut self,
        symbols: &SymbolTable,
        fields: &[SemanticField],
        target_layout: TargetLayout,
    ) {
        let mut fields_by_owner: HashMap<SymbolId, Vec<&SemanticField>> = HashMap::new();
        for field in fields {
            fields_by_owner.entry(field.owner).or_default().push(field);
        }

        let mut owners: Vec<_> = fields_by_owner.keys().copied().collect();
        owners.sort_by_key(|owner| owner.0);
        for owner in owners {
            let Some(symbol) = symbols.symbols.get(owner.0) else {
                continue;
            };
            let Some(mut owner_fields) = fields_by_owner.remove(&owner) else {
                continue;
            };
            owner_fields.sort_by_key(|field| (field.offset, field.id.0));
            let known_records = self
                .records
                .iter()
                .map(|record| (record.name.clone(), (record.size, record.alignment)))
                .collect::<HashMap<_, _>>();
            let alignment = owner_fields
                .iter()
                .filter_map(|field| semantic_value_alignment(&field.ty, &known_records, target_layout))
                .max()
                .unwrap_or(1);
            let unpadded_size = owner_fields.iter().fold(0u16, |size, field| {
                let width = semantic_value_width(&field.ty, &known_records, target_layout)
                    .unwrap_or(0);
                size.max(field.offset.saturating_add(width))
            });
            let size = align_up(unpadded_size, alignment).unwrap_or(unpadded_size);
            let tail_padding = size.saturating_sub(unpadded_size);
            let id = RecordLayoutId(self.records.len());
            self.record_lookup.insert(owner, id);
            let record_name = symbol.qualified_name.clone();
            let record_type = RecordType::new(
                record_name.clone(),
                owner_fields.iter().map(|field| RecordFieldType {
                    id: Some(field.id),
                    name: field.name.clone(),
                    ty: field.ty.clone(),
                    offset: field.offset,
                }),
                size,
            );
            self.records.push(SemanticRecordLayout {
                id,
                owner,
                name: record_name,
                record_type,
                fields: owner_fields
                    .iter()
                    .map(|field| SemanticRecordFieldLayout {
                        id: field.id,
                        name: field.name.clone(),
                        ty: field.ty.clone(),
                        offset: field.offset,
                        alignment: semantic_value_alignment(
                            &field.ty,
                            &known_records,
                            target_layout,
                        )
                        .unwrap_or(1),
                        span: field.span,
                    })
                    .collect(),
                size,
                alignment,
                tail_padding,
                span: symbol.span,
            });
        }
    }

    fn collect_arrays(
        &mut self,
        symbols: &SymbolTable,
        array_symbols: &HashSet<SymbolId>,
        array_lengths: &HashMap<SymbolId, u16>,
        target_layout: TargetLayout,
    ) {
        let mut ids: Vec<_> = array_symbols.iter().copied().collect();
        ids.sort_by_key(|id| id.0);
        for symbol_id in ids {
            let Some(symbol) = symbols.symbols.get(symbol_id.0) else {
                continue;
            };
            let Some(element_type) = symbol.ty.clone() else {
                continue;
            };
            let id = ArrayLayoutId(self.arrays.len());
            let records = self
                .records
                .iter()
                .map(|record| (record.name.clone(), (record.size, record.alignment)))
                .collect::<HashMap<_, _>>();
            let element_size = semantic_value_width(&element_type, &records, target_layout)
                .unwrap_or(0);
            let element_alignment =
                semantic_value_alignment(&element_type, &records, target_layout).unwrap_or(1);
            let stride = align_up(element_size, element_alignment).unwrap_or(element_size);
            let length = array_lengths.get(&symbol_id).copied();
            self.array_lookup.insert(symbol_id, id);
            self.arrays.push(SemanticArrayLayout {
                id,
                symbol: symbol_id,
                name: symbol.name.clone(),
                length,
                pointer_type: ValueType::pointer_to(element_type.clone()),
                element_size,
                element_alignment,
                stride,
                storage_size: length.map(|length| u32::from(length) * u32::from(stride)),
                element_type,
                origin: array_origin(symbols, symbol_id, &symbol.class),
                span: symbol.span,
            });
        }
    }
}

fn array_origin(
    symbols: &SymbolTable,
    symbol_id: SymbolId,
    symbol_class: &SymbolClass,
) -> SemanticArrayOrigin {
    if matches!(symbol_class, SymbolClass::Param) {
        return SemanticArrayOrigin::Parameter;
    }
    if symbols.symbols.get(symbol_id.0).is_some_and(|symbol| {
        symbol.scope == symbols.global_scope()
            || matches!(
                symbols.scopes.get(symbol.scope.0).map(|scope| scope.kind),
                Some(super::ScopeKind::Module(_))
            )
    }) {
        return SemanticArrayOrigin::Global;
    }
    if matches!(symbol_class, SymbolClass::Array) {
        return SemanticArrayOrigin::Local;
    }
    SemanticArrayOrigin::Unknown
}

fn semantic_value_width(
    value: &ValueType,
    records: &HashMap<String, (u16, u16)>,
    target_layout: TargetLayout,
) -> Option<u16> {
    value.value_width_bytes_for_layout(target_layout).or_else(|| {
        value
            .as_record_name()
            .and_then(|name| records.get(name).map(|(size, _)| *size))
    })
}

fn semantic_value_alignment(
    value: &ValueType,
    records: &HashMap<String, (u16, u16)>,
    target_layout: TargetLayout,
) -> Option<u16> {
    if target_layout.record_layout == RecordLayoutPolicy::Packed {
        return semantic_value_width(value, records, target_layout).map(|_| 1);
    }
    match value.kind() {
        super::ValueTypeKind::Scalar(scalar) => Some(
            scalar
                .width_bytes()
                .min(u16::from(target_layout.natural_word_alignment_bytes)),
        ),
        super::ValueTypeKind::Real => {
            Some(u16::from(target_layout.natural_word_alignment_bytes))
        }
        super::ValueTypeKind::Pointer(_) => {
            u16::try_from(target_layout.data_pointer.alignment_bytes.get()).ok()
        }
        super::ValueTypeKind::CallablePointer(_) => {
            u16::try_from(target_layout.code_pointer.alignment_bytes.get()).ok()
        }
        super::ValueTypeKind::Record(name) => records.get(&name).map(|(_, alignment)| *alignment),
        super::ValueTypeKind::Error => None,
    }
}

fn align_up(value: u16, alignment: u16) -> Option<u16> {
    if alignment <= 1 {
        return Some(value);
    }
    value
        .checked_add(alignment - 1)
        .map(|value| value / alignment * alignment)
}
