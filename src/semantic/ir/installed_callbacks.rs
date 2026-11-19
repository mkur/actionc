//! Discharge a reviewed callback condition for a particular linked program entry.
//! An arbitrary library invocation still has only its unconditional contract.
use super::*;
use crate::ast::AnnotationZeroPageRange;

/// A conditional review belongs to the linked source program. NIR and classic
/// receive only ordinary return facts after selection of the program entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemCallbackReview {
    pub vector: SymbolId,
    /// Present on the installing entry. A wrapper forwards the listed bytes.
    pub installed_prefix: Option<u64>,
    pub bytes: Vec<AnnotationZeroPageRange>,
}

impl SemProgram {
    pub(crate) fn installed_callback(
        &self,
        vector: SymbolId,
    ) -> Option<(&SemRoutine, &SemCallbackReview)> {
        let items = || self.modules.iter().flat_map(|m| &m.items);
        let variable = items().find_map(|i| match i {
            SemItem::Declaration(d) if d.symbol.id == vector => Some(d),
            _ => None,
        })?;
        if !matches!(variable.storage, SemDeclarationStorage::Scalar)
            || variable.symbol.is_volatile
            || variable.initializer.is_some()
            || variable.ty.value.as_callable_pointer().is_none()
        {
            return None;
        }
        let entry = self.program_entry_routine()?;
        let mut reviews = entry
            .callback_reviews
            .iter()
            .filter(|r| r.vector == vector && r.installed_prefix.is_some());
        let installation_review = reviews.next()?;
        if reviews.next().is_some() {
            return None;
        }
        let (installation, rest) = entry.body.split_first()?;
        let SemStmt::Assign { target, value, .. } = installation else {
            return None;
        };
        if !matches!(&target.kind, SemLValueKind::Symbol(v) if v.id == vector) {
            return None;
        }
        let SemExprKind::AddressOfSymbol(handler) = &value.kind else {
            return None;
        };

        // Any other use (including exporting the cell address, a relocation in
        // DATA/SET, or an assembler store) could replace the installed callback.
        // Permit only plain indirect fetches from the vector in opaque dispatchers.
        let watched = HashSet::from([vector]);
        let mut found = HashSet::new();
        for item in items() {
            if let SemItem::Routine(r) = item {
                if let Some(address) = &r.system_address {
                    collect_external_expr_references(address, &watched, &mut found);
                }
                for declaration in &r.locals {
                    collect_external_declaration_references(declaration, &watched, &mut found);
                }
                let body = if r.symbol.id == entry.symbol.id {
                    rest
                } else {
                    &r.body
                };
                for stmt in body {
                    if let SemStmt::InlineAsm { program, .. } = stmt
                        && program.mode == ForeignCodeMode::Opaque
                        && program.relocations.iter().all(|rel| {
                            !matches!(&rel.target, SemInlineAsmTarget::Symbol(s) if s.id == vector)
                                || crate::callee6502::installed_callbacks::indirect_vector(
                                    rel, vector,
                                )
                        })
                    {
                        continue;
                    }
                    collect_external_stmt_references(
                        std::slice::from_ref(stmt),
                        &watched,
                        &mut found,
                    );
                }
            } else {
                if matches!(item, SemItem::Statement(_) | SemItem::Unsupported { .. }) {
                    return None;
                }
                collect_external_item_references(item, &watched, &mut found);
            }
            if found.contains(&vector) {
                return None;
            }
        }
        Some((self.routine_by_id(handler.id)?, installation_review))
    }
}
