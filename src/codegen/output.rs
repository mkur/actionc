use super::*;

fn codegen_symbol_scope_key(scope: &CodegenSymbolScope) -> (&str, &str) {
    match scope {
        CodegenSymbolScope::Global => ("", ""),
        CodegenSymbolScope::Routine(name) => ("routine", name.as_str()),
    }
}

impl Generator {
    pub(super) fn finish(self) -> Result<CodegenOutput, Vec<Diagnostic>> {
        if !self.diagnostics.is_empty() {
            return Err(self.diagnostics);
        }

        let origin = self.emitter.origin;
        let main_run_address = self.emitter.labels.iter().find_map(|(label, offset)| {
            label
                .strip_prefix("routine:")
                .filter(|name| normalize_name(name) == "MAIN")
                .map(|_| origin.wrapping_add(*offset as u16))
        });
        let last_run_address = self
            .last_routine_label
            .as_ref()
            .and_then(|label| self.emitter.labels.get(label))
            .map(|offset| origin.wrapping_add(*offset as u16));
        let run_address = main_run_address.or(last_run_address).unwrap_or(origin);
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
        let mut storage_symbols = self.layout.codegen_storage_symbols();
        storage_symbols.extend(self.storage_symbols);
        storage_symbols.sort_by(|left, right| {
            codegen_symbol_scope_key(&left.scope)
                .cmp(&codegen_symbol_scope_key(&right.scope))
                .then_with(|| left.name.cmp(&right.name))
        });
        let map = CodegenMap {
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
        self.emitter.finish().map(|bytes| CodegenOutput {
            bytes,
            origin,
            run_address,
            skipped_ranges,
            routine_addresses,
            optimizations,
            proofs,
            proof_attempts,
            map,
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
