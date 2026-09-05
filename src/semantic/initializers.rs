//! One canonical scalar-leaf walk for initializer validation and SemIR plans.
//! Padding is not a source element; inline arrays repeat their resolved stride.
use super::*;

pub(super) struct StaticInitializerLeaf {
    pub offset: u16,
    pub ty: ValueType,
    pub width: u16,
    pub path: String,
}

pub(super) fn static_initializer_leaves(
    ty: &ValueType,
    target: TargetLayout,
    fields: &[SemanticField],
    field_lookup: &HashMap<String, HashMap<String, FieldId>>,
) -> Option<Vec<StaticInitializerLeaf>> {
    struct Walker<'a> {
        target: TargetLayout,
        fields: &'a [SemanticField],
        lookup: &'a HashMap<String, HashMap<String, FieldId>>,
        active: HashSet<String>,
        leaves: Vec<StaticInitializerLeaf>,
    }

    impl Walker<'_> {
        fn append(&mut self, ty: &ValueType, offset: u16, path: String) -> Option<()> {
            if let Some(width) = ty.value_width_bytes_for_layout(self.target) {
                offset.checked_add(width)?;
                self.leaves.push(StaticInitializerLeaf {
                    offset,
                    ty: ty.clone(),
                    width,
                    path,
                });
                return Some(());
            }
            let key = normalize_name(ty.as_record_name()?);
            if !self.active.insert(key.clone()) {
                return None;
            }
            let mut fields = self
                .lookup
                .get(&key)?
                .values()
                .map(|id| self.fields.get(id.0))
                .collect::<Option<Vec<_>>>()?;
            fields.sort_by_key(|field| (field.offset, field.id.0));
            for field in fields {
                let field_offset = offset.checked_add(field.offset)?;
                let field_path = if path.is_empty() {
                    field.name.clone()
                } else {
                    format!("{path}.{}", field.name)
                };
                match &field.storage {
                    RecordFieldStorage::Value => {
                        self.append(&field.ty, field_offset, field_path)?
                    }
                    RecordFieldStorage::InlineArray { array_type, stride } => {
                        for index in 0..array_type.length? {
                            let element_offset =
                                field_offset.checked_add(index.checked_mul(*stride)?)?;
                            self.append(
                                &array_type.element,
                                element_offset,
                                format!("{field_path}({index})"),
                            )?;
                        }
                    }
                }
            }
            self.active.remove(&key);
            Some(())
        }
    }

    let mut walker = Walker {
        target,
        fields,
        lookup: field_lookup,
        active: HashSet::new(),
        leaves: Vec::new(),
    };
    walker.append(ty, 0, String::new())?;
    Some(walker.leaves)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::tokenize;
    use crate::parser::parse;

    #[test]
    fn embedded_initializer_leaves_follow_canonical_alignment_and_paths() {
        let source = "TYPE Point=[BYTE tag CARD word] \
                      TYPE Packet=[BYTE lead Point ARRAY points(2) INT ARRAY words(3) BYTE tail] \
                      Packet data=[1 2 3 4 5 6 7 8 9] PROC Main() RETURN";
        for (target, offsets, extent) in [
            (TargetId::Atari6502, [0, 1, 2, 4, 5, 7, 9, 11, 13], 14),
            (TargetId::Wdc65816Small, [0, 2, 4, 6, 8, 10, 12, 14, 16], 18),
            (
                TargetId::Wdc65816Native,
                [0, 2, 4, 6, 8, 10, 12, 14, 16],
                18,
            ),
            (TargetId::Motorola68000, [0, 2, 4, 6, 8, 10, 12, 14, 16], 18),
        ] {
            let ast = parse(&tokenize(source).unwrap()).unwrap();
            let model = analyze_with_options(
                &ast,
                SemanticOptions {
                    embedded_record_arrays: true,
                    ..SemanticOptions::modern().with_target(target)
                },
            )
            .unwrap();
            let semir = ir::lower_program(&ast, &model);
            let declaration = semir.modules[0]
                .items
                .iter()
                .find_map(|item| match item {
                    ir::SemItem::Declaration(decl) if decl.symbol.name == "data" => Some(decl),
                    _ => None,
                })
                .unwrap();
            let plan = declaration.static_initializer.as_ref().unwrap();
            assert_eq!(plan.initialized_extent, extent, "{target:?}");
            assert_eq!(
                plan.writes.iter().map(|w| w.offset).collect::<Vec<_>>(),
                offsets
            );
            assert_eq!(
                plan.writes
                    .iter()
                    .map(|w| w.display_path.as_str())
                    .collect::<Vec<_>>(),
                [
                    "data.lead",
                    "data.points(0).tag",
                    "data.points(0).word",
                    "data.points(1).tag",
                    "data.points(1).word",
                    "data.words(0)",
                    "data.words(1)",
                    "data.words(2)",
                    "data.tail",
                ]
            );
        }
    }

    #[test]
    fn embedded_initializer_validation_uses_expanded_leaf_types() {
        for (source, expected) in [
            (
                "TYPE R=[BYTE ARRAY values(2)] R data=[1 2 3]",
                "too many initializer elements",
            ),
            (
                "TYPE R=[REAL ARRAY values(2) BYTE tail] R data=[1.0 2.0 3.0]",
                "REAL initializer element requires a REAL scalar leaf",
            ),
            (
                "BYTE target TYPE R=[CARD ARRAY addresses(2) BYTE tail] R data=[@target @target @target]",
                "requires a 2-byte scalar leaf",
            ),
        ] {
            let ast = parse(&tokenize(source).unwrap()).unwrap();
            let diagnostics = analyze_with_options(
                &ast,
                SemanticOptions {
                    embedded_record_arrays: true,
                    ..SemanticOptions::modern()
                },
            )
            .unwrap_err();
            assert!(
                diagnostics.iter().any(|d| d.message.contains(expected)),
                "{source}: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn initializer_extent_overflow_is_diagnosed_before_a_plan_can_be_lost() {
        for (declaration, count) in [
            ("CARD ARRAY data", 32768),
            ("TYPE Pair=[REAL ARRAY values(2)] Pair ARRAY data", 10923),
        ] {
            let values = std::iter::repeat_n("1", count)
                .collect::<Vec<_>>()
                .join(" ");
            let source = format!("{declaration}=[{values}] PROC Main() RETURN");
            let ast = parse(&tokenize(&source).unwrap()).unwrap();
            let errors = analyze_with_options(&ast, SemanticOptions::modern()).unwrap_err();
            assert!(
                errors
                    .iter()
                    .any(|error| error.message.contains("static initializer storage extent")),
                "{errors:?}"
            );
        }
    }
}
