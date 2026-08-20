use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RoutineInfo {
    pub(super) label: String,
    pub(super) params: Vec<StorageSlot>,
    pub(super) return_slot: Option<StorageSlot>,
    pub(super) system_address: Option<u16>,
    pub(super) facts: RoutineFacts,
    pub(super) effects: RoutineEffects,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RoutineInternalAbi {
    pub(super) result: InternalResultAbi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InternalResultAbi {
    None,
    Value {
        public_slot: StorageSlot,
        bytes: [Option<InternalResultByte>; 2],
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InternalResultByte {
    PublicSlot(u16),
    RegisterA,
    #[allow(dead_code)]
    RegisterX,
    #[allow(dead_code)]
    RegisterY,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct RoutineFacts {
    pub(super) returns_a_equals_a0: bool,
    pub(super) returns_a_equals_a1: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct InferredRoutineFacts {
    pub(super) returns_a_equals_a0_candidate: bool,
    pub(super) returns_a_equals_a1_candidate: bool,
    pub(super) saw_value_return: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct RoutineEffects {
    pub(super) known: bool,
    pub(super) preserves_a: bool,
    pub(super) preserves_x: bool,
    pub(super) preserves_y: bool,
    pub(super) writes_array_addr: bool,
    pub(super) writes_element_addr: bool,
    pub(super) writes_addr: bool,
    pub(super) writes_args: bool,
    pub(super) zero_page_writes: [u64; 4],
    pub(super) absolute_writes: [Option<EffectRange>; 32],
    pub(super) writes_unknown_absolute: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct EffectRange {
    pub(super) address: u16,
    pub(super) size: u16,
}

impl RoutineInfo {
    pub(super) fn internal_abi(&self) -> RoutineInternalAbi {
        RoutineInternalAbi::from_public_result_and_facts(self.return_slot, self.facts)
    }
}

impl RoutineInternalAbi {
    pub(super) fn from_public_result_and_facts(
        return_slot: Option<StorageSlot>,
        facts: RoutineFacts,
    ) -> Self {
        let Some(public_slot) = return_slot else {
            return Self {
                result: InternalResultAbi::None,
            };
        };
        let mut bytes = [None, None];
        for byte_index in 0..public_slot.size.min(2) {
            bytes[usize::from(byte_index)] = Some(InternalResultByte::PublicSlot(byte_index));
        }
        if facts.returns_a_equals_a0 && public_slot.size >= 1 {
            bytes[0] = Some(InternalResultByte::RegisterA);
        }
        if facts.returns_a_equals_a1 && public_slot.size >= 2 {
            bytes[1] = Some(InternalResultByte::RegisterA);
        }
        Self {
            result: InternalResultAbi::Value { public_slot, bytes },
        }
    }

    pub(super) fn public_result_slot(&self) -> Option<StorageSlot> {
        match self.result {
            InternalResultAbi::None => None,
            InternalResultAbi::Value { public_slot, .. } => Some(public_slot),
        }
    }

    pub(super) fn result_byte(&self, byte_index: u16) -> Option<InternalResultByte> {
        match self.result {
            InternalResultAbi::None => None,
            InternalResultAbi::Value { bytes, .. } => {
                bytes.get(usize::from(byte_index)).copied()?
            }
        }
    }

    pub(super) fn result_byte_is_register_a(&self, byte_index: u16) -> bool {
        self.result_byte(byte_index) == Some(InternalResultByte::RegisterA)
    }
}

impl RoutineEffects {
    pub(super) fn unknown() -> Self {
        Self {
            known: false,
            ..Self::default()
        }
    }

    pub(super) fn known_empty() -> Self {
        Self {
            known: true,
            ..Self::default()
        }
    }

    pub(super) fn merge(&mut self, other: Self) {
        if !self.known || !other.known {
            *self = Self::unknown();
            return;
        }
        self.preserves_a &= other.preserves_a;
        self.preserves_x &= other.preserves_x;
        self.preserves_y &= other.preserves_y;
        self.writes_array_addr |= other.writes_array_addr;
        self.writes_element_addr |= other.writes_element_addr;
        self.writes_addr |= other.writes_addr;
        self.writes_args |= other.writes_args;
        for (target, source) in self.zero_page_writes.iter_mut().zip(other.zero_page_writes) {
            *target |= source;
        }
        self.writes_unknown_absolute |= other.writes_unknown_absolute;
        for range in other.absolute_writes.into_iter().flatten() {
            self.record_absolute_write(range.address, range.size);
        }
    }

    pub(super) fn record_zero_page_write(&mut self, zero_page: ZeroPage) {
        if !self.known {
            return;
        }
        let address = zero_page.address();
        self.zero_page_writes[usize::from(address / 64)] |= 1u64 << (address % 64);
        if pointer_pair_contains(runtime_zp::ARRAY_ADDR, address) {
            self.writes_array_addr = true;
        }
        if pointer_pair_contains(runtime_zp::ELEMENT_ADDR, address) {
            self.writes_element_addr = true;
        }
        if pointer_pair_contains(runtime_zp::ADDR, address) {
            self.writes_addr = true;
        }
        if (runtime_zp::ARGS.address()..=runtime_zp::ARGS.offset(15).address()).contains(&address) {
            self.writes_args = true;
        }
    }

    pub(super) fn writes_pointer_pair(self, pointer: ZeroPage) -> bool {
        if pointer == runtime_zp::ARRAY_ADDR {
            self.writes_array_addr
        } else if pointer == runtime_zp::ELEMENT_ADDR {
            self.writes_element_addr
        } else if pointer == runtime_zp::ADDR {
            self.writes_addr
        } else {
            self.writes_zero_page(pointer) || self.writes_zero_page(pointer.offset(1))
        }
    }

    pub(super) fn writes_zero_page(self, zero_page: ZeroPage) -> bool {
        let address = zero_page.address();
        (self.zero_page_writes[usize::from(address / 64)] & (1u64 << (address % 64))) != 0
    }

    pub(super) fn record_absolute_write(&mut self, address: u16, size: u16) {
        if !self.known {
            return;
        }
        if address < 0x100 {
            let end = address.saturating_add(size.max(1)).min(0x100);
            for byte in address..end {
                self.record_zero_page_write(ZeroPage::new(byte as u8));
            }
            if end == address.saturating_add(size.max(1)) {
                return;
            }
        }
        let range = EffectRange {
            address,
            size: size.max(1),
        };
        if self
            .absolute_writes
            .iter()
            .flatten()
            .any(|existing| *existing == range)
        {
            return;
        }
        if let Some(slot) = self.absolute_writes.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(range);
        } else {
            self.writes_unknown_absolute = true;
        }
    }

    pub(super) fn record_unknown_absolute_write(&mut self) {
        if self.known {
            self.writes_unknown_absolute = true;
        }
    }

    pub(super) fn writes_absolute_range(self, address: u16, size: u16) -> bool {
        self.writes_unknown_absolute
            || self
                .absolute_writes
                .iter()
                .flatten()
                .any(|range| ranges_overlap(range.address, range.size, address, size.max(1)))
    }

    pub(super) fn clear_zero_page_write(&mut self, zero_page: ZeroPage) {
        if !self.known {
            return;
        }
        let address = zero_page.address();
        self.zero_page_writes[usize::from(address / 64)] &= !(1u64 << (address % 64));
        self.refresh_pointer_write_flags();
    }

    pub(super) fn clear_absolute_write(&mut self, address: u16, size: u16) {
        if !self.known {
            return;
        }
        let size = size.max(1);
        if address < 0x100 && address.saturating_add(size) <= 0x100 {
            for byte in address..address.saturating_add(size) {
                self.clear_zero_page_write(ZeroPage::new(byte as u8));
            }
            return;
        }
        let range = EffectRange { address, size };
        for slot in &mut self.absolute_writes {
            if *slot == Some(range) {
                *slot = None;
            }
        }
    }

    pub(super) fn refresh_pointer_write_flags(&mut self) {
        self.writes_array_addr = self.writes_pointer_pair_raw(runtime_zp::ARRAY_ADDR);
        self.writes_element_addr = self.writes_pointer_pair_raw(runtime_zp::ELEMENT_ADDR);
        self.writes_addr = self.writes_pointer_pair_raw(runtime_zp::ADDR);
        self.writes_args = (runtime_zp::ARGS.address()..=runtime_zp::ARGS.offset(15).address())
            .any(|address| self.writes_zero_page(ZeroPage::new(address)));
    }

    pub(super) fn writes_pointer_pair_raw(self, pointer: ZeroPage) -> bool {
        self.writes_zero_page(pointer) || self.writes_zero_page(pointer.offset(1))
    }

    pub(super) fn preserve_register(&mut self, register: AnnotationRegister) {
        if !self.known {
            return;
        }
        match register {
            AnnotationRegister::A => self.preserves_a = true,
            AnnotationRegister::X => self.preserves_x = true,
            AnnotationRegister::Y => self.preserves_y = true,
        }
    }

    pub(super) fn clobber_register(&mut self, register: AnnotationRegister) {
        match register {
            AnnotationRegister::A => self.preserves_a = false,
            AnnotationRegister::X => self.preserves_x = false,
            AnnotationRegister::Y => self.preserves_y = false,
        }
    }
}

fn pointer_pair_contains(pointer: ZeroPage, address: u8) -> bool {
    address == pointer.address() || address == pointer.offset(1).address()
}

pub(super) fn collect_routine_info(
    program: &Program,
    record_layouts: &RecordLayouts,
    runtime_target: RuntimeTarget,
) -> Result<HashMap<String, RoutineInfo>, Vec<Diagnostic>> {
    let mut routines = HashMap::new();
    for module in &program.modules {
        for item in &module.items {
            let Item::Routine(routine) = item else {
                continue;
            };
            let system_address = routine_absolute_system_address(routine);
            routines.insert(
                normalize_name(&routine.name),
                routine_info(
                    routine,
                    record_layouts,
                    format!("routine:{}", routine.name),
                    system_address,
                ),
            );
        }
    }

    if runtime_target == RuntimeTarget::Cartridge {
        add_cart_sys_interface_routine_info(&mut routines, record_layouts)?;
    }
    Ok(routines)
}

fn routine_info(
    routine: &Routine,
    record_layouts: &RecordLayouts,
    label: String,
    system_address: Option<u16>,
) -> RoutineInfo {
    let mut params = Vec::new();
    let mut arg_offset = 0u8;
    for param in &routine.params {
        let Some(_element_size) = type_size_with_records(&param.ty, record_layouts) else {
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
        for _ in &param.entries {
            let slot = if decl_is_array_like(param) {
                StorageSlot::zero_page(runtime_zp::ARGS.offset(arg_offset).address(), slot_size)
                    .signed(slot_signed_for_type(&param.ty))
            } else if let Some(pointee_size) = pointee_size {
                StorageSlot::zero_page_pointer(
                    runtime_zp::ARGS.offset(arg_offset).address(),
                    pointee_size,
                )
            } else {
                StorageSlot::zero_page(runtime_zp::ARGS.offset(arg_offset).address(), slot_size)
            }
            .record(record)
            .signed(slot_signed_for_type(&param.ty));
            params.push(slot);
            arg_offset = arg_offset.wrapping_add(slot_size as u8);
        }
    }
    let return_slot = match routine.kind {
        RoutineKind::Proc => None,
        RoutineKind::Func { return_type } => {
            let ty = TypeRef {
                base: TypeBase::Fund(return_type),
                pointer: false,
            };
            type_size(&ty).map(|size| {
                StorageSlot::zero_page(runtime_zp::ARGS.address(), size).signed(type_is_signed(&ty))
            })
        }
    };
    RoutineInfo {
        label,
        params,
        return_slot,
        system_address,
        facts: routine_facts_from_annotations(&routine.annotations),
        effects: system_effects_for_address(system_address).unwrap_or_else(RoutineEffects::unknown),
    }
}

fn add_cart_sys_interface_routine_info(
    routines: &mut HashMap<String, RoutineInfo>,
    record_layouts: &RecordLayouts,
) -> Result<(), Vec<Diagnostic>> {
    let interface = crate::runtime_bindings::parse_sys_interface()?;
    let bindings = crate::runtime_bindings::parse_bindings(Runtime::ActionCart)?;
    for routine in interface
        .modules
        .iter()
        .flat_map(|module| &module.items)
        .filter_map(|item| match item {
            Item::Routine(routine)
                if routine.is_external && routine.visibility == Visibility::Public =>
            {
                Some(routine)
            }
            _ => None,
        })
    {
        let binding_name = format!("SYS.{}", routine.name);
        let key = crate::runtime_bindings::binding_key(&binding_name);
        let Some(binding) = bindings.get(&key) else {
            return Err(vec![Diagnostic::new(
                routine.span,
                format!("cart runtime has no binding for `{binding_name}`"),
            )]);
        };
        let crate::runtime_bindings::BindingTarget::Absolute(address) = binding else {
            return Err(vec![Diagnostic::new(
                routine.span,
                format!("cart runtime binding for `{binding_name}` is not an address"),
            )]);
        };
        routines
            .entry(normalize_name(&routine.name))
            .or_insert_with(|| {
                routine_info(
                    routine,
                    record_layouts,
                    format!("runtime:{binding_name}"),
                    Some(*address),
                )
            });
    }
    Ok(())
}

fn system_effects_for_address(address: Option<u16>) -> Option<RoutineEffects> {
    match address? {
        // Atari OS CIOV. Treat it as a scoped external barrier: registers are
        // clobbered by default, IOCB state may change, ordinary Action storage
        // remains valid unless it overlaps this OS range.
        0xE456 => {
            let mut effects = RoutineEffects::known_empty();
            effects.record_absolute_write(0x0340, 0x80);
            Some(effects)
        }
        // TN uses this OS keyboard helper through Getchar. It returns in A and
        // can clobber CPU registers, but it does not invalidate Action storage.
        0xF2F8 => Some(RoutineEffects::known_empty()),
        _ => None,
    }
}

pub(super) fn routine_facts_from_annotations(annotations: &[ActioncAnnotation]) -> RoutineFacts {
    let mut facts = RoutineFacts::default();
    for annotation in annotations {
        match annotation {
            ActioncAnnotation::ReturnsAEqualsA0 => facts.returns_a_equals_a0 = true,
            ActioncAnnotation::DebugProfileCompat => {}
            ActioncAnnotation::Preserves { .. }
            | ActioncAnnotation::Clobbers { .. }
            | ActioncAnnotation::Writes { .. } => {}
        }
    }
    facts
}

#[derive(Debug, Clone, Copy)]
pub(super) struct StagedCallArg<'a> {
    pub(super) expr: &'a Expr,
    pub(super) slot: StorageSlot,
    pub(super) offset: u8,
}

#[derive(Debug)]
pub(super) struct StagedCallArgs<'a> {
    pub(super) specs: Vec<StagedCallArg<'a>>,
    pub(super) total_bytes: u16,
}

impl<'a> StagedCallArgs<'a> {
    pub(super) fn new(args: &'a [Expr], params: &[StorageSlot]) -> Self {
        let mut offset = 0u8;
        let mut specs = Vec::new();
        let mut total_bytes = 0u16;
        for (expr, slot) in args.iter().zip(params.iter().copied()) {
            specs.push(StagedCallArg { expr, slot, offset });
            offset = offset.wrapping_add(slot.size as u8);
            total_bytes += slot.size;
        }
        Self { specs, total_bytes }
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = StagedCallArg<'a>> + '_ {
        self.specs.iter().copied()
    }

    pub(super) fn late_constant_byte_args(&self) -> impl Iterator<Item = StagedCallArg<'a>> + '_ {
        self.specs
            .iter()
            .copied()
            .filter(|spec| spec.offset >= 3 && spec.slot.size == 1)
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DeferredCallRegisterArg<'a> {
    pub(super) offset: u8,
    pub(super) expr: &'a Expr,
    pub(super) slot: StorageSlot,
    pub(super) literal_address: Option<Absolute>,
}

#[derive(Debug, Default)]
pub(super) struct StagedCallRegisterPlan<'a> {
    pub(super) deferred: Vec<DeferredCallRegisterArg<'a>>,
    pub(super) preloaded_offsets: Vec<u8>,
    pub(super) staged_offsets: Vec<u8>,
    pub(super) stacked_offsets: Vec<u8>,
}

impl<'a> StagedCallRegisterPlan<'a> {
    pub(super) fn defer(
        &mut self,
        offset: u8,
        expr: &'a Expr,
        slot: StorageSlot,
        literal_address: Option<Absolute>,
    ) {
        self.deferred.push(DeferredCallRegisterArg {
            offset,
            expr,
            slot,
            literal_address,
        });
    }

    pub(super) fn mark_preloaded(&mut self, offset: u8) {
        self.preloaded_offsets.push(offset);
    }

    pub(super) fn mark_preloaded_word(&mut self, offset: u8) {
        self.mark_preloaded(offset);
        self.mark_preloaded(offset.wrapping_add(1));
    }

    pub(super) fn mark_staged(&mut self, offset: u8) {
        self.staged_offsets.push(offset);
    }

    pub(super) fn is_staged(&self, offset: u8) -> bool {
        self.staged_offsets.contains(&offset)
    }

    pub(super) fn mark_stacked(&mut self, offset: u8) {
        self.stacked_offsets.push(offset);
    }

    pub(super) fn is_stacked(&self, offset: u8) -> bool {
        self.stacked_offsets.contains(&offset)
    }

    pub(super) fn is_preloaded(&self, offset: u8) -> bool {
        self.preloaded_offsets.contains(&offset)
    }

    pub(super) fn deferred_at(&self, offset: u8) -> Option<DeferredCallRegisterArg<'a>> {
        self.deferred
            .iter()
            .find(|deferred| deferred.offset == offset)
            .copied()
    }

    pub(super) fn deferred_word_at(&self, offset: u8) -> Option<DeferredCallRegisterArg<'a>> {
        self.deferred
            .iter()
            .find(|deferred| deferred.offset == offset && deferred.slot.size > 1)
            .copied()
    }

    pub(super) fn deferred_direct_word_at(
        &self,
        offset: u8,
    ) -> Option<DeferredCallRegisterArg<'a>> {
        self.deferred
            .iter()
            .find(|deferred| {
                deferred.offset == offset
                    && deferred.slot.size == 2
                    && deferred.literal_address.is_none()
            })
            .copied()
    }
}
