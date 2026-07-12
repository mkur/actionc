use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NativeAddressDestination {
    ArrayAddr,
}

impl<'a, 'm> SemIrNativeEmitter<'a, 'm> {
    pub(super) fn materialize_value_to_target(
        &mut self,
        value: &SemExpr,
        target: NativeResolvedSlot,
    ) -> Result<bool, String> {
        if let Some(addressed) = native_addressed_lvalue(value) {
            self.materialize_address_of_to_target(addressed, target)?;
            return Ok(true);
        }
        if let Some(call) = self.classifier().routine_call_expr(value) {
            self.emit_call(call)?;
            self.materialize_return_slot_to_target(target)?;
            return Ok(true);
        }
        if self.materialize_word_value_to_target(value, target.clone())? {
            return Ok(true);
        }
        if self.materialize_byte_value_to_target(value, target)? {
            return Ok(true);
        }
        Ok(false)
    }

    pub(super) fn materialize_slot_to_target(
        &mut self,
        source: NativeResolvedSlot,
        target: NativeResolvedSlot,
    ) -> Result<bool, String> {
        if source.width != target.width {
            return Err("slot materialization width mismatch".to_string());
        }
        match target.width {
            1 => {
                self.emit_lda_addr(source.address);
                self.emit_sta_addr(target.address);
            }
            2 => {
                self.emit_lda_addr(source.address + 1);
                self.emit_sta_addr(target.address + 1);
                self.emit_lda_addr(source.address);
                self.emit_sta_addr(target.address);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub(super) fn materialize_word_value_to_target(
        &mut self,
        value: &SemExpr,
        target: NativeResolvedSlot,
    ) -> Result<bool, String> {
        if target.width != 2 {
            return Ok(false);
        }

        if let Some(address) = self.materialize_inline_string_literal_address(value)? {
            self.emit_word_literal_to_target(address, target);
            return Ok(true);
        }

        let Some(source) = self
            .classifier()
            .word_source(value, NativeByteSourceMode::ZeroExtendToWord)?
        else {
            return Ok(false);
        };
        self.materialize_word_source_to_target(source, target)
    }

    pub(super) fn materialize_byte_value_to_target(
        &mut self,
        value: &SemExpr,
        target: NativeResolvedSlot,
    ) -> Result<bool, String> {
        if target.width != 1 {
            return Ok(false);
        }
        if !self.materialize_byte_value_to_a(value)? {
            return Ok(false);
        }
        self.emit_sta_addr(target.address);
        Ok(true)
    }

    pub(super) fn materialize_byte_value_to_a(&mut self, value: &SemExpr) -> Result<bool, String> {
        let Some(source) = self.classifier().value_byte_source(value, 0)? else {
            return Ok(false);
        };
        self.materialize_byte_source_to_register(source, NativeByteRegister::A)?;
        Ok(true)
    }

    pub(super) fn materialize_inline_string_literal_address(
        &mut self,
        expr: &SemExpr,
    ) -> Result<Option<u16>, String> {
        match self.classifier().address_shape(expr)? {
            Some(NativeAddressShape {
                kind: NativeAddressKind::StringLiteral,
                source,
                ..
            }) => {
                let text = source
                    .strip_prefix('"')
                    .and_then(|text| text.strip_suffix('"'))
                    .ok_or_else(|| "string literal address source is malformed".to_string())?;
                Ok(Some(self.materialize_string_literal_storage(text)?))
            }
            Some(_) | None => Ok(None),
        }
    }

    fn materialize_string_literal_storage(&mut self, text: &str) -> Result<u16, String> {
        let bytes = string_literal_storage_bytes(text)?;
        let literal_address = self
            .current_address()?
            .checked_add(3)
            .ok_or_else(|| "string literal address overflow".to_string())?;
        let after_address = literal_address
            .checked_add(
                u16::try_from(bytes.len()).map_err(|_| "string literal is too long".to_string())?,
            )
            .ok_or_else(|| "string literal end address overflow".to_string())?;
        self.emit_jmp_addr(after_address);
        self.emit_raw_bytes(bytes);
        Ok(literal_address)
    }

    pub(super) fn materialize_return_slot_to_target(
        &mut self,
        target: NativeResolvedSlot,
    ) -> Result<(), String> {
        match target.width {
            1 => {
                self.emit_lda_args(0);
                self.emit_sta_addr(target.address);
            }
            2 => {
                self.emit_lda_args(1);
                self.emit_sta_addr(target.address + 1);
                self.emit_lda_args(0);
                self.emit_sta_addr(target.address);
            }
            _ => return Err("call results wider than a word are not supported".to_string()),
        }
        Ok(())
    }

    pub(super) fn materialize_address_of_to_target(
        &mut self,
        addressed: &SemLValue,
        target: NativeResolvedSlot,
    ) -> Result<(), String> {
        if target.width != 2 {
            return Err("address-of assignment target must be word-sized".to_string());
        }
        if let Some(symbol) = self.lvalue_symbol(addressed)
            && let Some(slot) = self.storage.get(&symbol.id).cloned()
            && let Some(array) = slot.array
        {
            match array.storage {
                CodegenArrayStorage::Inline => {
                    self.emit_word_literal_to_target(slot.address, target);
                }
                CodegenArrayStorage::Pointer | CodegenArrayStorage::Descriptor => {
                    self.materialize_slot_to_target(
                        NativeResolvedSlot {
                            address: slot.address,
                            width: 2,
                            pointee_width: None,
                            record: None,
                        },
                        target,
                    )?;
                }
            }
            return Ok(());
        }
        let source_address = self.lvalue_address(addressed)?;
        self.emit_lda_imm((source_address >> 8) as u8);
        self.emit_sta_addr(target.address + 1);
        self.emit_lda_imm((source_address & 0x00FF) as u8);
        self.emit_sta_addr(target.address);
        Ok(())
    }

    pub(super) fn materialize_addressable_pointer_to(
        &mut self,
        expr: &SemExpr,
        dest: NativeAddressDestination,
    ) -> Result<u16, String> {
        let source = self.classifier().required_addressable_slot(expr)?;
        if source.width != 2 {
            return Err("pointer expression must be word-sized".to_string());
        }
        let pointee_width = source
            .pointee_width
            .ok_or_else(|| "expression is not a pointer".to_string())?;
        match dest {
            NativeAddressDestination::ArrayAddr => {
                self.emit_lda_addr(source.address);
                self.emit_sta_array_addr(0);
                self.emit_lda_addr(source.address + 1);
                self.emit_sta_array_addr(1);
            }
        }
        Ok(pointee_width)
    }

    pub(super) fn materialize_pointer_deref_address(
        &mut self,
        deref: NativePointerDeref<'_>,
        dest: NativeAddressDestination,
    ) -> Result<u16, String> {
        let pointee_width = self.materialize_addressable_pointer_to(deref.pointer, dest)?;
        if pointee_width != deref.width {
            return Err("pointer dereference width mismatch".to_string());
        }
        Ok(pointee_width)
    }

    pub(super) fn materialize_pointer_index_address(
        &mut self,
        indexed: NativePointerIndexExpr<'_>,
        dest: NativeAddressDestination,
    ) -> Result<u16, String> {
        if !matches!(indexed.element_width, 1 | 2) {
            return Err("only byte and word pointer indexes are supported".to_string());
        }
        match dest {
            NativeAddressDestination::ArrayAddr => self
                .materialize_pointer_scaled_index_to_array_addr(
                    indexed.base.address,
                    indexed.index,
                    indexed.element_width,
                )?,
        }
        Ok(indexed.element_width)
    }

    pub(super) fn materialize_pointer_backed_array_index_address(
        &mut self,
        indexed: NativeArrayIndexAccess<'_>,
        dest: NativeAddressDestination,
    ) -> Result<u16, String> {
        if !matches!(indexed.element_width, 1 | 2) {
            return Err("only byte and word array indexes are supported".to_string());
        }
        if !matches!(
            indexed.storage,
            CodegenArrayStorage::Pointer | CodegenArrayStorage::Descriptor
        ) {
            return Err("array index is not pointer-backed".to_string());
        }
        match dest {
            NativeAddressDestination::ArrayAddr => self
                .materialize_pointer_scaled_index_to_array_addr(
                    indexed.slot.address,
                    indexed.index,
                    indexed.element_width,
                )?,
        }
        Ok(indexed.element_width)
    }

    fn materialize_pointer_offset_to_array_addr(&mut self, pointer_address: u16, offset: u16) {
        if offset == 0 {
            self.emit_lda_addr(pointer_address);
            self.emit_sta_array_addr(0);
            self.emit_lda_addr(pointer_address + 1);
            self.emit_sta_array_addr(1);
            return;
        }
        self.emit_clc();
        self.emit_lda_addr(pointer_address);
        self.emit_adc_imm((offset & 0x00FF) as u8);
        self.emit_sta_array_addr(0);
        self.emit_lda_addr(pointer_address + 1);
        self.emit_adc_imm((offset >> 8) as u8);
        self.emit_sta_array_addr(1);
    }

    fn materialize_pointer_scaled_index_to_array_addr(
        &mut self,
        pointer_address: u16,
        index: &SemExpr,
        element_width: u16,
    ) -> Result<(), String> {
        if let Some(offset) = literal_word(index).and_then(|index| index.checked_mul(element_width))
        {
            self.materialize_pointer_offset_to_array_addr(pointer_address, offset);
            return Ok(());
        }
        match element_width {
            1 => {
                self.emit_array_index_to_a(index, element_width)?;
                self.emit_sta_element_addr();
                self.emit_clc();
                self.emit_lda_addr(pointer_address);
                self.emit_adc_element_addr();
                self.emit_sta_array_addr(0);
                self.emit_lda_addr(pointer_address + 1);
                self.emit_adc_imm(0);
                self.emit_sta_array_addr(1);
            }
            2 => {
                self.emit_array_index_to_a(index, element_width)?;
                self.emit_php();
                self.emit_clc();
                self.emit_adc_addr(pointer_address);
                self.emit_sta_array_addr(0);
                self.emit_lda_imm(0);
                self.emit_rol_a();
                self.emit_plp();
                self.emit_adc_addr(pointer_address + 1);
                self.emit_sta_array_addr(1);
            }
            _ => return Err("only byte and word pointer indexes are supported".to_string()),
        }
        Ok(())
    }

    pub(super) fn materialize_pointer_scaled_index_a_to_array_addr(
        &mut self,
        pointer_address: u16,
        element_width: u16,
    ) -> Result<(), String> {
        match element_width {
            1 => {
                self.emit_sta_element_addr();
                self.emit_clc();
                self.emit_lda_addr(pointer_address);
                self.emit_adc_element_addr();
                self.emit_sta_array_addr(0);
                self.emit_lda_addr(pointer_address + 1);
                self.emit_adc_imm(0);
                self.emit_sta_array_addr(1);
            }
            2 => {
                self.emit_asl_a();
                self.emit_php();
                self.emit_clc();
                self.emit_adc_addr(pointer_address);
                self.emit_sta_array_addr(0);
                self.emit_lda_imm(0);
                self.emit_rol_a();
                self.emit_plp();
                self.emit_adc_addr(pointer_address + 1);
                self.emit_sta_array_addr(1);
            }
            _ => return Err("only byte and word pointer indexes are supported".to_string()),
        }
        Ok(())
    }

    pub(super) fn materialize_inline_array_byte_to_a(
        &mut self,
        indexed: NativeArrayIndexAccess<'_>,
    ) -> Result<(), String> {
        if indexed.storage != CodegenArrayStorage::Inline {
            return Err("array index is not inline storage".to_string());
        }
        if indexed.element_width != 1 {
            return Err("only byte inline array reads can materialize to A".to_string());
        }
        if let Some(index) = literal_word(indexed.index) {
            let Some(array) = indexed.slot.array else {
                return Err("inline array read lost array metadata".to_string());
            };
            if array.len > 0 && index >= array.len {
                return Err(format!(
                    "array constant index {} is out of bounds {}",
                    index, array.len
                ));
            }
            self.emit_lda_addr(indexed.slot.address + index);
            return Ok(());
        }
        self.emit_array_index_to_x(indexed.index, 1)?;
        self.emit_lda_addr_x(indexed.slot.address);
        Ok(())
    }

    pub(super) fn materialize_inline_array_word_to_target(
        &mut self,
        indexed: NativeArrayIndexAccess<'_>,
        target: NativeResolvedSlot,
    ) -> Result<(), String> {
        if indexed.storage != CodegenArrayStorage::Inline {
            return Err("array index is not inline storage".to_string());
        }
        if indexed.element_width != 2 || target.width != 2 {
            return Err("only word inline array reads can materialize to a target".to_string());
        }
        if let Some(index) = literal_word(indexed.index) {
            let Some(array) = indexed.slot.array else {
                return Err("inline array read lost array metadata".to_string());
            };
            if array.len > 0 && index >= array.len {
                return Err(format!(
                    "array constant index {} is out of bounds {}",
                    index, array.len
                ));
            }
            let offset = index
                .checked_mul(indexed.element_width)
                .ok_or_else(|| "array index offset overflow".to_string())?;
            self.emit_lda_addr(indexed.slot.address + offset + 1);
            self.emit_sta_addr(target.address + 1);
            self.emit_lda_addr(indexed.slot.address + offset);
            self.emit_sta_addr(target.address);
            return Ok(());
        }
        self.emit_array_index_to_x(indexed.index, indexed.element_width)?;
        self.emit_lda_addr_x(indexed.slot.address + 1);
        self.emit_sta_addr(target.address + 1);
        self.emit_lda_addr_x(indexed.slot.address);
        self.emit_sta_addr(target.address);
        Ok(())
    }

    pub(super) fn materialize_args_to_inline_array_element(
        &mut self,
        indexed: NativeArrayIndexAccess<'_>,
    ) -> Result<u16, String> {
        if indexed.storage != CodegenArrayStorage::Inline {
            return Err("array index is not inline storage".to_string());
        }
        if !matches!(indexed.element_width, 1 | 2) {
            return Err("only byte and word inline array stores are supported".to_string());
        }
        let Some(array) = indexed.slot.array else {
            return Err("inline array assignment lost array metadata".to_string());
        };
        if let Some(index) = literal_word(indexed.index) {
            if array.len > 0 && index >= array.len {
                return Err(format!(
                    "array constant index {} is out of bounds {}",
                    index, array.len
                ));
            }
            let offset = index
                .checked_mul(indexed.element_width)
                .ok_or_else(|| "array index offset overflow".to_string())?;
            self.emit_array_assignment_args_to_inline_slot(
                indexed.slot.address + offset,
                indexed.element_width,
            )?;
        } else {
            self.emit_array_index_to_x(indexed.index, indexed.element_width)?;
            self.emit_array_assignment_args_to_inline_indexed(
                indexed.slot.address,
                indexed.element_width,
            )?;
        }
        Ok(indexed.element_width)
    }

    pub(super) fn materialize_args_to_array_addr_element(
        &mut self,
        width: u16,
    ) -> Result<(), String> {
        match width {
            1 => {
                self.emit_lda_args(0);
                self.ensure_y_zero();
                self.emit_sta_array_addr_indirect_y();
            }
            2 => {
                self.emit_lda_args(1);
                self.emit_y_one();
                self.emit_sta_array_addr_indirect_y();
                self.emit_lda_args(0);
                self.emit_dey();
                self.emit_sta_array_addr_indirect_y();
            }
            _ => return Err("only byte and word indirect stores are supported".to_string()),
        }
        Ok(())
    }

    pub(super) fn materialize_value_to_array_addr_element(
        &mut self,
        value: &SemExpr,
        width: u16,
    ) -> Result<bool, String> {
        if !matches!(width, 1 | 2) {
            return Ok(false);
        }
        if let Some(call) = self.classifier().routine_call_expr(value) {
            let call_width = expr_width(value)
                .ok_or_else(|| "indirect call result width is not known".to_string())?;
            if call_width != width {
                return Err("indirect call result width mismatch".to_string());
            }
            self.materialize_array_addr_to_stack();
            self.emit_call(call)?;
            self.materialize_stack_to_array_addr();
            self.materialize_return_slot_to_target(native_args_slot(width))?;
            self.materialize_args_to_array_addr_element(width)?;
            return Ok(true);
        }
        if self.materialize_value_to_target(value, native_args_slot(width))? {
            self.materialize_args_to_array_addr_element(width)?;
            return Ok(true);
        }
        self.materialize_array_addr_to_stack();
        let result = self.emit_expr_to_target(value, native_args_slot(width));
        self.materialize_stack_to_array_addr();
        if result? {
            self.materialize_args_to_array_addr_element(width)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub(super) fn materialize_array_addr_element_to_target(
        &mut self,
        target: NativeResolvedSlot,
    ) -> Result<bool, String> {
        match target.width {
            1 => {
                self.ensure_y_zero();
                self.emit_lda_array_addr_indirect_y();
                self.emit_sta_addr(target.address);
            }
            2 => {
                self.emit_y_one();
                self.emit_lda_array_addr_indirect_y();
                self.emit_sta_addr(target.address + 1);
                self.emit_dey();
                self.emit_lda_array_addr_indirect_y();
                self.emit_sta_addr(target.address);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub(super) fn materialize_array_addr_element_to_a(&mut self) {
        self.ensure_y_zero();
        self.emit_lda_array_addr_indirect_y();
    }

    pub(super) fn materialize_byte_value_address_to_array_addr(
        &mut self,
        value: &SemExpr,
    ) -> Result<bool, String> {
        if let Some(indexed) = self.classifier().array_index_access(value)? {
            if indexed.element_width != 1 {
                return Ok(false);
            }
            match indexed.storage {
                CodegenArrayStorage::Pointer | CodegenArrayStorage::Descriptor => {
                    self.materialize_pointer_backed_array_index_address(
                        indexed,
                        NativeAddressDestination::ArrayAddr,
                    )?;
                    return Ok(true);
                }
                CodegenArrayStorage::Inline => return Ok(false),
            }
        }

        if let Some(indexed) = self.classifier().pointer_index_expr(value)? {
            if indexed.element_width != 1 {
                return Ok(false);
            }
            self.materialize_pointer_index_address(indexed, NativeAddressDestination::ArrayAddr)?;
            return Ok(true);
        }

        if let Some(deref) = self.classifier().pointer_deref_expr(value) {
            if deref.width != 1 {
                return Ok(false);
            }
            self.materialize_pointer_deref_address(deref, NativeAddressDestination::ArrayAddr)?;
            return Ok(true);
        }

        Ok(false)
    }

    pub(super) fn materialize_array_addr_to_stack(&mut self) {
        self.emit_lda_array_addr(1);
        self.emit_pha();
        self.emit_lda_array_addr(0);
        self.emit_pha();
    }

    pub(super) fn materialize_stack_to_array_addr(&mut self) {
        self.emit_pla();
        self.emit_sta_array_addr(0);
        self.emit_pla();
        self.emit_sta_array_addr(1);
    }

    fn emit_array_assignment_args_to_inline_slot(
        &mut self,
        address: u16,
        width: u16,
    ) -> Result<(), String> {
        match width {
            1 => {
                self.emit_lda_args(0);
                self.emit_sta_addr(address);
            }
            2 => {
                self.emit_lda_args(1);
                self.emit_sta_addr(address + 1);
                self.emit_lda_args(0);
                self.emit_sta_addr(address);
            }
            _ => return Err("only byte and word array stores are supported".to_string()),
        }
        Ok(())
    }

    pub(super) fn emit_array_assignment_args_to_inline_indexed(
        &mut self,
        address: u16,
        width: u16,
    ) -> Result<(), String> {
        match width {
            1 => {
                self.emit_lda_args(0);
                self.emit_sta_addr_x(address);
            }
            2 => {
                self.emit_lda_args(1);
                self.emit_sta_addr_x(address + 1);
                self.emit_lda_args(0);
                self.emit_sta_addr_x(address);
            }
            _ => return Err("only byte and word array stores are supported".to_string()),
        }
        Ok(())
    }

    pub(super) fn materialize_record_field_address(
        &mut self,
        access: NativeRecordFieldAccess<'_>,
        dest: NativeAddressDestination,
    ) -> Result<u16, String> {
        let base = self.classifier().required_lvalue_slot(access.base)?;
        if base.width != 2 || base.record.is_none() {
            return Err("record field base must be a record pointer".to_string());
        }
        let field = self.native_record_field_layout(&base, access.field)?;
        if field.width != access.width {
            return Err("record field width mismatch".to_string());
        }
        match dest {
            NativeAddressDestination::ArrayAddr => {
                self.materialize_pointer_offset_to_array_addr(base.address, field.offset)
            }
        }
        Ok(field.width)
    }

    pub(super) fn emit_word_literal_to_target(&mut self, value: u16, target: NativeResolvedSlot) {
        let high = (value >> 8) as u8;
        let low = (value & 0x00FF) as u8;
        if high == low && high <= 1 {
            self.emit_ldy_imm(high);
            self.emit_sty_addr(target.address + 1);
            self.emit_sty_addr(target.address);
        } else {
            self.emit_lda_imm(high);
            self.emit_sta_addr(target.address + 1);
            self.emit_lda_imm(low);
            self.emit_sta_addr(target.address);
        }
    }

    pub(super) fn materialize_word_source_to_target(
        &mut self,
        source: NativeWordSource,
        target: NativeResolvedSlot,
    ) -> Result<bool, String> {
        if let (NativeByteSource::Immediate(low), NativeByteSource::Immediate(high)) =
            (source.low, source.high)
        {
            self.emit_word_literal_to_target(u16::from_le_bytes([low, high]), target);
            return Ok(true);
        }

        self.materialize_byte_source_to_register(source.high, NativeByteRegister::A)?;
        self.emit_sta_addr(target.address + 1);
        self.materialize_byte_source_to_register(source.low, NativeByteRegister::A)?;
        self.emit_sta_addr(target.address);
        Ok(true)
    }

    pub(super) fn materialize_byte_source_to_register(
        &mut self,
        source: NativeByteSource,
        register: NativeByteRegister,
    ) -> Result<(), String> {
        match source {
            NativeByteSource::Immediate(byte) => match register {
                NativeByteRegister::A => self.emit_lda_imm(byte),
                NativeByteRegister::X => self.emit_ldx_imm(byte),
                NativeByteRegister::Y => self.emit_ldy_imm(byte),
            },
            NativeByteSource::Storage { address } => match register {
                NativeByteRegister::A => self.emit_lda_addr(address),
                NativeByteRegister::X => self.emit_ldx_addr(address),
                NativeByteRegister::Y => self.emit_ldy_addr(address),
            },
        }
        Ok(())
    }

    pub(super) fn materialize_call_arg_byte_to_a(
        &mut self,
        expr: &SemExpr,
        byte_index: u16,
    ) -> Result<(), String> {
        if let SemExprKind::Cast { expr, .. } = &expr.kind {
            return self.materialize_call_arg_byte_to_a(expr, byte_index);
        }
        if self.materialize_call_arg_byte_to_register(expr, byte_index, NativeByteRegister::A)? {
            return Ok(());
        }
        if byte_index == 0
            && expr_width(expr) == Some(1)
            && self.emit_pointer_index_expr_to_a(expr)?
        {
            return Ok(());
        }
        if byte_index == 0
            && expr_width(expr) == Some(1)
            && self.emit_array_index_expr_to_a(expr)?
        {
            return Ok(());
        }
        if let Some(slot) = self.classifier().addressable_slot(expr)? {
            if byte_index >= slot.width {
                return Err("call argument byte index is out of bounds".to_string());
            }
            self.emit_lda_addr(slot.address + byte_index);
            return Ok(());
        }
        if byte_index == 0 && expr_width(expr) == Some(1) {
            return self.emit_byte_expr_to_a(expr);
        }
        if byte_index > 0 && expr_width(expr) == Some(1) {
            self.emit_lda_imm(0);
            return Ok(());
        }
        let slot = self.classifier().required_addressable_slot(expr)?;
        if byte_index >= slot.width {
            return Err("call argument byte index is out of bounds".to_string());
        }
        self.emit_lda_addr(slot.address + byte_index);
        Ok(())
    }

    pub(super) fn materialize_word_call_arg_to_ax(&mut self, arg: &SemExpr) -> Result<(), String> {
        if let Some(address) = self.materialize_inline_string_literal_address(arg)? {
            self.emit_ldx_imm((address >> 8) as u8);
            self.emit_lda_imm((address & 0x00FF) as u8);
            return Ok(());
        }
        if self.materialize_byte_product_to_ax(arg)? {
            return Ok(());
        }
        if self.materialize_word_call_arg_to_args(arg)? {
            self.emit_ldx_args(1);
            self.emit_lda_args(0);
            return Ok(());
        }
        self.materialize_call_arg_byte_to_x(arg, 1)?;
        self.materialize_call_arg_byte_to_a(arg, 0)?;
        Ok(())
    }

    fn materialize_word_call_arg_to_args(&mut self, arg: &SemExpr) -> Result<bool, String> {
        if self.word_call_arg_bytes_can_materialize_direct(arg)? {
            return Ok(false);
        }
        let target = native_args_slot(2);
        if self.materialize_value_to_target(arg, target.clone())?
            || self.emit_expr_to_target(arg, target)?
        {
            return Ok(true);
        }
        Ok(false)
    }

    fn word_call_arg_bytes_can_materialize_direct(&self, arg: &SemExpr) -> Result<bool, String> {
        for byte_index in 0..2 {
            if self
                .classifier()
                .value_byte_source(arg, byte_index)?
                .is_some()
            {
                continue;
            }
            if let Some(slot) = self.classifier().addressable_slot(arg)?
                && byte_index < slot.width
            {
                continue;
            }
            return Ok(false);
        }
        Ok(true)
    }

    pub(super) fn materialize_byte_product_to_ax(&mut self, arg: &SemExpr) -> Result<bool, String> {
        if let SemExprKind::Cast { expr, .. } = &arg.kind {
            return self.materialize_byte_product_to_ax(expr);
        }
        let SemExprKind::Binary {
            op: BinaryOp::Mul,
            left,
            right,
        } = &arg.kind
        else {
            return Ok(false);
        };
        if expr_width(left) != Some(1) || expr_width(right) != Some(1) {
            return Ok(false);
        }

        if !self.emit_expr_to_a(left)? {
            return Err("only byte multiplication left operands are supported".to_string());
        }
        self.emit_pha();
        if !self.emit_expr_to_a(right)? {
            return Err("only byte multiplication right operands are supported".to_string());
        }
        self.emit_sta_afcur();
        self.emit_lda_imm(0);
        self.emit_sta_afcur_high();
        self.emit_pla();
        self.emit_ldx_imm(0);
        self.emit_jsr_runtime_helper(
            self.runtime_helpers.target(RuntimeHelperSlot::Mul),
            arg.span,
        );
        Ok(true)
    }

    pub(super) fn materialize_sargs_call_args(
        &mut self,
        arg_bytes: &[NativeCallArgByte<'_>],
    ) -> Result<(), String> {
        let mut inline_addresses = vec![None; arg_bytes.len()];
        let mut staged_slots = vec![None; arg_bytes.len()];
        let mut group_start = 0usize;
        while group_start < arg_bytes.len() {
            let expr = arg_bytes[group_start].expr;
            let mut group_end = group_start + 1;
            while group_end < arg_bytes.len() && std::ptr::eq(arg_bytes[group_end].expr, expr) {
                group_end += 1;
            }
            if let Some(address) = self.materialize_inline_string_literal_address(expr)? {
                inline_addresses[group_start..group_end].fill(Some(address));
            } else if self
                .materialize_sargs_call_arg_group_to_args(&arg_bytes[group_start..group_end])?
            {
                for (index, arg) in arg_bytes[group_start..group_end].iter().enumerate() {
                    staged_slots[group_start + index] =
                        Some(native_args_offset_slot(arg.offset, 1)?);
                }
            }
            group_start = group_end;
        }
        for (index, arg) in arg_bytes
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, arg)| arg.offset >= 3)
        {
            self.materialize_sargs_call_arg_byte_to_a(
                *arg,
                inline_addresses[index],
                staged_slots[index].clone(),
            )?;
            let offset = u8::try_from(arg.offset)
                .map_err(|_| "call argument offset is out of range".to_string())?;
            self.emit_sta_args(offset);
        }
        if let Some((index, arg)) = arg_bytes
            .iter()
            .enumerate()
            .find(|(_, arg)| arg.offset == 2)
        {
            self.materialize_sargs_call_arg_byte_to_y(
                *arg,
                inline_addresses[index],
                staged_slots[index].clone(),
            )?;
        }
        if let Some((index, arg)) = arg_bytes
            .iter()
            .enumerate()
            .find(|(_, arg)| arg.offset == 1)
        {
            self.materialize_sargs_call_arg_byte_to_x(
                *arg,
                inline_addresses[index],
                staged_slots[index].clone(),
            )?;
        }
        if let Some((index, arg)) = arg_bytes
            .iter()
            .enumerate()
            .find(|(_, arg)| arg.offset == 0)
        {
            self.materialize_sargs_call_arg_byte_to_a(
                *arg,
                inline_addresses[index],
                staged_slots[index].clone(),
            )?;
        }
        Ok(())
    }

    fn materialize_sargs_call_arg_group_to_args(
        &mut self,
        arg_bytes: &[NativeCallArgByte<'_>],
    ) -> Result<bool, String> {
        if arg_bytes.len() != 2 {
            return Ok(false);
        }
        let low = arg_bytes[0];
        let high = arg_bytes[1];
        if low.byte_index != 0
            || high.byte_index != 1
            || high.offset != low.offset.saturating_add(1)
            || self.sargs_call_arg_byte_can_materialize_direct(low)?
            || self.sargs_call_arg_byte_can_materialize_direct(high)?
            || self.expr_contains_routine_call(low.expr)
        {
            return Ok(false);
        }
        let target = native_args_offset_slot(low.offset, 2)?;
        if self.materialize_value_to_target(low.expr, target.clone())?
            || self.emit_expr_to_target(low.expr, target)?
        {
            return Ok(true);
        }
        Ok(false)
    }

    fn sargs_call_arg_byte_can_materialize_direct(
        &self,
        arg: NativeCallArgByte<'_>,
    ) -> Result<bool, String> {
        if self
            .classifier()
            .value_byte_source(arg.expr, arg.byte_index)?
            .is_some()
        {
            return Ok(true);
        }
        if let Some(slot) = self.classifier().addressable_slot(arg.expr)?
            && arg.byte_index < slot.width
        {
            return Ok(true);
        }
        Ok(arg.byte_index == 0 && expr_width(arg.expr) == Some(1))
    }

    fn materialize_staged_call_arg_byte_to_register(
        &mut self,
        staged_slot: Option<NativeResolvedSlot>,
        register: NativeByteRegister,
    ) -> bool {
        let Some(slot) = staged_slot else {
            return false;
        };
        match register {
            NativeByteRegister::A => self.emit_lda_addr(slot.address),
            NativeByteRegister::X => self.emit_ldx_addr(slot.address),
            NativeByteRegister::Y => self.emit_ldy_addr(slot.address),
        }
        true
    }

    fn materialize_sargs_call_arg_byte_to_a(
        &mut self,
        arg: NativeCallArgByte<'_>,
        inline_address: Option<u16>,
        staged_slot: Option<NativeResolvedSlot>,
    ) -> Result<(), String> {
        if self.materialize_staged_call_arg_byte_to_register(staged_slot, NativeByteRegister::A) {
            return Ok(());
        }
        if let Some(address) = inline_address {
            self.emit_lda_imm(inline_address_byte(address, arg.byte_index)?);
            return Ok(());
        }
        if arg.width == 1 && arg.byte_index == 0 {
            return self.emit_byte_expr_to_a(arg.expr);
        }
        self.materialize_call_arg_byte_to_a(arg.expr, arg.byte_index)
    }

    fn materialize_sargs_call_arg_byte_to_x(
        &mut self,
        arg: NativeCallArgByte<'_>,
        inline_address: Option<u16>,
        staged_slot: Option<NativeResolvedSlot>,
    ) -> Result<(), String> {
        if self.materialize_staged_call_arg_byte_to_register(staged_slot, NativeByteRegister::X) {
            return Ok(());
        }
        if let Some(address) = inline_address {
            self.emit_ldx_imm(inline_address_byte(address, arg.byte_index)?);
            return Ok(());
        }
        if arg.width == 1 && arg.byte_index == 0 {
            return self.emit_byte_expr_to_x(arg.expr);
        }
        self.materialize_call_arg_byte_to_x(arg.expr, arg.byte_index)
    }

    fn materialize_sargs_call_arg_byte_to_y(
        &mut self,
        arg: NativeCallArgByte<'_>,
        inline_address: Option<u16>,
        staged_slot: Option<NativeResolvedSlot>,
    ) -> Result<(), String> {
        if self.materialize_staged_call_arg_byte_to_register(staged_slot, NativeByteRegister::Y) {
            return Ok(());
        }
        if let Some(address) = inline_address {
            self.emit_ldy_imm(inline_address_byte(address, arg.byte_index)?);
            return Ok(());
        }
        if arg.width == 1 && arg.byte_index == 0 {
            return self.emit_byte_expr_to_y(arg.expr);
        }
        self.materialize_call_arg_byte_to_y(arg.expr, arg.byte_index)
    }

    pub(super) fn materialize_call_arg_byte_to_x(
        &mut self,
        expr: &SemExpr,
        byte_index: u16,
    ) -> Result<(), String> {
        if let SemExprKind::Cast { expr, .. } = &expr.kind {
            return self.materialize_call_arg_byte_to_x(expr, byte_index);
        }
        if self.materialize_call_arg_byte_to_register(expr, byte_index, NativeByteRegister::X)? {
            return Ok(());
        }
        if byte_index == 0 && expr_width(expr) == Some(1) {
            return self.emit_byte_expr_to_x(expr);
        }
        if byte_index > 0 && expr_width(expr) == Some(1) {
            self.emit_ldx_imm(0);
            return Ok(());
        }
        let slot = self.classifier().required_addressable_slot(expr)?;
        if byte_index >= slot.width {
            return Err("call argument byte index is out of bounds".to_string());
        }
        self.emit_ldx_addr(slot.address + byte_index);
        Ok(())
    }

    pub(super) fn materialize_call_arg_byte_to_y(
        &mut self,
        expr: &SemExpr,
        byte_index: u16,
    ) -> Result<(), String> {
        if let SemExprKind::Cast { expr, .. } = &expr.kind {
            return self.materialize_call_arg_byte_to_y(expr, byte_index);
        }
        if self.materialize_call_arg_byte_to_register(expr, byte_index, NativeByteRegister::Y)? {
            return Ok(());
        }
        if byte_index == 0 && expr_width(expr) == Some(1) {
            return self.emit_byte_expr_to_y(expr);
        }
        if byte_index > 0 && expr_width(expr) == Some(1) {
            self.emit_ldy_imm(0);
            return Ok(());
        }
        let slot = self.classifier().required_addressable_slot(expr)?;
        if byte_index >= slot.width {
            return Err("call argument byte index is out of bounds".to_string());
        }
        self.emit_ldy_addr(slot.address + byte_index);
        Ok(())
    }

    fn materialize_call_arg_byte_to_register(
        &mut self,
        expr: &SemExpr,
        byte_index: u16,
        register: NativeByteRegister,
    ) -> Result<bool, String> {
        let Some(source) = self.classifier().value_byte_source(expr, byte_index)? else {
            return Ok(false);
        };
        self.materialize_byte_source_to_register(source, register)?;
        Ok(true)
    }

    pub(super) fn apply_adc_byte_source(&mut self, source: NativeByteSource) {
        match source {
            NativeByteSource::Immediate(byte) => self.emit_adc_imm(byte),
            NativeByteSource::Storage { address } => self.emit_adc_addr(address),
        }
    }

    pub(super) fn apply_sbc_byte_source(&mut self, source: NativeByteSource) {
        match source {
            NativeByteSource::Immediate(byte) => self.emit_sbc_imm(byte),
            NativeByteSource::Storage { address } => self.emit_sbc_addr(address),
        }
    }

    pub(super) fn apply_logic_byte_source(&mut self, op: BinaryOp, source: NativeByteSource) {
        match (op, source) {
            (BinaryOp::And, NativeByteSource::Immediate(byte)) => self.emit_and_imm(byte),
            (BinaryOp::Or, NativeByteSource::Immediate(byte)) => self.emit_ora_imm(byte),
            (BinaryOp::Xor, NativeByteSource::Immediate(byte)) => self.emit_eor_imm(byte),
            (BinaryOp::And, NativeByteSource::Storage { address }) => self.emit_and_addr(address),
            (BinaryOp::Or, NativeByteSource::Storage { address }) => self.emit_ora_addr(address),
            (BinaryOp::Xor, NativeByteSource::Storage { address }) => self.emit_eor_addr(address),
            _ => unreachable!("logic operator checked by caller"),
        }
    }
}

fn native_addressed_lvalue(expr: &SemExpr) -> Option<&SemLValue> {
    match &expr.kind {
        SemExprKind::Cast { expr, .. } => native_addressed_lvalue(expr),
        SemExprKind::AddressOf(addressed) => Some(addressed),
        SemExprKind::ImplicitAddressOf(address) => Some(&address.place),
        _ => None,
    }
}

fn inline_address_byte(address: u16, byte_index: u16) -> Result<u8, String> {
    match byte_index {
        0 => Ok((address & 0x00FF) as u8),
        1 => Ok((address >> 8) as u8),
        _ => Err("inline address byte index is out of bounds".to_string()),
    }
}
