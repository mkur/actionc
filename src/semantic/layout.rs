use std::collections::{HashMap, HashSet};

use crate::source::Span;

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
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticRecordFieldLayout {
    pub id: FieldId,
    pub name: String,
    pub ty: ValueType,
    pub offset: u16,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticArrayLayout {
    pub id: ArrayLayoutId,
    pub symbol: SymbolId,
    pub name: String,
    pub element_type: ValueType,
    pub pointer_type: ValueType,
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
        fields: &[SemanticField],
    ) -> Self {
        let mut facts = Self::default();
        facts.collect_records(symbols, fields);
        facts.collect_arrays(symbols, array_symbols);
        facts
    }

    pub fn record_for_owner(&self, owner: SymbolId) -> Option<&SemanticRecordLayout> {
        self.record_lookup
            .get(&owner)
            .and_then(|id| self.records.get(id.0))
    }

    pub fn array_for_symbol(&self, symbol: SymbolId) -> Option<&SemanticArrayLayout> {
        self.array_lookup
            .get(&symbol)
            .and_then(|id| self.arrays.get(id.0))
    }

    fn collect_records(&mut self, symbols: &SymbolTable, fields: &[SemanticField]) {
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
                .map(|record| (record.name.clone(), record.size))
                .collect::<HashMap<_, _>>();
            let size = owner_fields.iter().fold(0u16, |size, field| {
                let width = semantic_value_width(&field.ty, &known_records).unwrap_or(0);
                size.max(field.offset.saturating_add(width))
            });
            let id = RecordLayoutId(self.records.len());
            self.record_lookup.insert(owner, id);
            let record_type = RecordType::new(
                symbol.name.clone(),
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
                name: symbol.name.clone(),
                record_type,
                fields: owner_fields
                    .iter()
                    .map(|field| SemanticRecordFieldLayout {
                        id: field.id,
                        name: field.name.clone(),
                        ty: field.ty.clone(),
                        offset: field.offset,
                        span: field.span,
                    })
                    .collect(),
                size,
                span: symbol.span,
            });
        }
    }

    fn collect_arrays(&mut self, symbols: &SymbolTable, array_symbols: &HashSet<SymbolId>) {
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
            self.array_lookup.insert(symbol_id, id);
            self.arrays.push(SemanticArrayLayout {
                id,
                symbol: symbol_id,
                name: symbol.name.clone(),
                pointer_type: ValueType::pointer_to(element_type.clone()),
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
    if symbols
        .symbols
        .get(symbol_id.0)
        .is_some_and(|symbol| symbol.scope == symbols.global_scope())
    {
        return SemanticArrayOrigin::Global;
    }
    if matches!(symbol_class, SymbolClass::Array) {
        return SemanticArrayOrigin::Local;
    }
    SemanticArrayOrigin::Unknown
}

fn semantic_value_width(value: &ValueType, record_sizes: &HashMap<String, u16>) -> Option<u16> {
    if value.pointer {
        Some(2)
    } else {
        value.scalar_width_bytes().or_else(|| {
            value
                .as_record_name()
                .and_then(|name| record_sizes.get(name).copied())
        })
    }
}
