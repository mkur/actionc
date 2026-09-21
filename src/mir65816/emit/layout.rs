//! Routine-local relaxation after typed selection, before either output packer.
use super::{Code, Target};
use std::collections::BTreeSet;

/// Check nonserialized branch claims against actual encodings and, when placed,
/// PBR. Relative operands are complete before image/o65 relocation collection.
pub(crate) fn validate_branches(code: &Code, base: Option<u32>) -> Result<(), String> {
    let mut ranges = vec![];
    for site in &code.conditional_branches {
        let at = site.offset;
        let size = if site.short { 2 } else { 6 };
        let end = at
            .checked_add(size)
            .filter(|&e| e <= code.bytes.len())
            .ok_or("conditional site outside routine")?;
        if !matches!(site.predicate, 0x10 | 0x30 | 0x90 | 0xb0 | 0xd0 | 0xf0) {
            return Err("invalid conditional predicate".into());
        }
        let target = *code
            .labels
            .get(&site.target)
            .ok_or("unresolved conditional label")?;
        let boundary = |p| p == 0 || code.boundaries.contains(&p);
        if target >= code.bytes.len() || !boundary(at) || !boundary(end) || !boundary(target) {
            return Err("conditional target/site is not an emission boundary".into());
        }
        if code.labels.values().any(|&p| at < p && p < end) {
            return Err("label inside conditional encoding".into());
        }
        for f in &code.fixups {
            let hi = f
                .offset
                .checked_add(if f.byte.is_some() { 1 } else { 3 })
                .ok_or("fixup extent overflow")?;
            if f.offset < end
                && at < hi
                && (site.short
                    || f.offset != at + 3
                    || f.target != Target::Label(site.target)
                    || f.addend != 0
                    || f.byte.is_some())
            {
                return Err("unrelated fixup overlaps conditional encoding".into());
            }
        }
        if code
            .return_fixups
            .iter()
            .any(|(p, _)| *p < end && p.checked_add(2).is_none_or(|hi| at < hi))
        {
            return Err("PER overlaps conditional encoding".into());
        }
        if site.short {
            let delta = i8::try_from(target as i64 - (at + 2) as i64)
                .map_err(|_| "short conditional out of range")?;
            if code.bytes[at..end] != [site.predicate, delta as u8] {
                return Err("short conditional encoding mismatch".into());
            }
        } else {
            if code.bytes[at..at + 3] != [site.predicate ^ 0x20, 4, 0x5c]
                || code
                    .fixups
                    .iter()
                    .filter(|f| {
                        f.offset == at + 3
                            && f.target == Target::Label(site.target)
                            && f.addend == 0
                            && f.byte.is_none()
                    })
                    .count()
                    != 1
            {
                return Err("long conditional encoding/fixup mismatch".into());
            }
        }
        if let Some(base) = base {
            let address = |p: usize| {
                u32::try_from(p)
                    .ok()
                    .and_then(|p| base.checked_add(p))
                    .filter(|&p| p < 0x1000000)
                    .ok_or("conditional address overflow")
            };
            let pc = address(at)?;
            let next = address(end)?;
            let dest = address(target)?;
            if pc >> 16 != next >> 16 || pc >> 16 != dest >> 16 {
                return Err("conditional transfer crosses code bank".into());
            }
        }
        ranges.push(at..end);
    }
    ranges.sort_by_key(|r| r.start);
    if ranges.windows(2).any(|r| r[0].end > r[1].start) {
        return Err("overlapping conditional sites".into());
    }
    Ok(())
}

pub(super) fn finalize(mut code: Code, relax: bool) -> Result<Code, String> {
    validate_branches(&code, None)?;
    if !relax {
        return Ok(code);
    }
    if code.conditional_branches.iter().any(|s| s.short) {
        return Err("conditional layout already finalized".into());
    }
    // Selection records sites in physical order. No alignment exists within a
    // routine, so deleting intervening bytes cannot increase branch distance.
    let mut sites = code.conditional_branches.clone();
    sites.sort_by_key(|s| s.offset);
    let mut short = BTreeSet::new();
    loop {
        let before = short.len();
        for s in &sites {
            if short.contains(&s.offset) {
                continue;
            }
            let shift = |p: usize| 4 * short.iter().filter(|&&at| at + 6 <= p).count();
            let target = code.labels[&s.target];
            let own = if target >= s.offset + 6 { 4 } else { 0 };
            let delta =
                (target - shift(target) - own) as i64 - (s.offset - shift(s.offset) + 2) as i64;
            if i8::try_from(delta).is_ok() {
                short.insert(s.offset);
            }
        }
        if short.len() == before {
            break;
        }
    }
    let map = |p: usize| -> Result<usize, String> {
        if p > code.bytes.len() || short.iter().any(|&at| at + 2 <= p && p < at + 6) {
            return Err("metadata inside removed conditional encoding".into());
        }
        Ok(p - 4 * short.iter().filter(|&&at| at + 6 <= p).count())
    };
    let boundary = |p| -> Result<usize, String> {
        if p != 0 && !code.boundaries.contains(&p) {
            return Err("metadata is not an emission boundary".into());
        }
        map(p)
    };
    let mut bytes = Vec::with_capacity(code.bytes.len() - 4 * short.len());
    let mut cursor = 0;
    for site in &mut sites {
        let at = site.offset;
        bytes.extend_from_slice(&code.bytes[cursor..at]);
        if short.contains(&at) {
            let delta =
                i8::try_from(map(code.labels[&site.target])? as i64 - (map(at)? + 2) as i64)
                    .map_err(|_| "unstable short-branch layout")?;
            bytes.extend([site.predicate, delta as u8]);
            site.short = true;
        } else {
            bytes.extend_from_slice(&code.bytes[at..at + 6]);
        }
        site.offset = map(at)?;
        cursor = at + 6;
    }
    bytes.extend_from_slice(&code.bytes[cursor..]);
    for pos in code.labels.values_mut() {
        *pos = boundary(*pos)?;
    }
    let owned: BTreeSet<_> = short.iter().map(|at| at + 3).collect();
    code.fixups.retain(|f| !owned.contains(&f.offset));
    for f in &mut code.fixups {
        f.offset = map(f.offset)?;
    }
    for (at, _) in &mut code.return_fixups {
        *at = map(*at)?;
    }
    for span in code.mir_spans.values_mut() {
        if span.start > span.end {
            return Err("inverted MIR span".into());
        }
        *span = boundary(span.start)?..boundary(span.end)?;
    }
    for transfer in &mut code.mir_transfers {
        transfer.offset = boundary(transfer.offset)?;
    }
    #[cfg(feature = "native65816-state-proof")]
    for snapshot in &mut code.state_trace {
        snapshot.pc = boundary(snapshot.pc)?;
    }
    code.boundaries = code
        .boundaries
        .iter()
        .map(|&p| map(p))
        .collect::<Result<_, _>>()?;
    code.conditional_branches = sites;
    code.bytes = bytes;
    validate_branches(&code, None)?;
    Ok(code)
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
