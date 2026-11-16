//! Classic compatibility projection: DATA is an initialized inline byte array.
//! Its source semantics and every relocation were resolved in SemIR.
use super::*;

impl SemIrAstLowerer<'_> {
    pub(super) fn data_declaration(&mut self, data: &SemDataDeclaration) -> Option<Decl> {
        let SemDataInitializer::Literal(elements) = &data.initializer;
        let mut initializers = Vec::new();
        let mut extent = 0u32;
        for element in elements {
            match element {
                SemDataElement::Bytes(bytes) => {
                    extent += bytes.len() as u32;
                    initializers.extend(bytes.iter().copied().map(StorageInit::Byte));
                }
                SemDataElement::Integer { value, width } => {
                    extent += u32::from(*width);
                    initializers
                        .extend((0..*width).map(|i| StorageInit::Byte((value >> (8 * i)) as u8)));
                }
                SemDataElement::Address {
                    target,
                    selector,
                    addend,
                    width,
                } => {
                    let kind = match selector {
                        Some(AddressByteSelector::Low) => StorageRelocationKind::Low8,
                        Some(AddressByteSelector::High) => StorageRelocationKind::High8,
                        None => StorageRelocationKind::Word16,
                    };
                    if kind.width() != u16::from(*width) {
                        self.unsupported(data.span, "non-6502 DATA address width");
                        return None;
                    }
                    let target = match target {
                        SemDataAddressTarget::Symbol(id) => {
                            if let Some(address) = self.external_addresses.and_then(|a| a.get(id)) {
                                StorageRelocationTarget::Absolute(*address)
                            } else {
                                StorageRelocationTarget::Name(
                                    self.projection_names
                                        .get(id)
                                        .expect("resolved DATA target has a projected declaration")
                                        .clone(),
                                )
                            }
                        }
                        SemDataAddressTarget::Absolute(address) => {
                            let Ok(address) = u16::try_from(address.value) else {
                                self.unsupported(data.span, "non-6502 DATA address");
                                return None;
                            };
                            StorageRelocationTarget::Absolute(address)
                        }
                    };
                    initializers.push(StorageInit::Relocation {
                        kind,
                        target,
                        addend: *addend,
                        span: data.span,
                    });
                    extent += u32::from(*width);
                }
            }
        }
        let Ok(initialized_extent) = u16::try_from(extent) else {
            self.unsupported(data.span, "DATA larger than 65535 bytes");
            return None;
        };
        self.static_initializers.insert(
            data.span,
            ClassicStaticInitializer {
                initialized_extent,
                initializers,
            },
        );
        let text = extent.to_string();
        Some(Decl::Var(VarDecl {
            visibility: Visibility::Private,
            qualifiers: VarQualifiers::default(),
            ty: TypeRef {
                base: TypeBase::Fund(FundType::Byte),
                pointer: false,
            },
            storage: VarStorage::Array,
            entries: vec![DeclEntry {
                name: self.symbol_name(&data.symbol),
                dimensions: vec![],
                size: Some(Expr {
                    kind: ExprKind::Number(NumberLiteral {
                        text: text.clone(),
                        kind: crate::lexer::NumberKind::Card,
                        value: Some(u64::from(extent)),
                    }),
                    text,
                    span: data.span,
                }),
                initializer: None,
                span: data.span,
            }],
            span: data.span,
        }))
    }
}
