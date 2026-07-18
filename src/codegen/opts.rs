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

#[derive(Debug, Clone, PartialEq, Eq)]
struct BranchOverJumpRewrite {
    branch_start: usize,
    branch_opcode: u8,
    fallthrough_label: String,
    target_label: String,
    span: Span,
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
                && patch.addend == 0
        });
        let has_absolute_jump_patch = self.emitter.patches.iter().any(|patch| {
            patch.offset == branch_start + 3
                && patch.kind == PatchKind::Absolute16
                && patch.label == false_label
                && patch.addend == 0
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

        let Some(relative_patch) = self.emitter.patches.iter_mut().find(|patch| {
            patch.offset == candidate.branch_start + 1
                && patch.kind == PatchKind::Relative8
                && patch.label == candidate.true_label
                && patch.addend == 0
        }) else {
            return;
        };
        relative_patch.label = candidate.false_label.clone();
        self.emitter.bytes[candidate.branch_start] = inverted_opcode;
        self.emitter.patches.retain(|patch| {
            !(patch.offset == candidate.branch_start + 3
                && patch.kind == PatchKind::Absolute16
                && patch.label == candidate.false_label
                && patch.addend == 0)
        });
        if self.emitter.labels.get(&candidate.true_label) == Some(&(candidate.branch_start + 5))
            && !self
                .emitter
                .patches
                .iter()
                .any(|patch| patch.label == candidate.true_label)
        {
            // Retargeting the only branch can orphan the synthetic fallthrough label on
            // the next instruction. Drop that private label so a later fixed-point
            // rewrite is not blocked by a label that no executable edge can reach.
            self.emitter.labels.remove(&candidate.true_label);
        }
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

    /// Revisit generated control flow after every label in the current routine is bound.
    ///
    /// Some statement lowerings emit their own `branch; JMP; fallthrough` shapes, and an
    /// eager branch inversion can expose another copy of that shape. Keep applying the
    /// structured rewrite until no emitter-owned patch/label pattern remains.
    pub(super) fn finalize_modern_branch_inversions(&mut self, routine_start: usize) {
        if !self.profile.enables_modern_optimizations() {
            self.branch_inversion_candidates
                .retain(|candidate| candidate.branch_start < routine_start);
            return;
        }

        while let Some(rewrite) = self.find_branch_over_jump_rewrite(routine_start) {
            if !self.apply_branch_over_jump_rewrite(rewrite) {
                break;
            }
        }

        // All labels in this routine are now bound. Candidates left behind by the eager
        // path cannot become actionable later and must not leak into the next routine.
        self.branch_inversion_candidates
            .retain(|candidate| candidate.branch_start < routine_start);
    }

    fn find_branch_over_jump_rewrite(&self, routine_start: usize) -> Option<BranchOverJumpRewrite> {
        let routine_end = self.emitter.position();
        let mut relative_patches = self
            .emitter
            .patches
            .iter()
            .filter(|patch| patch.kind == PatchKind::Relative8 && patch.addend == 0)
            .collect::<Vec<_>>();
        relative_patches.sort_by_key(|patch| patch.offset);

        for relative_patch in relative_patches {
            let Some(branch_start) = relative_patch.offset.checked_sub(1) else {
                continue;
            };
            if branch_start < routine_start || branch_start + 5 > routine_end {
                continue;
            }
            let Some(&branch_opcode) = self.emitter.bytes.get(branch_start) else {
                continue;
            };
            if invert_branch_opcode(branch_opcode).is_none()
                || self.emitter.bytes.get(branch_start + 2) != Some(&opcode::JMP_ABS)
            {
                continue;
            }
            if self.emitter.labels.get(&relative_patch.label) != Some(&(branch_start + 5)) {
                continue;
            }
            let Some(jump_patch) = self.emitter.patches.iter().find(|patch| {
                patch.offset == branch_start + 3
                    && patch.kind == PatchKind::Absolute16
                    && patch.addend == 0
            }) else {
                continue;
            };
            let Some(&target_position) = self.emitter.labels.get(&jump_patch.label) else {
                continue;
            };

            let delete_start = branch_start + 2;
            let delete_end = delete_start + 3;
            if self.emitter.labels.iter().any(|(label, position)| {
                (delete_start..delete_end).contains(position)
                    && !self.is_removable_synthetic_fallthrough_label(label)
            }) {
                continue;
            }
            if self.emitter.patches.iter().any(|patch| {
                (delete_start..delete_end).contains(&patch.offset)
                    && !(patch.offset == jump_patch.offset
                        && patch.kind == PatchKind::Absolute16
                        && patch.label == jump_patch.label
                        && patch.addend == 0)
            }) {
                continue;
            }
            if self.resolved_machine_branch_crosses_delete(delete_start, 3) {
                continue;
            }

            let mut target_after_deletion = target_position;
            adjust_position_after_delete(&mut target_after_deletion, delete_start, 3);
            let branch_origin = branch_start + 2;
            let delta = target_after_deletion as isize - branch_origin as isize;
            if !(-128..=127).contains(&delta) {
                continue;
            }

            return Some(BranchOverJumpRewrite {
                branch_start,
                branch_opcode,
                fallthrough_label: relative_patch.label.clone(),
                target_label: jump_patch.label.clone(),
                span: relative_patch.span,
            });
        }

        None
    }

    fn is_removable_synthetic_fallthrough_label(&self, label: &str) -> bool {
        (label.starts_with("compare:done:") || label.starts_with("condition:false:"))
            && !self
                .emitter
                .patches
                .iter()
                .any(|patch| patch.label == label)
    }

    fn apply_branch_over_jump_rewrite(&mut self, rewrite: BranchOverJumpRewrite) -> bool {
        let Some(inverted_opcode) = invert_branch_opcode(rewrite.branch_opcode) else {
            return false;
        };
        let Some(relative_patch) = self.emitter.patches.iter_mut().find(|patch| {
            patch.offset == rewrite.branch_start + 1
                && patch.kind == PatchKind::Relative8
                && patch.label == rewrite.fallthrough_label
                && patch.addend == 0
        }) else {
            return false;
        };
        relative_patch.label = rewrite.target_label.clone();

        let Some(jump_patch_index) = self.emitter.patches.iter().position(|patch| {
            patch.offset == rewrite.branch_start + 3
                && patch.kind == PatchKind::Absolute16
                && patch.label == rewrite.target_label
                && patch.addend == 0
        }) else {
            return false;
        };

        self.emitter.bytes[rewrite.branch_start] = inverted_opcode;
        self.emitter.patches.remove(jump_patch_index);
        let delete_start = rewrite.branch_start + 2;
        let removable_labels = self
            .emitter
            .labels
            .iter()
            .filter(|(label, position)| {
                (delete_start..delete_start + 3).contains(position)
                    && self.is_removable_synthetic_fallthrough_label(label)
            })
            .map(|(label, _)| label.clone())
            .collect::<Vec<_>>();
        for label in removable_labels {
            self.emitter.labels.remove(&label);
        }
        self.delete_emitted_bytes(rewrite.branch_start + 2, 3);
        self.optimizations.push(CodegenOptimization {
            kind: CodegenOptimizationKind::BranchInverted,
            profile: self.profile,
            routine: self.current_routine_name(),
            source_span: Some(rewrite.span),
            address: Some(
                self.emitter
                    .origin
                    .wrapping_add(rewrite.branch_start as u16),
            ),
            bytes_saved: 3,
            message: format!(
                "inverted branch over fallthrough label {} and removed long jump to {}",
                rewrite.fallthrough_label, rewrite.target_label
            ),
        });
        true
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

    fn resolved_machine_branch_crosses_delete(&self, start: usize, len: usize) -> bool {
        let origin = self.emitter.origin;
        for range in self
            .source_ranges
            .iter()
            .filter(|range| range.kind == CodegenSourceRangeKind::MachineBlock)
        {
            let block_start = range.start.wrapping_sub(origin) as usize;
            let block_end = range.end.wrapping_sub(origin) as usize;
            if block_start >= block_end || block_end > self.emitter.bytes.len() {
                continue;
            }
            if start < block_end && block_start < start + len {
                return true;
            }

            let mut offset = block_start;
            while offset < block_end {
                let Some(instruction) = decode_instruction(self.emitter.bytes[offset]) else {
                    break;
                };
                if offset + instruction.len > block_end {
                    break;
                }
                if instruction.mode == AddressingMode::Relative {
                    let operand = self.emitter.bytes[offset + 1] as i8 as isize;
                    let target = offset as isize + 2 + operand;
                    if target >= 0 {
                        let target = target as usize;
                        let delete_end = start + len;
                        if (start..delete_end).contains(&target)
                            || (offset < start && target >= delete_end)
                            || (target < start && offset >= delete_end)
                        {
                            return true;
                        }
                    }
                }
                offset += instruction.len;
            }
        }
        false
    }

    pub(super) fn adjust_recorded_addresses_after_delete(&mut self, start: usize, len: usize) {
        let start_position = start;
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
        for proof in &mut self.proofs {
            if let Some(address) = &mut proof.address {
                adjust_address_after_delete(address, start, len);
            }
        }
        for attempt in &mut self.proof_attempts {
            if let Some(address) = &mut attempt.address {
                adjust_address_after_delete(address, start, len);
            }
        }
        for machine_block in &mut self.machine_blocks {
            adjust_address_after_delete(&mut machine_block.address, start, len);
        }
        if let Some(position) = &mut self.last_label_position {
            adjust_position_after_delete(position, start_position, len as usize);
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
    pub(super) fn routine_entry_plan(&self, routine: &Routine) -> RoutineEntryPlan {
        if !self.profile.enables_modern_optimizations() {
            return RoutineEntryPlan::trampoline(RoutineTrampolineReason::CompatibleProfile);
        }
        let Some(proof) = self.routine_boundary_proof(&routine.name) else {
            return RoutineEntryPlan::trampoline(RoutineTrampolineReason::UnprovenBoundary);
        };
        // Public and address-observable routines may bind their stable entry label
        // directly to the prologue. Only compatible routine-name assignment needs
        // a writable JMP operand at that label.
        if proof.patchable_entry_required {
            return RoutineEntryPlan::trampoline(RoutineTrampolineReason::RetargetableRoutine);
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
