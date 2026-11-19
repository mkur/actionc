//! Match the selected program's installed callback with its reviewed 6502
//! return path before projecting the wrapper facts into classic or NIR.
use super::{BodyItem, OpaqueTarget, Review};
use crate::asm6502::InlineAsmRelocationKind;
use crate::ast::AnnotationZeroPageRange;
use crate::foreign::{ForeignCodeMode, ForeignSymbolUse};
use crate::semantic::{SymbolId, ir::*};
use std::collections::HashSet;

impl SemProgram {
    pub(crate) fn review_for_selected_entry(
        &self,
        routine: &SemRoutine,
    ) -> Option<Review<SymbolId>> {
        let mut review = routine.callee_review.clone()?;
        let Some(forwarded) = routine
            .callback_reviews
            .iter()
            .find(|r| r.installed_prefix.is_none())
        else {
            return Some(review);
        };
        if self.target_layout.target == crate::target::TargetId::Atari6502
            && let Some((handler, installation)) = self.installed_callback(forwarded.vector)
            && return_prefix_fingerprint(self, handler, &installation.bytes)
                == installation.installed_prefix
            && dispatches_through(self, routine, forwarded.vector, &mut HashSet::new())
        {
            for a in &forwarded.bytes {
                for b in &installation.bytes {
                    let (start, end) = (a.start.max(b.start), a.end.min(b.end));
                    if start <= end {
                        review
                            .facts
                            .preserves_zp
                            .push(AnnotationZeroPageRange { start, end });
                    }
                }
            }
        }
        Some(review)
    }
}

pub(crate) fn indirect_vector(rel: &SemInlineAsmRelocation, vector: SymbolId) -> bool {
    matches!(&rel.target, SemInlineAsmTarget::Symbol(s) if s.id == vector)
        && rel.symbol_use == ForeignSymbolUse::PointerRead
        && rel.addend == 0
        && InlineAsmRelocationKind::from_foreign(rel.encoding)
            == Some(InlineAsmRelocationKind::Absolute16)
}

// The root review binds all typed direct dependencies. The named callback
// must actually occur as a dispatch operand within that reviewed closure.
fn dispatches_through(
    program: &SemProgram,
    routine: &SemRoutine,
    vector: SymbolId,
    seen: &mut HashSet<SymbolId>,
) -> bool {
    if routine.callee_review.is_none() || !seen.insert(routine.symbol.id) {
        return false;
    }
    let [SemStmt::InlineAsm { program: asm, .. }] = routine.body.as_slice() else {
        return false;
    };
    if asm.mode != ForeignCodeMode::Opaque {
        return false;
    }
    for rel in &asm.relocations {
        if indirect_vector(rel, vector) {
            return true;
        }
        if matches!(
            rel.symbol_use,
            ForeignSymbolUse::Call | ForeignSymbolUse::Control
        ) && let SemInlineAsmTarget::Symbol(s) = &rel.target
            && let Some(child) = program.routine_by_id(s.id)
            && dispatches_through(program, child, vector, seen)
        {
            return true;
        }
    }
    false
}

/// A manually reviewed leading opaque block covers every ordinary return to
/// the caller. Its fallthrough has abandoned that invocation. Bind the exact
/// bytes, relocations, ABI and claimed lanes. Recovery calls in the block
/// execute only on the abandoning path and supply no preservation facts.
fn return_prefix_fingerprint(
    program: &SemProgram,
    handler: &SemRoutine,
    ranges: &[AnnotationZeroPageRange],
) -> Option<u64> {
    if !handler
        .system_address
        .as_ref()
        .is_some_and(|a| matches!(a.kind, SemExprKind::CurrentLocation))
        || handler.is_external
        || !handler.locals.is_empty()
        || !handler.params.is_empty()
        || handler.signature.return_type.is_some()
        || handler.callable_type.variadic.is_some()
    {
        return None;
    }
    let SemStmt::InlineAsm { program: asm, .. } = handler.body.first()? else {
        return None;
    };
    if asm.mode != ForeignCodeMode::Opaque {
        return None;
    }
    let mut lexical_locals = false;
    visit_lexical_declarations(&handler.body, &mut |_, _| lexical_locals = true);
    if lexical_locals {
        return None;
    }
    let mut cursor = 0;
    let mut items = Vec::new();
    for rel in &asm.relocations {
        if !matches!(rel.required_address_bits, None | Some(8)) {
            return None;
        }
        let offset = usize::from(rel.offset);
        items.extend(
            asm.bytes
                .get(cursor..offset)?
                .iter()
                .copied()
                .map(BodyItem::Byte),
        );
        cursor = offset.checked_add(usize::try_from(rel.encoding.width().get()).ok()?)?;
        let kind = InlineAsmRelocationKind::from_foreign(rel.encoding)?;
        let target = match &rel.target {
            SemInlineAsmTarget::InlineOffset(n) => OpaqueTarget::Local(*n),
            SemInlineAsmTarget::Absolute(n) => OpaqueTarget::Absolute(*n),
            SemInlineAsmTarget::Symbol(s) => {
                let item = program
                    .modules
                    .iter()
                    .flat_map(|m| &m.items)
                    .find(|i| match i {
                        SemItem::Routine(r) => r.symbol.id == s.id,
                        SemItem::Declaration(d) => d.symbol.id == s.id,
                        _ => false,
                    })?;
                match item {
                    SemItem::Routine(r)
                        if rel.symbol_use == ForeignSymbolUse::Call
                            && r.params.is_empty()
                            && r.signature.return_type.is_none()
                            && !r.is_external =>
                    {
                        OpaqueTarget::Routine {
                            name: &r.symbol.qualified_name,
                            contract: 0,
                        }
                    }
                    SemItem::Declaration(d)
                        if matches!(d.storage, SemDeclarationStorage::Scalar)
                            && !d.symbol.is_volatile
                            && d.initializer.is_none() =>
                    {
                        OpaqueTarget::Storage {
                            name: &d.symbol.qualified_name,
                            extent: d.ty.width?.try_into().ok()?,
                            array: false,
                        }
                    }
                    _ => return None,
                }
            }
        };
        items.push(BodyItem::OpaqueOperand {
            target,
            kind,
            addend: rel.addend,
            requires_zp: rel.required_address_bits == Some(8),
            usage: rel.symbol_use,
        });
    }
    items.extend(asm.bytes.get(cursor..)?.iter().copied().map(BodyItem::Byte));
    let mut version = b"normal-return-prefix-v1".to_vec();
    for r in ranges {
        version.extend([r.start, r.end]);
    }
    crate::callee6502::opaque_fingerprint(&[], 0, &items, &version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::ActioncAnnotation;

    #[test]
    fn shipped_callback_prefixes_match_their_reviews() {
        let mut actual = Vec::new();
        let mut expected = Vec::new();
        for name in ["TN.ACT", "TNDBG.ACT"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("samples/tn/modern")
                .join(name);
            let ast = crate::includes::load_program_with_includes(path).unwrap();
            let model = crate::semantic::analyze_with_options(
                &ast,
                crate::semantic::SemanticOptions::modern(),
            )
            .unwrap();
            let sem = lower_program(&ast, &model);
            let handler = sem
                .modules
                .iter()
                .flat_map(|m| &m.items)
                .find_map(|i| match i {
                    SemItem::Routine(r) if r.symbol.name == "NavError" => Some(r),
                    _ => None,
                })
                .unwrap();
            let ActioncAnnotation::InstalledCallback {
                fingerprint,
                ranges,
                ..
            } = &sem.program_entry_routine().unwrap().annotations[0]
            else {
                panic!("missing return review")
            };
            actual.push(format!(
                "{:016x}",
                return_prefix_fingerprint(&sem, handler, ranges).unwrap()
            ));
            expected.push(format!("{fingerprint:016x}"));
        }
        assert_eq!(actual, expected);
    }
}
