//! DATA declarations enter NIR as mutable static images, never executable code.
use super::*;
use crate::semantic::ir::{
    SemDataAddressTarget, SemDataDeclaration, SemDataElement, SemDataInitializer, SemItem,
};

impl NirLowerer {
    pub(super) fn collect_data_ids(&mut self, program: &SemProgram) {
        for item in program.modules.iter().flat_map(|module| &module.items) {
            if let SemItem::Data(data) = item {
                self.data_ids_by_symbol
                    .insert(data.symbol.id, SymbolId(self.next_static));
                self.next_static += 1;
            }
        }
    }

    pub(super) fn lower_data(
        &self,
        data: &SemDataDeclaration,
        program: &SemProgram,
    ) -> NirStaticData {
        let SemDataInitializer::Literal(elements) = &data.initializer;
        let mut image = NirDataImage::default();
        for element in elements {
            let offset = ByteOffset::new(image.bytes.len() as u32);
            let width = match element {
                SemDataElement::Bytes(bytes) => {
                    image.bytes.extend(bytes);
                    continue;
                }
                SemDataElement::Integer { value, width } => {
                    image.fragments.push(NirDataFragment::Integer {
                        offset,
                        width: ByteSize::from(*width),
                        value: *value,
                    });
                    *width
                }
                SemDataElement::Address {
                    target,
                    selector,
                    addend,
                    width,
                } => {
                    let target = match target {
                        SemDataAddressTarget::Absolute(address) => {
                            NirDataAddressTarget::Absolute(*address)
                        }
                        SemDataAddressTarget::Symbol(target) => {
                            if let Some(id) = self.data_ids_by_symbol.get(target) {
                                NirDataAddressTarget::Static(*id)
                            } else if let Some(routine) = program
                                .modules
                                .iter()
                                .flat_map(|m| &m.items)
                                .find_map(|item| match item {
                                    SemItem::Routine(r) if r.symbol.id == *target => Some(r),
                                    _ => None,
                                })
                            {
                                NirDataAddressTarget::Routine(
                                    self.routine_ids[&storage_key(&routine.symbol.name)],
                                )
                            } else {
                                NirDataAddressTarget::Storage(NirStorageId::Global(
                                    self.global_ids_by_symbol[target],
                                ))
                            }
                        }
                    };
                    let encoding = match selector {
                        Some(selector) => NirDataAddressEncoding::TargetByte {
                            target: self.target_layout.target,
                            byte_index: match selector {
                                AddressByteSelector::Low => 0,
                                AddressByteSelector::High => 1,
                            },
                        },
                        None => NirDataAddressEncoding::Pointer {
                            address_space: data_address_target_space(target),
                            width: ByteSize::from(*width),
                        },
                    };
                    image.fragments.push(NirDataFragment::Address {
                        offset,
                        encoding,
                        target,
                        addend: i64::from(*addend),
                        span: data.span,
                    });
                    *width
                }
            };
            image
                .bytes
                .resize(image.bytes.len() + usize::from(width), 0);
        }
        NirStaticData {
            id: self.data_ids_by_symbol[&data.symbol.id],
            name: data.symbol.name.clone(),
            ty: NirType::from_value_with_layout(
                &ValueType::fund(FundType::Byte),
                self.target_layout,
            ),
            image,
            display: format!("DATA {}", data.symbol.name),
            alignment: ByteSize::ONE,
            mutable: true,
            section: "data".into(),
        }
    }
}
