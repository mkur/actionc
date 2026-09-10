use super::*;

fn codegen_symbol_scope_key(scope: &CodegenSymbolScope) -> (&str, &str) {
    match scope {
        CodegenSymbolScope::Global => ("", ""),
        CodegenSymbolScope::Routine(name) => ("routine", name.as_str()),
    }
}

impl Generator {
    pub(super) fn finish_with_runtime_requirements(
        mut self,
    ) -> Result<(CodegenOutput, Vec<String>), Vec<Diagnostic>> {
        if !self.diagnostics.is_empty() {
            return Err(self.diagnostics);
        }

        for helper in self.used_default_runtime_helpers.iter().copied().filter(|helper| helper.is_owned_division()) {
            self.emitter.bind_label(helper.owned_label(), Span::new(0, 0))
                .map_err(|error| vec![error])?;
            let body = crate::integer6502::division_body(
                matches!(helper, RuntimeHelperSlot::Div | RuntimeHelperSlot::Mod),
                matches!(helper, RuntimeHelperSlot::Mod | RuntimeHelperSlot::UMod),
            );
            for (offset, byte) in body.bytes.into_iter().enumerate() {
                if offset == body.error_operand {
                    match &self.runtime_error_target {
                        RuntimeHelperTarget::Absolute(address) => {
                            self.emitter.emit_u16_le(address.address());
                        }
                        RuntimeHelperTarget::Label(label) => {
                            self.emitter.emit_u16_label(label, Span::new(0, 0));
                        }
                    }
                } else if offset != body.error_operand + 1 {
                    self.emitter.emit_u8(byte);
                }
            }
            self.emitter.emit_u8(0x60);
        }

        for helper in &self.used_wide_helpers {
            self.emitter.bind_label(helper.label(), Span::new(0, 0)).map_err(|error| vec![error])?;
            let (bytes, error_operand) = helper.body();
            for (offset, byte) in bytes.into_iter().enumerate() {
                if Some(offset) == error_operand {
                    match &self.runtime_error_target {
                        RuntimeHelperTarget::Absolute(address) => self.emitter.emit_u16_le(address.address()),
                        RuntimeHelperTarget::Label(label) => self.emitter.emit_u16_label(label, Span::new(0, 0)),
                    }
                } else if !error_operand.is_some_and(|operand| offset == operand + 1) {
                    self.emitter.emit_u8(byte);
                }
            }
            self.emitter.emit_u8(0x60);
        }

        // Uninitialized array homes follow every executable helper. Binding
        // them before helper emission makes input buffers overwrite helper code.
        if self.segment_storage { self.emit_array_backing_storage(); }

        let origin = self.emitter.origin;
        let run_address = self
            .program_entry_label
            .as_ref()
            .and_then(|label| self.emitter.labels.get(label))
            .map_or(origin, |offset| origin.wrapping_add(*offset as u16));
        let skipped_ranges = self.skipped_ranges;
        let routine_addresses = self.routine_addresses;
        let routine_ranges = self.routine_ranges;
        let routine_signatures = self.routine_signatures;
        let source_ranges = self.source_ranges;
        let routine_effects = routine_ranges
            .iter()
            .filter_map(|range| {
                self.routines
                    .get(&normalize_name(&range.name))
                    .and_then(|info| format_trusted_routine_effect_summary(info.effects))
                    .map(|summary| CodegenRoutineEffect {
                        routine: range.name.clone(),
                        summary,
                    })
            })
            .collect::<Vec<_>>();
        let machine_blocks = self.machine_blocks;
        let optimizations = self.optimizations;
        let proofs = self.proofs;
        let proof_attempts = self.proof_attempts;
        let mut runtime_bindings = self
            .used_atari_fpp_services
            .iter()
            .copied()
            .map(|service| CodegenRuntimeBinding {
                helper: format!("ATARI_FPP_{}", service.name()),
                implementation: format!("Atari OS FPP {}", service.name()),
                address: Some(service.address()),
                reason: "native REAL arithmetic/conversion".to_string(),
                origin: "Atari OS ROM".to_string(),
                suppressed_default: None,
                kind: CodegenRuntimeBindingKind::AtariFpp,
                license: None,
            })
            .collect::<Vec<_>>();
        runtime_bindings.extend(self.used_default_runtime_helpers.iter().copied()
            .filter(|helper| helper.is_owned_division())
            .map(|helper| CodegenRuntimeBinding {
                helper: helper.name().to_string(),
                implementation: helper.owned_label(),
                address: self.emitter.labels.get(&helper.owned_label())
                    .map(|offset| origin.wrapping_add(*offset as u16)),
                reason: "modern integer division/remainder".to_string(),
                origin: "compiler-owned 6502 arithmetic".to_string(),
                suppressed_default: None,
                kind: CodegenRuntimeBindingKind::CompilerHelper,
                license: None,
            }));
        runtime_bindings.extend(self.used_wide_helpers.iter().map(|helper| CodegenRuntimeBinding {
            helper: format!("{helper:?}32"), implementation: helper.label(),
            address: self.emitter.labels.get(&helper.label()).map(|offset| origin.wrapping_add(*offset as u16)),
            reason: "32-bit integer legalization".into(), origin: "compiler-owned 6502 arithmetic".into(),
            suppressed_default: None, kind: CodegenRuntimeBindingKind::CompilerHelper, license: None,
        }));
        let mut classic_runtime_requirements: Vec<String> = self.used_default_runtime_helpers
            .iter()
            .map(|helper| {
                if helper.is_owned_division() { "Error" } else { helper.name() }.to_string()
            })
            .collect();
        if self.uses_runtime_fault && !classic_runtime_requirements.iter().any(|name| name == "Error") {
            classic_runtime_requirements.push("Error".into());
        }
        let mut storage_symbols = self.layout.codegen_storage_symbols();
        storage_symbols.extend(self.storage_symbols);
        storage_symbols.sort_by(|left, right| {
            codegen_symbol_scope_key(&left.scope)
                .cmp(&codegen_symbol_scope_key(&right.scope))
                .then_with(|| left.name.cmp(&right.name))
        });
        let map = CodegenMap {
            runtime: crate::runtime::Runtime::ActionCart,
            runtime_bindings,
            origin,
            run_address,
            skipped_ranges: skipped_ranges.clone(),
            routine_addresses: routine_addresses.clone(),
            routine_ranges,
            routine_signatures,
            storage_symbols,
            source_ranges,
            routine_effects,
            machine_blocks,
            optimizations: optimizations.clone(),
            proofs: proofs.clone(),
            proof_attempts: proof_attempts.clone(),
        };
        self.emitter.finish_with_relocations().map(|emission| {
            (
                CodegenOutput {
                    bytes: emission.bytes,
                    origin,
                    run_address,
                    relocations: emission.relocations,
                    skipped_ranges,
                    routine_addresses,
                    optimizations,
                    proofs,
                    proof_attempts,
                    map,
                },
                classic_runtime_requirements,
            )
        })
    }
}

fn format_trusted_routine_effect_summary(effects: RoutineEffects) -> Option<String> {
    if !effects.known {
        return None;
    }
    let mut parts = Vec::new();
    let mut clobbered_registers = Vec::new();
    if !effects.preserves_a {
        clobbered_registers.push("A");
    }
    if !effects.preserves_x {
        clobbered_registers.push("X");
    }
    if !effects.preserves_y {
        clobbered_registers.push("Y");
    }
    if clobbered_registers.len() < 3 {
        let preserved = [
            effects.preserves_a.then_some("A"),
            effects.preserves_x.then_some("X"),
            effects.preserves_y.then_some("Y"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        if !preserved.is_empty() {
            parts.push(format!("preserves {}", preserved.join(",")));
        }
    }
    if !clobbered_registers.is_empty() {
        parts.push(format!("clobbers {}", clobbered_registers.join(",")));
    }
    let zero_page_writes = format_zero_page_writes(effects);
    if !zero_page_writes.is_empty() {
        parts.push(format!("writes zp {}", zero_page_writes.join(",")));
    }
    let absolute_writes = format_absolute_writes(effects);
    if !absolute_writes.is_empty() {
        parts.push(format!("writes abs {}", absolute_writes.join(",")));
    }
    if effects.writes_unknown_absolute {
        parts.push("writes unknown-abs".to_string());
    }
    if parts.is_empty() {
        parts.push("no memory writes; clobbers A,X,Y".to_string());
    }
    Some(parts.join("; "))
}

fn format_absolute_writes(effects: RoutineEffects) -> Vec<String> {
    effects
        .absolute_writes
        .iter()
        .flatten()
        .map(|range| {
            if range.size <= 1 {
                format!("${:04X}", range.address)
            } else {
                format!(
                    "${:04X}-${:04X}",
                    range.address,
                    range.address.wrapping_add(range.size - 1)
                )
            }
        })
        .collect()
}

pub(super) fn format_zero_page_writes(effects: RoutineEffects) -> Vec<String> {
    let mut writes = Vec::new();
    for address in 0u16..=0xFF {
        if effects.writes_zero_page(ZeroPage::new(address as u8)) {
            writes.push(format!("${address:02X}"));
        }
    }
    writes
}

pub fn format_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn format_load_file(output: &CodegenOutput) -> Vec<u8> {
    let mut load = Vec::new();
    let end = output
        .origin
        .wrapping_add(output.bytes.len().saturating_sub(1) as u16);

    load.extend([0xFF, 0xFF]);
    load.extend(output.origin.to_le_bytes());
    load.extend(end.to_le_bytes());
    load.extend(&output.bytes);
    load.extend(RUNAD.to_le_bytes());
    load.extend(RUNAD.wrapping_add(1).to_le_bytes());
    load.extend(output.run_address.to_le_bytes());
    load
}
