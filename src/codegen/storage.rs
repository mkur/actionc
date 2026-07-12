use super::*;

// Extracted from src/codegen.rs: record layouts
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct RecordLayouts {
    pub(super) by_name: HashMap<String, usize>,
    pub(super) layouts: Vec<RecordLayout>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordLayout {
    pub(super) size: u16,
    pub(super) fields: HashMap<String, RecordField>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RecordField {
    pub(super) offset: u16,
    pub(super) size: u16,
    pub(super) signed: bool,
}

pub(super) fn record_field_fits_indirect_y(field: RecordField) -> bool {
    field
        .offset
        .checked_add(field.size.saturating_sub(1))
        .is_some_and(|last| last <= u8::MAX as u16)
}

// Extracted from src/codegen.rs: storage slots
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StorageSlot {
    pub(super) address: u16,
    pub(super) size: u16,
    pub(super) space: AddressSpace,
    pub(super) index_offset: u16,
    pub(super) pointee_size: Option<u16>,
    pub(super) array: Option<ArrayStorage>,
    pub(super) record: Option<usize>,
    pub(super) signed: bool,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct EffectiveAddress {
    pub(super) base: StorageSlot,
    pub(super) index: StorageSlot,
    pub(super) pointer: ZeroPage,
    pub(super) element_size: u16,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum EffectiveAddressStoreSource {
    Constant(u16),
    Slot(StorageSlot),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StorageLayout {
    pub(super) symbols: HashMap<String, StorageSlot>,
    pub(super) machine_caret_values: HashMap<String, u16>,
    pub(super) machine_symbol_addresses: HashMap<String, MachineSymbolAddress>,
    // Large fixed-address arrays have a physical compatibility descriptor in
    // output storage, but their source-level address value is the fixed base.
    pub(super) absolute_array_value_addresses: HashMap<String, u16>,
    pub(super) next_address: u16,
    pub(super) storage_size: u16,
    pub(super) initializers: Vec<StorageInit>,
    pub(super) array_backings: Vec<ArrayBacking>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum StorageInit {
    Byte(u8),
    LabelWord(String),
    Skip(SkippedRange),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ArrayBacking {
    pub(super) label: String,
    pub(super) size: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MachineSymbolAddress {
    Absolute(u16),
    Label(String),
}

// Extracted from src/codegen.rs: array address spaces
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ArrayStorage {
    Inline,
    Pointer,
    Descriptor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AddressSpace {
    Absolute,
    AbsoluteX,
    ZeroPage,
    IndirectIndexedY,
}

impl From<AddressSpace> for CodegenAddressSpace {
    fn from(value: AddressSpace) -> Self {
        match value {
            AddressSpace::Absolute => Self::Absolute,
            AddressSpace::AbsoluteX => Self::AbsoluteX,
            AddressSpace::ZeroPage => Self::ZeroPage,
            AddressSpace::IndirectIndexedY => Self::IndirectIndexedY,
        }
    }
}

impl From<ArrayStorage> for CodegenArrayStorage {
    fn from(value: ArrayStorage) -> Self {
        match value {
            ArrayStorage::Inline => Self::Inline,
            ArrayStorage::Pointer => Self::Pointer,
            ArrayStorage::Descriptor => Self::Descriptor,
        }
    }
}

pub(super) fn codegen_storage_symbol(
    name: String,
    scope: CodegenSymbolScope,
    kind: CodegenSymbolKind,
    slot: StorageSlot,
) -> CodegenStorageSymbol {
    CodegenStorageSymbol {
        name,
        scope,
        kind,
        address: slot.address,
        size: slot.size,
        address_space: slot.space.into(),
        pointee_size: slot.pointee_size,
        array: slot.array.map(Into::into),
        signed: slot.signed,
    }
}

impl RecordLayouts {
    pub(super) fn get(&self, name: &str) -> Option<(usize, &RecordLayout)> {
        let id = *self.by_name.get(&normalize_name(name))?;
        Some((id, &self.layouts[id]))
    }

    pub(super) fn field(&self, record: usize, field: &str) -> Option<RecordField> {
        self.layouts
            .get(record)?
            .fields
            .get(&normalize_name(field))
            .copied()
    }
}

impl StorageLayout {
    pub(super) fn empty(storage_base: u16) -> Self {
        Self {
            symbols: HashMap::new(),
            machine_caret_values: HashMap::new(),
            machine_symbol_addresses: HashMap::new(),
            absolute_array_value_addresses: HashMap::new(),
            next_address: storage_base,
            storage_size: 0,
            initializers: Vec::new(),
            array_backings: Vec::new(),
        }
    }

    pub(super) fn codegen_storage_symbols(&self) -> Vec<CodegenStorageSymbol> {
        let mut symbols = self
            .symbols
            .iter()
            .map(|(name, slot)| {
                codegen_storage_symbol(
                    name.clone(),
                    CodegenSymbolScope::Global,
                    CodegenSymbolKind::Storage,
                    *slot,
                )
            })
            .collect::<Vec<_>>();
        symbols.sort_by(|left, right| left.name.cmp(&right.name));
        symbols
    }

    pub(super) fn from_program(
        program: &Program,
        storage_base: u16,
        compatible: bool,
        record_layouts: &RecordLayouts,
        numeric_defines: &HashMap<String, u16>,
    ) -> Self {
        let mut layout = Self::empty(storage_base);

        for module in &program.modules {
            for item in &module.items {
                if let Item::Declaration(Decl::Var(decl)) = item {
                    layout.add_var_decl(decl, compatible, record_layouts, numeric_defines);
                }
            }
        }

        layout
    }

    pub(super) fn add_var_decl(
        &mut self,
        decl: &VarDecl,
        compatible: bool,
        record_layouts: &RecordLayouts,
        numeric_defines: &HashMap<String, u16>,
    ) {
        let Some(element_size) = type_size_with_records(&decl.ty, record_layouts) else {
            return;
        };
        let Some(slot_size) = storage_size_with_records(&decl.ty, record_layouts) else {
            return;
        };
        let pointee_size = pointee_size_with_records(&decl.ty, record_layouts);
        let record = record_id_for_type(&decl.ty, record_layouts);
        let array_like = decl_is_array_like(decl);
        let compatible_card_vector_decl = compatible
            && !array_like
            && decl.storage == VarStorage::Plain
            && !decl.ty.pointer
            && matches!(decl.ty.base, TypeBase::Fund(FundType::Card));
        let mut compatible_card_alias_seen = false;

        for entry in &decl.entries {
            self.record_machine_caret_value(decl, entry);
            if array_like && compatible {
                self.add_compatible_array_decl(decl, entry, element_size, numeric_defines);
                continue;
            }

            if compatible
                && !array_like
                && pointee_size.is_none()
                && let Some(initializer) = &entry.initializer
                && let Some(address) = self.absolute_alias_initializer(initializer)
            {
                let slot = alias_storage_slot(address, slot_size, pointee_size)
                    .record(record)
                    .signed(slot_signed_for_type(&decl.ty));
                self.symbols.insert(normalize_name(&entry.name), slot);
                if compatible_card_vector_decl {
                    compatible_card_alias_seen = true;
                }
                continue;
            }

            let compatible_card_alias_initialized_padding = compatible_card_vector_decl
                && compatible_card_alias_seen
                && entry.initializer.is_some();
            let total_size = if array_like {
                array_byte_size_with_defines(entry, element_size, numeric_defines)
            } else if compatible_card_vector_decl
                && compatible_card_alias_seen
                && entry.initializer.is_none()
            {
                4
            } else {
                slot_size
            };
            if compatible_card_alias_initialized_padding {
                self.allocate_initialized(3, StorageInit::Byte(0));
            }
            let address = if compatible {
                self.allocate_entry_initialized(entry, total_size, element_size)
            } else {
                let address = self.next_address;
                self.advance(total_size);
                address
            };
            let slot = if array_like {
                StorageSlot::array(address, element_size, ArrayStorage::Inline)
            } else if let Some(pointee_size) = pointee_size {
                StorageSlot::pointer(address, pointee_size)
            } else {
                StorageSlot::absolute(address, slot_size)
            }
            .record(record)
            .signed(slot_signed_for_type(&decl.ty));
            self.symbols.insert(normalize_name(&entry.name), slot);
        }
    }

    fn record_machine_caret_value(&mut self, decl: &VarDecl, entry: &DeclEntry) {
        if !decl_is_array_like(decl) {
            return;
        }
        let Some(initializer) = &entry.initializer else {
            return;
        };
        let value = absolute_alias_initializer(&self.symbols, initializer)
            .or_else(|| constant_u16(initializer));
        if let Some(value) = value {
            self.machine_caret_values
                .insert(normalize_name(&entry.name), value);
        }
    }

    fn add_compatible_array_decl(
        &mut self,
        decl: &VarDecl,
        entry: &DeclEntry,
        element_size: u16,
        numeric_defines: &HashMap<String, u16>,
    ) {
        let signed = type_is_signed(&decl.ty);
        let name = normalize_name(&entry.name);
        if let Some(address) = absolute_array_address_initializer(entry) {
            if array_len_with_defines(entry, numeric_defines).is_some_and(|len| len > 0x0100) {
                let descriptor =
                    self.allocate_initialized_bytes(4, &fixed_array_pointer_storage(address));
                self.symbols.insert(
                    name.clone(),
                    StorageSlot::array(descriptor, element_size, ArrayStorage::Descriptor)
                        .signed(signed),
                );
                self.absolute_array_value_addresses
                    .insert(name.clone(), address);
                self.machine_symbol_addresses
                    .insert(name, MachineSymbolAddress::Absolute(address));
                return;
            }
            self.symbols.insert(
                name,
                StorageSlot::array(address, element_size, ArrayStorage::Inline).signed(signed),
            );
            return;
        }
        if element_size == 1
            && let Some(bytes) = string_initializer_storage(entry)
        {
            let total_size = string_initialized_byte_size_with_defines(
                entry,
                bytes.len() as u16,
                numeric_defines,
            );
            let address = self.allocate_initialized_bytes(total_size, &bytes);
            self.symbols.insert(
                name,
                StorageSlot::array(address, element_size, ArrayStorage::Inline).signed(signed),
            );
            return;
        }

        if let Some(bytes) = numeric_array_initializer_storage(entry, element_size) {
            let len = array_len_with_defines(entry, numeric_defines)
                .unwrap_or((bytes.len() as u16) / element_size);
            let byte_size = element_size.saturating_mul(len).max(bytes.len() as u16);
            if element_size == 1 && entry.size.is_some() {
                let address = self.allocate_initialized_bytes(byte_size, &bytes);
                self.symbols.insert(
                    name,
                    StorageSlot::array(address, element_size, ArrayStorage::Inline).signed(signed),
                );
                return;
            }
            let backing_address = self.allocate_initialized_bytes(byte_size, &bytes);
            let descriptor_address = self.next_address;
            let descriptor_size = if entry.size.is_some() { 4 } else { 2 };
            self.advance(descriptor_size);
            let backing = Immediate::new(backing_address);
            self.initializers.push(StorageInit::Byte(backing.low()));
            self.initializers.push(StorageInit::Byte(backing.high()));
            if descriptor_size == 4 {
                self.initializers.push(StorageInit::Byte(0));
                self.initializers.push(StorageInit::Byte(0));
            }
            self.symbols.insert(
                name.clone(),
                StorageSlot::array(descriptor_address, element_size, ArrayStorage::Descriptor)
                    .signed(signed),
            );
            self.machine_symbol_addresses
                .insert(name, MachineSymbolAddress::Absolute(backing_address));
            return;
        }

        let Some(len) = array_len_with_defines(entry, numeric_defines) else {
            let pointer_bytes = entry
                .initializer
                .as_ref()
                .and_then(constant_u16)
                .map(|address| {
                    let address = Immediate::new(address);
                    [address.low(), address.high()]
                });
            let address = if let Some(bytes) = pointer_bytes {
                self.allocate_initialized_bytes(2, &bytes)
            } else {
                self.allocate_initialized(2, StorageInit::Byte(0))
            };
            self.symbols.insert(
                name,
                StorageSlot::array(address, element_size, ArrayStorage::Pointer).signed(signed),
            );
            return;
        };

        let byte_size = element_size.saturating_mul(len);
        if element_size == 1 && len <= 0x0100 {
            let address = self.allocate_sized_byte_array_storage(byte_size, len);
            self.symbols.insert(
                name,
                StorageSlot::array(address, element_size, ArrayStorage::Inline).signed(signed),
            );
            return;
        }

        let label = format!("array:{}", name);
        let address = self.next_address;
        self.advance(4);
        self.initializers
            .push(StorageInit::LabelWord(label.clone()));
        let size = Immediate::new(byte_size);
        self.initializers.push(StorageInit::Byte(size.low()));
        self.initializers.push(StorageInit::Byte(size.high()));
        self.array_backings.push(ArrayBacking {
            label: label.clone(),
            size: byte_size,
        });
        self.symbols.insert(
            name.clone(),
            StorageSlot::array(address, element_size, ArrayStorage::Descriptor).signed(signed),
        );
        self.machine_symbol_addresses
            .insert(name, MachineSymbolAddress::Label(label));
    }

    fn allocate_sized_byte_array_storage(&mut self, byte_size: u16, len: u16) -> u16 {
        let bytes = sized_byte_array_storage_bytes(byte_size, len);
        self.allocate_initialized_bytes(byte_size, &bytes)
    }

    pub(super) fn lookup(&self, name: &str) -> Option<StorageSlot> {
        self.symbols.get(&normalize_name(name)).copied()
    }

    fn absolute_alias_initializer(&self, expr: &Expr) -> Option<u16> {
        absolute_alias_initializer(&self.symbols, expr)
    }

    pub(super) fn allocate(&mut self, size: u16) -> u16 {
        let address = self.next_address;
        self.advance(size);
        address
    }

    fn allocate_initialized(&mut self, size: u16, init: StorageInit) -> u16 {
        let address = self.next_address;
        self.advance(size);
        self.initializers
            .extend(std::iter::repeat_n(init, usize::from(size)));
        address
    }

    fn allocate_entry_initialized(
        &mut self,
        entry: &DeclEntry,
        total_size: u16,
        element_size: u16,
    ) -> u16 {
        let address = self.next_address;
        self.advance(total_size);
        extend_entry_initializers(&mut self.initializers, entry, total_size, element_size);
        address
    }

    fn allocate_initialized_bytes(&mut self, size: u16, bytes: &[u8]) -> u16 {
        let address = self.next_address;
        self.advance(size);
        self.initializers
            .extend(bytes.iter().copied().map(StorageInit::Byte));
        let padding = usize::from(size).saturating_sub(bytes.len());
        self.initializers
            .extend(std::iter::repeat_n(StorageInit::Byte(0), padding));
        address
    }

    fn advance(&mut self, size: u16) {
        self.next_address = self.next_address.wrapping_add(size);
        self.storage_size = self.storage_size.wrapping_add(size);
    }
}

pub(super) fn allocate_routine_symbols(
    routine: &Routine,
    base: u16,
    record_layouts: &RecordLayouts,
    compatible_layout: bool,
    numeric_defines: &HashMap<String, u16>,
    outer_symbols: &HashMap<String, StorageSlot>,
) -> RoutineAllocation {
    // Local declarations may name global storage in an address initializer
    // (`BYTE high=word+1`).  Resolve against the enclosing program layout,
    // while returning only routine-owned symbols to the caller.
    let mut symbols = outer_symbols.clone();
    let mut next_address = base;
    let mut initializers = Vec::new();
    let mut array_backings = Vec::new();
    let mut machine_symbol_addresses = HashMap::new();

    for param in &routine.params {
        let Some(element_size) = type_size_with_records(&param.ty, record_layouts) else {
            continue;
        };
        let slot_size = if decl_is_array_like(param) {
            2
        } else if let Some(slot_size) = storage_size_with_records(&param.ty, record_layouts) {
            slot_size
        } else {
            continue;
        };
        let pointee_size = pointee_size_with_records(&param.ty, record_layouts);
        let record = record_id_for_type(&param.ty, record_layouts);
        for entry in &param.entries {
            let address = next_address;
            next_address = next_address.wrapping_add(slot_size);
            let slot = if decl_is_array_like(param) {
                StorageSlot::array(address, element_size, ArrayStorage::Pointer)
            } else if let Some(pointee_size) = pointee_size {
                StorageSlot::pointer(address, pointee_size)
            } else {
                StorageSlot::absolute(address, slot_size)
            }
            .record(record)
            .signed(slot_signed_for_type(&param.ty));
            symbols.insert(normalize_name(&entry.name), slot);
            initializers.extend(std::iter::repeat_n(
                StorageInit::Byte(0),
                usize::from(slot_size),
            ));
        }
    }

    for local in &routine.locals {
        if let Decl::Var(decl) = local {
            add_var_decl_to_routine_storage(
                &mut symbols,
                decl,
                &routine.name,
                record_layouts,
                &mut next_address,
                &mut initializers,
                &mut array_backings,
                &mut machine_symbol_addresses,
                compatible_layout,
                numeric_defines,
            );
        }
    }

    let routine_symbols = routine_storage_symbol_names(routine)
        .into_iter()
        .filter_map(|name| symbols.get(&name).copied().map(|slot| (name, slot)))
        .collect();

    RoutineAllocation {
        symbols: routine_symbols,
        initializers,
        array_backings,
        machine_symbol_addresses,
    }
}

fn routine_storage_symbol_names(routine: &Routine) -> Vec<String> {
    routine
        .params
        .iter()
        .chain(routine.locals.iter().filter_map(|decl| match decl {
            Decl::Var(decl) => Some(decl),
            _ => None,
        }))
        .flat_map(|decl| decl.entries.iter().map(|entry| normalize_name(&entry.name)))
        .collect()
}

fn add_var_decl_to_routine_storage(
    symbols: &mut HashMap<String, StorageSlot>,
    decl: &VarDecl,
    routine_name: &str,
    record_layouts: &RecordLayouts,
    next_address: &mut u16,
    initializers: &mut Vec<StorageInit>,
    array_backings: &mut Vec<ArrayBacking>,
    machine_symbol_addresses: &mut HashMap<String, MachineSymbolAddress>,
    compatible_layout: bool,
    numeric_defines: &HashMap<String, u16>,
) {
    let Some(element_size) = type_size_with_records(&decl.ty, record_layouts) else {
        return;
    };
    let Some(slot_size) = storage_size_with_records(&decl.ty, record_layouts) else {
        return;
    };
    let pointee_size = pointee_size_with_records(&decl.ty, record_layouts);
    let record = record_id_for_type(&decl.ty, record_layouts);
    let local_unsized_initialized_word_array_count = if decl_is_array_like(decl) && element_size > 1
    {
        decl.entries
            .iter()
            .filter(|entry| {
                entry.size.is_none()
                    && numeric_array_initializer_storage(entry, element_size).is_some()
            })
            .count()
    } else {
        0
    };
    let mut local_unsized_initialized_word_array_seen = false;

    for entry in &decl.entries {
        if !decl_is_array_like(decl)
            && pointee_size.is_none()
            && let Some(initializer) = &entry.initializer
            && let Some(address) = absolute_alias_initializer(symbols, initializer)
        {
            let slot = alias_storage_slot(address, slot_size, pointee_size)
                .record(record)
                .signed(slot_signed_for_type(&decl.ty));
            symbols.insert(normalize_name(&entry.name), slot);
            continue;
        }

        let pointer_backed_array = decl_is_array_like(decl)
            && array_entry_is_unsized_pointer_with_defines(entry, element_size, numeric_defines);
        let large_uninitialized_byte_array = decl_is_array_like(decl)
            && uninitialized_sized_byte_array_len_with_defines(
                entry,
                element_size,
                numeric_defines,
            )
            .is_some_and(|len| {
                let threshold = if compatible_layout { 0x0100 } else { 0x00FF };
                len > threshold
            });
        if decl_is_array_like(decl)
            && element_size > 1
            && let Some(bytes) = numeric_array_initializer_storage(entry, element_size)
        {
            let len = array_len_with_defines(entry, numeric_defines)
                .unwrap_or((bytes.len() as u16) / element_size);
            let byte_size = element_size.saturating_mul(len).max(bytes.len() as u16);
            if compatible_layout
                && entry.size.is_none()
                && local_unsized_initialized_word_array_count > 1
                && !local_unsized_initialized_word_array_seen
            {
                *next_address = (*next_address).wrapping_add(2);
                initializers.extend(std::iter::repeat_n(StorageInit::Byte(0), 2));
            }
            local_unsized_initialized_word_array_seen |= entry.size.is_none();
            let backing_address = *next_address;
            *next_address = (*next_address).wrapping_add(byte_size);
            initializers.extend(bytes.iter().copied().map(StorageInit::Byte));
            let padding = usize::from(byte_size).saturating_sub(bytes.len());
            initializers.extend(std::iter::repeat_n(StorageInit::Byte(0), padding));

            let descriptor_address = *next_address;
            let descriptor_size = if entry.size.is_some() { 4 } else { 2 };
            *next_address = (*next_address).wrapping_add(descriptor_size);
            let backing = Immediate::new(backing_address);
            initializers.push(StorageInit::Byte(backing.low()));
            initializers.push(StorageInit::Byte(backing.high()));
            if descriptor_size == 4 {
                initializers.push(StorageInit::Byte(0));
                initializers.push(StorageInit::Byte(0));
            }
            let slot = StorageSlot::array(descriptor_address, element_size, ArrayStorage::Pointer)
                .record(record)
                .signed(slot_signed_for_type(&decl.ty));
            let name = normalize_name(&entry.name);
            symbols.insert(name.clone(), slot);
            machine_symbol_addresses.insert(name, MachineSymbolAddress::Absolute(backing_address));
            continue;
        }
        if decl_is_array_like(decl)
            && absolute_array_address_initializer(entry).is_none()
            && numeric_array_initializer_storage(entry, element_size).is_none()
            && string_initializer_storage(entry).is_none()
            && let Some(initializer) = &entry.initializer
            && let Some(target_address) = absolute_alias_initializer(symbols, initializer)
        {
            let descriptor_size = if entry.size.is_some() { 4 } else { 2 };
            let address = *next_address;
            *next_address = (*next_address).wrapping_add(descriptor_size);
            let target = Immediate::new(target_address);
            initializers.push(StorageInit::Byte(target.low()));
            initializers.push(StorageInit::Byte(target.high()));
            if descriptor_size == 4 {
                initializers.push(StorageInit::Byte(target.low()));
                initializers.push(StorageInit::Byte(target.high()));
            }
            let slot = StorageSlot::array(address, element_size, ArrayStorage::Pointer)
                .record(record)
                .signed(slot_signed_for_type(&decl.ty));
            symbols.insert(normalize_name(&entry.name), slot);
            continue;
        }
        if decl_is_array_like(decl)
            && absolute_array_address_initializer(entry).is_none()
            && numeric_array_initializer_storage(entry, element_size).is_none()
            && string_initializer_storage(entry).is_none()
            && let Some(label) = symbolic_array_address_initializer(entry)
        {
            let descriptor_size = if entry.size.is_some() { 4 } else { 2 };
            let address = *next_address;
            *next_address = (*next_address).wrapping_add(descriptor_size);
            initializers.push(StorageInit::LabelWord(label.clone()));
            if descriptor_size == 4 {
                initializers.push(StorageInit::LabelWord(label));
            }
            let slot = StorageSlot::array(address, element_size, ArrayStorage::Pointer)
                .record(record)
                .signed(slot_signed_for_type(&decl.ty));
            symbols.insert(normalize_name(&entry.name), slot);
            continue;
        }
        let descriptor_backed_array = decl_is_array_like(decl)
            && !pointer_backed_array
            && numeric_array_initializer_storage(entry, element_size).is_none()
            && (element_size > 1 || large_uninitialized_byte_array);
        if descriptor_backed_array && let Some(len) = array_len_with_defines(entry, numeric_defines)
        {
            let byte_size = element_size.saturating_mul(len);
            let address = *next_address;
            *next_address = (*next_address).wrapping_add(4);
            let label = format!(
                "array:{}:{}",
                normalize_name(routine_name),
                normalize_name(&entry.name)
            );
            initializers.push(StorageInit::LabelWord(label.clone()));
            let size = Immediate::new(byte_size);
            initializers.push(StorageInit::Byte(size.low()));
            initializers.push(StorageInit::Byte(size.high()));
            array_backings.push(ArrayBacking {
                label: label.clone(),
                size: byte_size,
            });
            let slot = StorageSlot::array(address, element_size, ArrayStorage::Descriptor)
                .record(record)
                .signed(slot_signed_for_type(&decl.ty));
            let name = normalize_name(&entry.name);
            symbols.insert(name.clone(), slot);
            machine_symbol_addresses.insert(name, MachineSymbolAddress::Label(label));
            continue;
        }

        let absolute_array_address = if decl_is_array_like(decl) {
            absolute_array_address_initializer(entry)
        } else {
            None
        };
        let total_size = if pointer_backed_array {
            2
        } else if decl_is_array_like(decl) {
            array_byte_size_with_defines(entry, element_size, numeric_defines)
        } else {
            slot_size
        };
        let address = *next_address;
        *next_address = (*next_address).wrapping_add(total_size);
        if decl_is_array_like(decl)
            && element_size == 1
            && string_initializer_storage(entry).is_none()
            && numeric_array_initializer_storage(entry, element_size).is_none()
            && let Some(len) = array_len_with_defines(entry, numeric_defines)
        {
            let skip_threshold = if compatible_layout { 0x0100 } else { 0x00FF };
            if total_size > skip_threshold {
                initializers.push(StorageInit::Skip(SkippedRange {
                    start: address,
                    len: total_size,
                }));
            } else {
                initializers.extend(
                    sized_byte_array_storage_bytes(total_size, len)
                        .iter()
                        .copied()
                        .map(StorageInit::Byte),
                );
            }
        } else {
            extend_entry_initializers(initializers, entry, total_size, element_size);
        }
        let slot = if let Some(address) = absolute_array_address {
            StorageSlot::array(address, element_size, ArrayStorage::Inline)
        } else if pointer_backed_array {
            StorageSlot::array(address, element_size, ArrayStorage::Pointer)
        } else if decl_is_array_like(decl) {
            StorageSlot::array(address, element_size, ArrayStorage::Inline)
        } else if let Some(pointee_size) = pointee_size {
            StorageSlot::pointer(address, pointee_size)
        } else {
            StorageSlot::absolute(address, slot_size)
        }
        .record(record)
        .signed(slot_signed_for_type(&decl.ty));
        symbols.insert(normalize_name(&entry.name), slot);
    }
}

pub(super) fn add_var_decl_to_symbols(
    symbols: &mut HashMap<String, StorageSlot>,
    decl: &VarDecl,
    record_layouts: &RecordLayouts,
    numeric_defines: &HashMap<String, u16>,
    allow_absolute_scalar_aliases: bool,
    mut allocate: impl FnMut(u16) -> u16,
) {
    let Some(element_size) = type_size_with_records(&decl.ty, record_layouts) else {
        return;
    };
    let Some(slot_size) = storage_size_with_records(&decl.ty, record_layouts) else {
        return;
    };
    let pointee_size = pointee_size_with_records(&decl.ty, record_layouts);
    let record = record_id_for_type(&decl.ty, record_layouts);

    for entry in &decl.entries {
        if allow_absolute_scalar_aliases
            && !decl_is_array_like(decl)
            && pointee_size.is_none()
            && let Some(initializer) = &entry.initializer
            && let Some(address) = absolute_alias_initializer(symbols, initializer)
        {
            let slot = alias_storage_slot(address, slot_size, pointee_size)
                .record(record)
                .signed(slot_signed_for_type(&decl.ty));
            symbols.insert(normalize_name(&entry.name), slot);
            continue;
        }

        let absolute_array_address = if decl_is_array_like(decl) {
            absolute_array_address_initializer(entry)
        } else {
            None
        };
        let total_size = if decl_is_array_like(decl) {
            array_byte_size_with_defines(entry, element_size, numeric_defines)
        } else {
            slot_size
        };
        let address = allocate(total_size);
        let slot = if let Some(address) = absolute_array_address {
            StorageSlot::array(address, element_size, ArrayStorage::Inline)
        } else if decl_is_array_like(decl) {
            StorageSlot::array(address, element_size, ArrayStorage::Inline)
        } else if let Some(pointee_size) = pointee_size {
            StorageSlot::pointer(address, pointee_size)
        } else {
            StorageSlot::absolute(address, slot_size)
        }
        .record(record)
        .signed(slot_signed_for_type(&decl.ty));
        symbols.insert(normalize_name(&entry.name), slot);
    }
}

// Extracted from src/codegen.rs: storage slot impl
impl StorageSlot {
    pub(super) fn absolute(address: u16, size: u16) -> Self {
        Self {
            address,
            size,
            space: AddressSpace::Absolute,
            index_offset: 0,
            pointee_size: None,
            array: None,
            record: None,
            signed: false,
        }
    }

    pub(super) fn array(address: u16, element_size: u16, array: ArrayStorage) -> Self {
        Self {
            address,
            size: element_size,
            space: AddressSpace::Absolute,
            index_offset: 0,
            pointee_size: None,
            array: Some(array),
            record: None,
            signed: false,
        }
    }

    pub(super) fn zero_page(address: u8, size: u16) -> Self {
        Self {
            address: u16::from(address),
            size,
            space: AddressSpace::ZeroPage,
            index_offset: 0,
            pointee_size: None,
            array: None,
            record: None,
            signed: false,
        }
    }

    pub(super) fn pointer(address: u16, pointee_size: u16) -> Self {
        Self {
            address,
            size: 2,
            space: AddressSpace::Absolute,
            index_offset: 0,
            pointee_size: Some(pointee_size),
            array: None,
            record: None,
            signed: false,
        }
    }

    pub(super) fn zero_page_pointer(address: u8, pointee_size: u16) -> Self {
        Self {
            address: u16::from(address),
            size: 2,
            space: AddressSpace::ZeroPage,
            index_offset: 0,
            pointee_size: Some(pointee_size),
            array: None,
            record: None,
            signed: false,
        }
    }

    pub(super) fn indirect_indexed_y(pointer: ZeroPage, size: u16) -> Self {
        Self {
            address: u16::from(pointer.address()),
            size,
            space: AddressSpace::IndirectIndexedY,
            index_offset: 0,
            pointee_size: None,
            array: None,
            record: None,
            signed: false,
        }
    }

    pub(super) fn absolute_x(address: u16, size: u16) -> Self {
        Self {
            address,
            size,
            space: AddressSpace::AbsoluteX,
            index_offset: 0,
            pointee_size: None,
            array: None,
            record: None,
            signed: false,
        }
    }

    pub(super) fn record(mut self, record: Option<usize>) -> Self {
        self.record = record;
        self
    }

    pub(super) fn signed(mut self, signed: bool) -> Self {
        self.signed = signed;
        self
    }

    pub(super) fn offset_bytes(self, offset: u16) -> Self {
        if self.space == AddressSpace::IndirectIndexedY {
            Self {
                index_offset: self.index_offset.wrapping_add(offset),
                ..self
            }
        } else {
            Self {
                address: self.address.wrapping_add(offset),
                ..self
            }
        }
    }

    pub(super) fn y_index(self, byte_index: u16) -> u8 {
        self.index_offset.wrapping_add(byte_index) as u8
    }

    pub(super) fn with_size(self, size: u16) -> Self {
        Self { size, ..self }
    }

    pub(super) fn byte_address(self, byte_index: u16) -> u16 {
        self.address.wrapping_add(byte_index)
    }

    pub(super) fn absolute_byte(self, byte_index: u16) -> Absolute {
        Absolute::new(self.byte_address(byte_index))
    }

    pub(super) fn zero_page_byte(self, byte_index: u16) -> ZeroPage {
        ZeroPage::new(self.byte_address(byte_index) as u8)
    }
}

// Extracted from src/codegen.rs: addressing wrappers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Immediate(u16);

impl Immediate {
    pub fn new(value: u16) -> Self {
        Self(value)
    }

    pub fn byte(self, byte_index: u16) -> u8 {
        if byte_index >= 2 {
            return 0;
        }
        ((self.0 >> (byte_index * 8)) & 0xFF) as u8
    }

    pub fn low(self) -> u8 {
        self.byte(0)
    }

    pub fn high(self) -> u8 {
        self.byte(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Absolute(u16);

impl Absolute {
    pub const fn new(address: u16) -> Self {
        Self(address)
    }

    pub fn address(self) -> u16 {
        self.0
    }

    pub fn low(self) -> u8 {
        (self.0 & 0xFF) as u8
    }

    pub fn high(self) -> u8 {
        (self.0 >> 8) as u8
    }

    pub fn offset(self, offset: u16) -> Self {
        Self(self.0.wrapping_add(offset))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbsoluteX(Absolute);

impl AbsoluteX {
    pub fn new(address: u16) -> Self {
        Self(Absolute::new(address))
    }

    pub fn absolute(self) -> Absolute {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZeroPage(u8);

impl ZeroPage {
    pub const fn new(address: u8) -> Self {
        Self(address)
    }

    pub fn address(self) -> u8 {
        self.0
    }

    pub fn offset(self, offset: u8) -> Self {
        Self(self.0.wrapping_add(offset))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZeroPageX(ZeroPage);

impl ZeroPageX {
    pub fn new(address: u8) -> Self {
        Self(ZeroPage::new(address))
    }

    pub fn zero_page(self) -> ZeroPage {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexedIndirectX(ZeroPage);

impl IndexedIndirectX {
    pub fn new(pointer: ZeroPage) -> Self {
        Self(pointer)
    }

    pub fn pointer(self) -> ZeroPage {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndirectIndexedY(ZeroPage);

impl IndirectIndexedY {
    pub fn new(pointer: ZeroPage) -> Self {
        Self(pointer)
    }

    pub fn pointer(self) -> ZeroPage {
        self.0
    }
}

// Extracted from src/codegen.rs: record layout collection
pub(super) fn collect_record_layouts(program: &Program) -> RecordLayouts {
    let mut records = RecordLayouts::default();
    for module in &program.modules {
        for item in &module.items {
            match item {
                Item::Declaration(decl) => collect_record_layout_decl(&mut records, decl),
                Item::Routine(routine) => {
                    for local in &routine.locals {
                        collect_record_layout_decl(&mut records, local);
                    }
                }
                _ => {}
            }
        }
    }
    records
}

pub(super) fn collect_record_layout_decl(records: &mut RecordLayouts, decl: &Decl) {
    let (name, fields) = match decl {
        Decl::Type(type_decl) => (&type_decl.name, &type_decl.fields),
        Decl::Record(record_decl) => (&record_decl.name, &record_decl.fields),
        Decl::Var(_) => return,
    };
    let Some(layout) = build_record_layout(fields) else {
        return;
    };
    let id = records.layouts.len();
    records.by_name.insert(normalize_name(name), id);
    records.layouts.push(layout);
}

pub(super) fn build_record_layout(fields: &[VarDecl]) -> Option<RecordLayout> {
    let mut layout = RecordLayout {
        size: 0,
        fields: HashMap::new(),
    };
    for decl in fields {
        if decl.storage != VarStorage::Plain || decl.ty.pointer {
            return None;
        }
        let size = type_size(&decl.ty)?;
        for entry in &decl.entries {
            if entry.size.is_some() || entry.initializer.is_some() {
                return None;
            }
            layout.fields.insert(
                normalize_name(&entry.name),
                RecordField {
                    offset: layout.size,
                    size,
                    signed: type_is_signed(&decl.ty),
                },
            );
            layout.size = layout.size.wrapping_add(size);
        }
    }
    Some(layout)
}

// Extracted from src/codegen.rs: runtime zp
pub mod runtime_zp {
    use super::ZeroPage;

    pub const BRKKEY: ZeroPage = ZeroPage::new(0x11);
    pub const AFLAST: ZeroPage = ZeroPage::new(0x82);
    pub const AFCUR: ZeroPage = ZeroPage::new(0x84);
    pub const AFSIZE: ZeroPage = ZeroPage::new(0x86);
    pub const ARGS: ZeroPage = ZeroPage::new(0xA0);
    pub const ARG0: ZeroPage = ARGS;
    pub const VALUE_TEMP: ZeroPage = ZeroPage::new(0xAA);
    pub const ELEMENT_ADDR: ZeroPage = ZeroPage::new(0xAC);
    pub const ARRAY_ADDR: ZeroPage = ZeroPage::new(0xAE);
    pub const DEVICE: ZeroPage = ZeroPage::new(0xB7);
    pub const ADDR: ZeroPage = ZeroPage::new(0xC0);
    pub const TOKEN: ZeroPage = ZeroPage::new(0xC2);
}

// Extracted from src/codegen.rs: record sizing helpers
pub(super) fn type_size_with_records(ty: &TypeRef, record_layouts: &RecordLayouts) -> Option<u16> {
    if ty.pointer {
        return Some(2);
    }
    type_size(ty).or_else(|| {
        let TypeBase::Named(name) = &ty.base else {
            return None;
        };
        record_layouts.get(name).map(|(_, layout)| layout.size)
    })
}

// Extracted from src/codegen.rs: storage sizing helpers
pub(super) fn storage_size_with_records(
    ty: &TypeRef,
    record_layouts: &RecordLayouts,
) -> Option<u16> {
    if ty.pointer {
        Some(2)
    } else {
        type_size_with_records(ty, record_layouts)
    }
}

pub(super) fn pointee_size_with_records(
    ty: &TypeRef,
    record_layouts: &RecordLayouts,
) -> Option<u16> {
    if ty.pointer {
        type_size_with_records(
            &TypeRef {
                base: ty.base.clone(),
                pointer: false,
            },
            record_layouts,
        )
    } else {
        None
    }
}

pub(super) fn record_id_for_type(ty: &TypeRef, record_layouts: &RecordLayouts) -> Option<usize> {
    let TypeBase::Named(name) = &ty.base else {
        return None;
    };
    record_layouts.get(name).map(|(id, _)| id)
}

// Extracted from src/codegen.rs: alias pointer slots
pub(super) fn alias_storage_slot(
    address: u16,
    size: u16,
    pointee_size: Option<u16>,
) -> StorageSlot {
    if let Some(pointee_size) = pointee_size {
        if address <= 0xFF {
            StorageSlot::zero_page_pointer(address as u8, pointee_size)
        } else {
            StorageSlot::pointer(address, pointee_size)
        }
    } else if address <= 0xFF {
        StorageSlot::zero_page(address as u8, size)
    } else {
        StorageSlot::absolute(address, size)
    }
}

pub(super) fn pointer_pointee_slot(pointer: StorageSlot, addr: ZeroPage) -> Option<StorageSlot> {
    let size = pointer
        .pointee_size
        .or_else(|| pointer.array.map(|_| pointer.size))?;
    let slot = StorageSlot::indirect_indexed_y(addr, size).signed(pointer.signed);
    debug_assert_pointer_pointee_slot(pointer, slot);
    debug_assert_indirect_slot_pointer(slot, addr, "pointer pointee");
    Some(slot)
}

pub(super) fn debug_assert_pointer_pointee_slot(pointer: StorageSlot, slot: StorageSlot) {
    debug_assert!(
        pointer.pointee_size.is_some() || pointer.array.is_some(),
        "pointer dereference requires a known pointee size"
    );
    debug_assert_eq!(
        slot.size,
        pointer
            .pointee_size
            .or_else(|| pointer.array.map(|_| pointer.size))
            .unwrap_or_default(),
        "pointer dereference slot must use pointee size, not pointer storage size"
    );
    debug_assert_eq!(
        slot.signed, pointer.signed,
        "pointer dereference slot must preserve pointee signedness"
    );
    debug_assert!(
        slot.pointee_size.is_none(),
        "pointer dereference result must be a value slot, not another pointer"
    );
    debug_assert!(
        slot.array.is_none(),
        "pointer dereference result must be a value slot, not an array slot"
    );
}
