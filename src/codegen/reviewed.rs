//! Project the identity of an unwrapped classic assembly routine before callers
//! are emitted. Only ordinary, statically sized global backing is supported.
use super::*;
use crate::callee6502::{BodyItem, OpaqueTarget, ENTRY_VERSION, ReviewDependencies, fingerprint};
use crate::callee6502::sys_graphics::GraphicsRoutine;
use crate::foreign::{ForeignCodeMode, ForeignRelocationEncoding};
use crate::semantic::ir::*;
use crate::target::ByteSize;

#[derive(Clone, Default)]
pub(super) struct ClassicCalleeReviews {
    pub(super) entries: HashMap<String, u64>,
    pub(super) contracts: HashMap<String, crate::callee6502::CallFacts<crate::semantic::SymbolId>>,
    pub(super) graphics: HashMap<String, (GraphicsRoutine, Option<u16>)>,
}

impl ClassicCalleeReviews {
    pub(super) fn collect(
        program: &SemProgram,
        names: &std::collections::BTreeMap<crate::semantic::SymbolId, String>,
    ) -> Self {
        let mut facts = Self::default();
        for item in program.modules.iter().flat_map(|module| &module.items) {
            let SemItem::Routine(routine) = item else {
                continue;
            };
            let name = names
                .get(&routine.symbol.id)
                .unwrap_or(&routine.symbol.name);
            if routine.callee_review.is_some() {
                if classic_matches(program, routine, &mut ReviewDependencies::new()).is_some() {
                    let selected = program.review_for_selected_entry(routine).expect("declared review");
                    facts.contracts.insert(normalize_name(name), selected.facts);
                }
                continue;
            }
            let Some(expected) =
                routine
                    .annotations
                    .iter()
                    .find_map(|annotation| match annotation {
                        ActioncAnnotation::AbiInputsOnly { fingerprint } => Some(*fingerprint),
                        _ => None,
                    })
            else {
                continue;
            };
            if classic_fingerprint(program, routine, ENTRY_VERSION) == Some(expected) {
                facts.entries.insert(normalize_name(name), expected);
            }
        }
        facts
    }
}

/// Classic tracks memory contents on normal return. An unknown footprint
/// starts with every byte writable so partial preservation narrows only the
/// locations explicitly covered.
pub(super) fn classic_call_effects<Id>(facts: &crate::callee6502::CallFacts<Id>) -> RoutineEffects {
    let mut effects = if let Some(writes) = &facts.writes
        && !facts.buffers.iter().any(|b| b.write)
    {
        let mut effects = RoutineEffects::known_empty();
        if let Some(range) = facts.indirect_writes {
            if let Some(size) = (range.end - range.start).checked_add(1) {
                effects.record_absolute_write(range.start, size);
            } else {
                effects = RoutineEffects::all_writable();
            }
        }
        for range in writes {
            if range.global.is_some() || (range.start == 0 && range.end == u16::MAX) {
                // At this point a symbolic range may still alias absolute storage.
                effects = RoutineEffects::all_writable();
                break;
            }
            effects.record_absolute_write(range.start, range.end - range.start + 1);
        }
        effects
    } else {
        RoutineEffects::all_writable()
    };
    for range in &facts.preserves_zp {
        for addr in range.start..=range.end {
            effects.clear_zero_page_write(ZeroPage::new(addr));
        }
    }
    effects.preserves_a = facts.preserves_regs.a;
    effects.preserves_x = facts.preserves_regs.x;
    effects.preserves_y = facts.preserves_regs.y;
    effects
}

fn classic_matches<'a>(program: &SemProgram, routine: &'a SemRoutine,
    dependencies: &mut ReviewDependencies<crate::semantic::SymbolId>,
) -> Option<&'a crate::callee6502::Review<crate::semantic::SymbolId>> {
    let review = routine.callee_review.as_ref()?;
    let [SemStmt::InlineAsm { program: asm, .. }] = routine.body.as_slice() else { return None };
    (review.bind_precondition(classic_body(program, routine, review.version(), dependencies)?) == review.fingerprint
        && review.dependencies_match(|id| {
            if !asm.relocations.iter().any(|rel| matches!(&rel.target, SemInlineAsmTarget::Symbol(s) if s.id == *id)) { return None; }
            u16::try_from(program.layout.array_for_symbol(*id)?.storage_size?).ok()
        })).then_some(review)
}

fn classic_fingerprint(program: &SemProgram, routine: &SemRoutine, version: &[u8]) -> Option<u64> {
    classic_body(program, routine, version, &mut ReviewDependencies::new())
}

fn classic_body(program: &SemProgram, routine: &SemRoutine, version: &[u8],
    dependencies: &mut ReviewDependencies<crate::semantic::SymbolId>) -> Option<u64> {
    let [SemStmt::InlineAsm { program: asm, .. }] = routine.body.as_slice() else {
        return None;
    };
    let opaque = asm.mode == ForeignCodeMode::Opaque;
    // A normal parameterless entry with only uninitialized inline arrays has
    // static storage before its entry label and no executable setup.
    let private_arrays = opaque && routine.system_address.is_none()
        && routine.signature.params.is_empty() && !routine.locals.is_empty()
        && routine.locals.iter().all(|d| matches!(d.storage, SemDeclarationStorage::Array {
            origin: SemArrayOrigin::Local, fixed_address: None, .. })
            && d.initializer.is_none() && !d.symbol.is_volatile
            && program.layout.array_for_symbol(d.symbol.id).is_some_and(|a| a.storage_size.is_some()));
    let unwrapped = routine.system_address.as_ref().is_some_and(|address|
        matches!(address.kind, SemExprKind::CurrentLocation)) && routine.locals.is_empty();
    if routine.callable_type.variadic.is_some() || !(unwrapped || private_arrays) {
        return None;
    }
    let lane = |ty: &crate::semantic::ValueType| {
        if ty.is_pointer() {
            Some(b'P')
        } else {
            let width = ty.value_width_bytes_for_layout(program.target_layout)?;
            matches!(width, 1 | 2 | 4).then_some(width as u8)
        }
    };
    let params = routine
        .signature
        .params
        .iter()
        .map(lane)
        .collect::<Option<Vec<_>>>()?;
    let result = match &routine.signature.return_type {
        Some(ty) => lane(ty)?,
        None => 0,
    };
    let declarations = || program.modules.iter().flat_map(|m| &m.items);
    let find_routine = |id| declarations().find_map(|i| match i {
        SemItem::Routine(r) if r.symbol.id == id => Some(r), _ => None,
    });
    let mut call_identity = |callee: &SemRoutine| {
        if declarations().any(|i| matches!(i, SemItem::Routine(r)
            if r.symbol.id != callee.symbol.id && r.symbol.name.eq_ignore_ascii_case(&callee.symbol.name))) {
            return None;
        }
        dependencies.resolve(callee.symbol.id, |deps| {
            classic_matches(program, callee, deps)?;
            program.review_for_selected_entry(callee)?.dependency_hash(|id| declarations().find_map(|i| match i {
                SemItem::Declaration(d) if d.symbol.id == *id => Some(d.symbol.name.clone()), _ => None,
            }))
        })
    };
    let mut items = Vec::new();
    let mut cursor = 0usize;
    for rel in &asm.relocations {
        let start = usize::from(rel.offset);
        items.extend(
            asm.bytes
                .get(cursor..start)?
                .iter()
                .copied()
                .map(BodyItem::Byte),
        );
        let width = usize::try_from(rel.encoding.width().get()).ok()?;
        cursor = start.checked_add(width)?;
        if opaque {
            let target = match &rel.target {
                SemInlineAsmTarget::InlineOffset(offset) => OpaqueTarget::Local(*offset),
                SemInlineAsmTarget::Absolute(address) => OpaqueTarget::Absolute(*address),
                SemInlineAsmTarget::Symbol(symbol) => {
                    if let Some(callee) = find_routine(symbol.id) {
                        OpaqueTarget::Routine { name: &callee.symbol.name, contract: call_identity(callee)? }
                    } else if let Some(local) = routine.locals.iter().find(|d| d.symbol.id == symbol.id) {
                        let extent = program.layout.array_for_symbol(symbol.id)?.storage_size?;
                        OpaqueTarget::LocalArray { name: &local.symbol.name, extent: extent.try_into().ok()? }
                    } else {
                        let mut named = declarations().filter_map(|i| match i {
                            SemItem::Declaration(d) if d.symbol.name.eq_ignore_ascii_case(&symbol.name) => Some(d),
                            _ => None,
                        });
                        let decl = named.next()?;
                        if named.next().is_some() || decl.symbol.id != symbol.id || symbol.is_volatile
                            || decl.initializer.is_some() { return None; }
                        let (extent, array) = match decl.storage {
                            SemDeclarationStorage::Array { origin: SemArrayOrigin::Global, fixed_address: None, .. } =>
                                (program.layout.array_for_symbol(symbol.id)?.storage_size?, true),
                            SemDeclarationStorage::Scalar => (decl.ty.width?, false),
                            _ => return None,
                        };
                        OpaqueTarget::Storage { name: &decl.symbol.name, extent: u16::try_from(extent).ok()?, array }
                    }
                }
            };
            if !matches!(rel.required_address_bits, None | Some(8)) { return None; }
            items.push(BodyItem::OpaqueOperand {
                target, kind: crate::asm6502::InlineAsmRelocationKind::from_foreign(rel.encoding)?,
                addend: rel.addend, requires_zp: rel.required_address_bits.is_some(), usage: rel.symbol_use,
            });
            continue;
        }
        match (&rel.target, rel.encoding, rel.required_address_bits) {
            (SemInlineAsmTarget::Absolute(value), ForeignRelocationEncoding::Unsigned { width: ByteSize::ONE }, None)
                if rel.addend == 0 => items.push(BodyItem::Byte(u8::try_from(*value).ok()?)),
            (
                SemInlineAsmTarget::Absolute(addr),
                ForeignRelocationEncoding::Address { width: w },
                None,
            ) if w.get() == 2 && rel.addend == 0 => {
                items.extend(addr.to_le_bytes().map(BodyItem::Byte));
            }
            (
                SemInlineAsmTarget::Absolute(addr),
                ForeignRelocationEncoding::TargetByte { byte_index: 0, .. }
                | ForeignRelocationEncoding::Unsigned {
                    width: ByteSize::ONE,
                },
                Some(8),
            ) if rel.addend == 0 => {
                items.push(BodyItem::Byte(u8::try_from(*addr).ok()?));
            }
            (
                SemInlineAsmTarget::Symbol(symbol),
                ForeignRelocationEncoding::Address { width: w },
                None,
            ) if w.get() == 2 => {
                if let Some(callee) = find_routine(symbol.id) {
                    if rel.addend != 0 { return None; }
                    items.push(BodyItem::Call { name: &callee.symbol.name, contract: call_identity(callee)? });
                    continue;
                }
                let decl =
                    program
                        .modules
                        .iter()
                        .flat_map(|m| &m.items)
                        .find_map(|item| match item {
                            SemItem::Declaration(decl) if decl.symbol.id == symbol.id => Some(decl),
                            _ => None,
                        })?;
                if !matches!(
                    decl.storage,
                    SemDeclarationStorage::Array {
                        fixed_address: None,
                        origin: SemArrayOrigin::Global,
                        ..
                    }
                ) || symbol.is_volatile
                    || decl
                        .initializer
                        .as_ref()
                        .is_some_and(|init| !matches!(init.kind, SemExprKind::InitializerList(_)))
                {
                    return None;
                }
                if program.modules.iter().flat_map(|m| &m.items).any(|i| {
                    matches!(i, SemItem::Declaration(d) if d.symbol.id != symbol.id &&
                        d.symbol.name.eq_ignore_ascii_case(&symbol.name))
                }) {
                    return None;
                }
                let extent =
                    u16::try_from(program.layout.array_for_symbol(symbol.id)?.storage_size?)
                        .ok()?;
                items.push(BodyItem::Global {
                    name: &decl.symbol.name,
                    extent,
                    addend: rel.addend,
                });
            }
            _ => return None,
        }
    }
    items.extend(asm.bytes.get(cursor..)?.iter().copied().map(BodyItem::Byte));
    if opaque { crate::callee6502::opaque_fingerprint(&params, result, &items, version) }
    else { fingerprint(&params, result, &items, version) }
}

#[cfg(test)]
mod tests {

    #[test]
    fn reviewed_indirect_bounds_preserve_other_zp_bytes_and_cover_the_last_address() {
        let mut facts: crate::callee6502::CallFacts<()> = Default::default();
        facts.writes = Some(vec![crate::callee6502::DataRange { global: None, start: 0xa0, end: 0xa2 }]);
        facts.indirect_writes = Some(crate::ast::AnnotationAddressRange { start: 0x100, end: 0xffff });
        let effects = super::classic_call_effects(&facts);
        assert!(effects.writes_zero_page(super::ZeroPage::new(0xa2)));
        assert!(!effects.writes_zero_page(super::ZeroPage::new(0xac)));
        assert!(effects.writes_absolute_range(0xffff, 1));
    }

    #[test]
    fn reviewed_abi_inputs_match_classic_and_mir() {
        let source = "BYTE ARRAY asciiMap(4)=[1 2 3 4]\n;@actionc abi-inputs-only 8ecfa3be2a747397\nBYTE FUNC Ascii=*(CHAR ch)\nASM\ntay\nrol a\nrol a\nrol a\nrol a\nand #$03\ntax\ntya\nand #$9F\nora asciiMap,x\nsta $A0\nrts\nENDASM\nPROC Main() RETURN";
        for (src, valid) in [
            (source.to_owned(), true),
            (source.replace("#$9F", "#$8F"), false),
            (source.replace("Map(4)", "Map(5)"), false),
            (source.replace("CHAR ch", "CARD ch"), false),
        ] {
            let ast = crate::parser::parse(&crate::lexer::tokenize(&src).unwrap()).unwrap();
            let model = crate::semantic::analyze_with_options(
                &ast,
                crate::semantic::SemanticOptions::modern(),
            )
            .unwrap();
            let sem = crate::semantic::ir::lower_program(&ast, &model);
            assert_eq!(
                super::super::semir::semir_to_projection(&sem)
                    .unwrap()
                    .callee_reviews
                    .entries
                    .contains_key("ASCII"),
                valid
            );
        }
    }
}
