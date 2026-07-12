use super::*;

// Extracted from src/codegen.rs: optimization log model
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodegenOptimization {
    pub kind: CodegenOptimizationKind,
    pub profile: CodegenProfile,
    pub routine: Option<String>,
    pub source_span: Option<Span>,
    pub address: Option<u16>,
    pub bytes_saved: i16,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodegenOptimizationKind {
    TrampolineElided,
    FinalRtsRemoved,
    RegisterReloadRemoved,
    ConstantStoreReusedRegister,
    CallResultMaterializationRemoved,
    PointerReloadRemoved,
    EffectiveAddressLowered,
    EffectiveAddressReused,
    ArgumentStoreRemoved,
    ArgumentStackForwarded,
    BranchInverted,
    TailCall,
    JumpToRtsRemoved,
    CallFactPreserved,
}

// Extracted from src/codegen.rs: optimization cleanup helpers
pub(super) fn invert_branch_opcode(opcode: u8) -> Option<u8> {
    match opcode {
        opcode::BEQ_REL => Some(opcode::BNE_REL),
        opcode::BNE_REL => Some(opcode::BEQ_REL),
        opcode::BCC_REL => Some(opcode::BCS_REL),
        opcode::BCS_REL => Some(opcode::BCC_REL),
        opcode::BMI_REL => Some(opcode::BPL_REL),
        opcode::BPL_REL => Some(opcode::BMI_REL),
        opcode::BVS_REL => Some(opcode::BVC_REL),
        opcode::BVC_REL => Some(opcode::BVS_REL),
        _ => None,
    }
}

fn is_relative_branch_opcode(opcode: u8) -> bool {
    matches!(
        opcode,
        opcode::BPL_REL
            | opcode::BMI_REL
            | opcode::BCC_REL
            | opcode::BCS_REL
            | opcode::BVC_REL
            | opcode::BVS_REL
            | opcode::BNE_REL
            | opcode::BEQ_REL
    )
}

pub(super) fn adjust_position_after_delete(position: &mut usize, start: usize, len: usize) {
    let end = start + len;
    if *position >= end {
        *position -= len;
    } else if *position > start {
        *position = start;
    }
}

pub(super) fn adjust_address_after_delete(address: &mut u16, start: u16, len: u16) {
    let end = start.wrapping_add(len);
    if *address >= end {
        *address = address.wrapping_sub(len);
    } else if *address > start {
        *address = start;
    }
}

pub(super) fn adjust_range_after_delete(
    start_addr: &mut u16,
    end_addr: &mut u16,
    start: u16,
    len: u16,
) {
    adjust_address_after_delete(start_addr, start, len);
    adjust_address_after_delete(end_addr, start, len);
    if *end_addr < *start_addr {
        *end_addr = *start_addr;
    }
}

// Extracted from src/codegen.rs: branch inversion candidate
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BranchInversionCandidate {
    pub(super) branch_start: usize,
    pub(super) branch_opcode: u8,
    pub(super) true_label: String,
    pub(super) false_label: String,
    pub(super) span: Span,
}

impl Generator {
    // Extracted from src/codegen.rs: optimization logging
    pub(super) fn record_modern_optimization(
        &mut self,
        kind: CodegenOptimizationKind,
        bytes_saved: i16,
        source_span: Option<Span>,
        message: impl Into<String>,
    ) {
        if self.profile != CodegenProfile::Modern {
            return;
        }
        self.optimizations.push(CodegenOptimization {
            kind,
            profile: self.profile,
            routine: self.current_routine_name(),
            source_span,
            address: Some(self.current_absolute_address()),
            bytes_saved,
            message: message.into(),
        });
    }

    pub(super) fn record_codegen_proof(
        &mut self,
        kind: impl Into<String>,
        source_span: Span,
        summary: impl Into<String>,
    ) {
        if self.profile != CodegenProfile::Modern {
            return;
        }
        let kind = kind.into();
        let summary = summary.into();
        self.record_codegen_proof_attempt(kind.clone(), source_span, true, summary.clone());
        self.proofs.push(CodegenProof {
            routine: self.current_routine_name(),
            source_span,
            address: Some(self.current_absolute_address()),
            kind,
            summary,
        });
    }

    pub(super) fn record_codegen_proof_rejection(
        &mut self,
        kind: impl Into<String>,
        source_span: Span,
        summary: impl Into<String>,
    ) {
        self.record_codegen_proof_attempt(kind, source_span, false, summary);
    }

    fn record_codegen_proof_attempt(
        &mut self,
        kind: impl Into<String>,
        source_span: Span,
        accepted: bool,
        summary: impl Into<String>,
    ) {
        if self.profile != CodegenProfile::Modern {
            return;
        }
        self.proof_attempts.push(CodegenProofAttempt {
            routine: self.current_routine_name(),
            source_span,
            address: Some(self.current_absolute_address()),
            kind: kind.into(),
            accepted,
            summary: summary.into(),
        });
    }

    pub(super) fn current_routine_name(&self) -> Option<String> {
        self.last_routine_label
            .as_ref()
            .and_then(|label| label.strip_prefix("routine:"))
            .map(str::to_string)
    }

    // Extracted from src/codegen.rs: branch inversion cleanup
    pub(super) fn maybe_record_branch_inversion_candidate(
        &mut self,
        branch_start: usize,
        true_label: &str,
        false_label: &str,
        span: Span,
    ) {
        if !self.profile.enables_modern_optimizations() {
            return;
        }
        if self.emitter.position() != branch_start + 5 {
            return;
        }
        let Some(&branch_opcode) = self.emitter.bytes.get(branch_start) else {
            return;
        };
        if invert_branch_opcode(branch_opcode).is_none()
            || self.emitter.bytes.get(branch_start + 2) != Some(&opcode::JMP_ABS)
        {
            return;
        }
        if self
            .emitter
            .labels
            .iter()
            .any(|(label, position)| *position == branch_start + 2 && label != true_label)
        {
            return;
        }
        let has_relative_branch_patch = self.emitter.patches.iter().any(|patch| {
            patch.offset == branch_start + 1
                && patch.kind == PatchKind::Relative8
                && patch.label == true_label
        });
        let has_absolute_jump_patch = self.emitter.patches.iter().any(|patch| {
            patch.offset == branch_start + 3
                && patch.kind == PatchKind::Absolute16
                && patch.label == false_label
        });
        if !has_relative_branch_patch || !has_absolute_jump_patch {
            return;
        }
        self.branch_inversion_candidates
            .push(BranchInversionCandidate {
                branch_start,
                branch_opcode,
                true_label: true_label.to_string(),
                false_label: false_label.to_string(),
                span,
            });
    }

    pub(super) fn try_invert_branch_to_label(&mut self, label: &str) {
        if !self.profile.enables_modern_optimizations() {
            return;
        }
        let Some(index) = self
            .branch_inversion_candidates
            .iter()
            .position(|candidate| candidate.false_label == label)
        else {
            return;
        };
        let candidate = self.branch_inversion_candidates.remove(index);
        let target_after_deletion = self.emitter.position().saturating_sub(3);
        let branch_origin = candidate.branch_start + 2;
        let delta = target_after_deletion as isize - branch_origin as isize;
        if !(-128..=127).contains(&delta) {
            return;
        }
        let Some(inverted_opcode) = invert_branch_opcode(candidate.branch_opcode) else {
            return;
        };

        self.emitter.bytes[candidate.branch_start] = inverted_opcode;
        self.emitter.bytes[candidate.branch_start + 1] = delta as i8 as u8;
        self.emitter.patches.retain(|patch| {
            patch.offset != candidate.branch_start + 1 && patch.offset != candidate.branch_start + 3
        });
        self.delete_emitted_bytes(candidate.branch_start + 2, 3);
        self.optimizations.push(CodegenOptimization {
            kind: CodegenOptimizationKind::BranchInverted,
            profile: self.profile,
            routine: self.current_routine_name(),
            source_span: Some(candidate.span),
            address: Some(
                self.emitter
                    .origin
                    .wrapping_add(candidate.branch_start as u16),
            ),
            bytes_saved: 3,
            message: format!(
                "inverted branch to {} and removed long jump to {}",
                candidate.true_label, candidate.false_label
            ),
        });
    }

    pub(super) fn delete_emitted_bytes(&mut self, start: usize, len: usize) {
        let end = start + len;
        self.emitter.bytes.drain(start..end);
        for position in self.emitter.labels.values_mut() {
            adjust_position_after_delete(position, start, len);
        }
        for patch in &mut self.emitter.patches {
            adjust_position_after_delete(&mut patch.offset, start, len);
        }
        for candidate in &mut self.branch_inversion_candidates {
            adjust_position_after_delete(&mut candidate.branch_start, start, len);
        }
        self.adjust_recorded_addresses_after_delete(start, len);
    }

    fn resolved_relative_branch_crosses_delete(&self, start: usize, len: usize) -> bool {
        let end = start + len;
        for branch_start in 0..self.emitter.bytes.len().saturating_sub(1) {
            let opcode = self.emitter.bytes[branch_start];
            if !is_relative_branch_opcode(opcode) {
                continue;
            }
            if (start..end).contains(&branch_start) {
                return true;
            }
            if self
                .emitter
                .patches
                .iter()
                .any(|patch| patch.kind == PatchKind::Relative8 && patch.offset == branch_start + 1)
            {
                continue;
            }
            let old_operand = self.emitter.bytes[branch_start + 1] as i8 as isize;
            let old_target = branch_start as isize + 2 + old_operand;
            if old_target < 0 {
                continue;
            }
            let target = old_target as usize;
            if (start..end).contains(&target)
                || (branch_start < start && target >= end)
                || (target < start && branch_start >= end)
            {
                return true;
            }
        }
        false
    }

    pub(super) fn adjust_recorded_addresses_after_delete(&mut self, start: usize, len: usize) {
        let start = self.emitter.origin.wrapping_add(start as u16);
        let len = len as u16;
        for routine in &mut self.routine_addresses {
            adjust_address_after_delete(&mut routine.address, start, len);
        }
        for range in &mut self.routine_ranges {
            adjust_range_after_delete(&mut range.start, &mut range.end, start, len);
        }
        for range in &mut self.source_ranges {
            adjust_range_after_delete(&mut range.start, &mut range.end, start, len);
        }
        for range in &mut self.skipped_ranges {
            adjust_address_after_delete(&mut range.start, start, len);
        }
        for optimization in &mut self.optimizations {
            if let Some(address) = &mut optimization.address {
                adjust_address_after_delete(address, start, len);
            }
        }
        if let Some(cursor) = &mut self.compatible_cursor {
            adjust_address_after_delete(cursor, start, len);
        }
        adjust_address_after_delete(&mut self.deferred_output_cursor, start, len);
    }

    // Extracted from src/codegen.rs: tail-call routine body
    pub(super) fn emit_modern_tail_call_routine_body(&mut self, routine: &Routine) -> bool {
        if !self.profile.enables_modern_optimizations() || routine.body.is_empty() {
            return false;
        }
        let Some((tail_stmt, prefix)) = routine.body.split_last() else {
            return false;
        };
        let Stmt::Call { expr, span } = tail_stmt else {
            return false;
        };
        let ExprKind::Call { callee, args } = &expr.kind else {
            return false;
        };
        if !self.can_emit_call_target(callee, args) {
            return false;
        }

        self.generate_stmt_list(prefix);
        let start = self.current_absolute_address();
        if !self.emit_tail_call(callee, args, *span) {
            self.diagnostics.push(Diagnostic::new(
                *span,
                "codegen only supports user routine calls and numeric-address system calls",
            ));
            self.emit_return_rts(*span);
            return true;
        }
        self.record_modern_optimization(
            CodegenOptimizationKind::TailCall,
            1,
            Some(*span),
            format!(
                "lowered final call in {} at ${start:04X} to tail jump",
                routine.name
            ),
        );
        true
    }

    // Extracted from src/codegen.rs: routine entry planning
    pub(super) fn compatible_routine_entry_plan(
        &self,
        routine: &Routine,
        allocation: &RoutineAllocation,
    ) -> RoutineEntryPlan {
        if !self.profile.enables_modern_optimizations() {
            return RoutineEntryPlan::trampoline(RoutineTrampolineReason::CompatibleProfile);
        }
        if self
            .routine_assignment_targets
            .contains(&normalize_name(&routine.name))
        {
            return RoutineEntryPlan::trampoline(RoutineTrampolineReason::RetargetableRoutine);
        }
        if !allocation.initializers.is_empty() {
            return RoutineEntryPlan::trampoline(RoutineTrampolineReason::ExplicitStorage);
        }
        if !allocation.array_backings.is_empty() {
            return RoutineEntryPlan::trampoline(RoutineTrampolineReason::ArrayBackingStorage);
        }
        RoutineEntryPlan::direct()
    }

    // Extracted from src/codegen.rs: return cleanup
    pub(super) fn rewrite_jumps_to_current_rts(&mut self, span: Span) {
        if !self.profile.enables_modern_optimizations() {
            return;
        }
        while let Some((patch_index, patch)) = self.current_rts_jump_patch() {
            let jump_start = patch.offset - 1;
            self.emitter.bytes[jump_start] = opcode::RTS;
            self.emitter.patches.remove(patch_index);
            self.delete_emitted_bytes(patch.offset, 2);
            self.optimizations.push(CodegenOptimization {
                kind: CodegenOptimizationKind::JumpToRtsRemoved,
                profile: self.profile,
                routine: self.current_routine_name(),
                source_span: Some(span),
                address: Some(self.emitter.origin.wrapping_add(jump_start as u16)),
                bytes_saved: 2,
                message: format!(
                    "replaced JMP to return label {} at ${:04X} with RTS",
                    patch.label,
                    self.emitter.origin.wrapping_add(jump_start as u16)
                ),
            });
        }
    }

    pub(super) fn current_rts_jump_patch(&self) -> Option<(usize, Patch)> {
        let current = self.emitter.position();
        self.emitter
            .patches
            .iter()
            .enumerate()
            .filter(|(_, patch)| {
                patch.kind == PatchKind::Absolute16
                    && patch.offset > 0
                    && self.emitter.bytes.get(patch.offset - 1) == Some(&opcode::JMP_ABS)
                    && self
                        .emitter
                        .labels
                        .get(&patch.label)
                        .is_some_and(|position| *position == current)
                    && !self.resolved_relative_branch_crosses_delete(patch.offset, 2)
            })
            .max_by_key(|(_, patch)| patch.offset)
            .map(|(index, patch)| (index, patch.clone()))
    }

    pub(super) fn try_rewrite_jsr_rts_to_tail_jmp(&mut self, span: Span) -> bool {
        if !self.profile.enables_modern_optimizations() || self.emitter.bytes.len() < 3 {
            return false;
        }
        let current = self.emitter.position();
        if self
            .emitter
            .labels
            .values()
            .any(|position| *position == current)
        {
            return false;
        }
        let jsr_offset = self.emitter.bytes.len() - 3;
        if self.emitter.bytes[jsr_offset] != opcode::JSR_ABS {
            return false;
        }
        self.emitter.bytes[jsr_offset] = opcode::JMP_ABS;
        self.record_modern_optimization(
            CodegenOptimizationKind::TailCall,
            1,
            Some(span),
            format!(
                "rewrote trailing JSR at ${:04X} plus RTS to tail JMP",
                self.emitter.origin.wrapping_add(jsr_offset as u16)
            ),
        );
        true
    }

    // Extracted from src/codegen.rs: tail-call statement
    pub(super) fn emit_modern_tail_call_stmt(&mut self, stmt: &Stmt, next: Option<&Stmt>) -> bool {
        if !self.profile.enables_modern_optimizations() || !is_bare_return(next) {
            return false;
        }
        let Stmt::Call { expr, span } = stmt else {
            return false;
        };
        let ExprKind::Call { callee, args } = &expr.kind else {
            return false;
        };
        if !self.can_emit_call_target(callee, args) {
            return false;
        }
        let start = self.current_absolute_address();
        if !self.emit_tail_call(callee, args, *span) {
            return false;
        }
        self.record_modern_optimization(
            CodegenOptimizationKind::TailCall,
            1,
            Some(*span),
            format!("lowered final call at ${start:04X} to tail jump"),
        );
        true
    }
}
