//! Routine-local relaxation after typed selection, before either output packer.
use super::{Code, JumpEncoding, Target};
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
        if !matches!(
            site.predicate,
            0x10 | 0x30 | 0x50 | 0x70 | 0x90 | 0xb0 | 0xd0 | 0xf0
        ) {
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
    for site in &code.local_jumps {
        let at = site.offset;
        let end = at
            .checked_add(site.encoding.size())
            .filter(|&e| e <= code.bytes.len())
            .ok_or("local jump outside routine")?;
        let target = *code
            .labels
            .get(&site.target)
            .ok_or("unresolved local jump label")?;
        let boundary = |p| p == 0 || code.boundaries.contains(&p);
        if target >= code.bytes.len() || !boundary(at) || !boundary(end) || !boundary(target) {
            return Err("local jump target/site is not an emission boundary".into());
        }
        if code.labels.values().any(|&p| at < p && p < end) {
            return Err("label inside local jump encoding".into());
        }
        let long = site.encoding == JumpEncoding::Long;
        for f in &code.fixups {
            let hi = f
                .offset
                .checked_add(if f.byte.is_some() { 1 } else { 3 })
                .ok_or("fixup extent overflow")?;
            if f.offset < end
                && at < hi
                && (!long
                    || f.offset != at + 1
                    || f.target != Target::Label(site.target)
                    || f.addend != 0
                    || f.byte.is_some())
            {
                return Err("unrelated fixup overlaps local jump".into());
            }
        }
        if code
            .return_fixups
            .iter()
            .any(|(p, _)| *p < end && p.checked_add(2).is_none_or(|hi| at < hi))
        {
            return Err("PER overlaps local jump".into());
        }
        let delta = target as i64 - end as i64;
        let expected = match site.encoding {
            JumpEncoding::Long => {
                if code
                    .fixups
                    .iter()
                    .filter(|f| {
                        f.offset == at + 1
                            && f.target == Target::Label(site.target)
                            && f.addend == 0
                            && f.byte.is_none()
                    })
                    .count()
                    != 1
                {
                    return Err("long local jump fixup mismatch".into());
                }
                vec![0x5c, 0, 0, 0]
            }
            JumpEncoding::Relative8 => vec![
                0x80,
                i8::try_from(delta).map_err(|_| "BRA out of range")? as u8,
            ],
            JumpEncoding::Relative16 => {
                let [lo, hi] = i16::try_from(delta)
                    .map_err(|_| "BRL out of range")?
                    .to_le_bytes();
                vec![0x82, lo, hi]
            }
        };
        if code.bytes[at..end] != expected {
            return Err("local jump encoding mismatch".into());
        }
        if let Some(base) = base {
            let address = |p: usize| {
                u32::try_from(p)
                    .ok()
                    .and_then(|p| base.checked_add(p))
                    .filter(|&p| p < 0x1000000)
                    .ok_or("local jump address overflow")
            };
            let pc = address(at)?;
            if pc >> 16 != address(end)? >> 16 || pc >> 16 != address(target)? >> 16 {
                return Err("local jump crosses code bank".into());
            }
        }
        ranges.push(at..end);
    }
    ranges.sort_by_key(|r| r.start);
    if ranges.windows(2).any(|r| r[0].end > r[1].start) {
        return Err("overlapping local transfer sites".into());
    }
    Ok(())
}

pub(super) fn finalize(mut code: Code, relax: bool) -> Result<Code, String> {
    #[cfg(any(test, feature = "native65816-state-proof"))]
    crate::mir65816::emit::work::add("layout", 1);
    validate_branches(&code, None)?;
    if let Some(selected) = &code.selected {
        selected.reconcile(&code)?;
    }
    if !relax {
        return Ok(code);
    }
    if code.conditional_branches.iter().any(|s| s.short)
        || code
            .local_jumps
            .iter()
            .any(|s| s.encoding != JumpEncoding::Long)
    {
        return Err("local transfer layout already finalized".into());
    }
    #[derive(Clone)]
    struct Site {
        at: usize,
        target: usize,
        original: usize,
        size: usize,
        predicate: Option<u8>,
    }
    let mut sites: Vec<_> = code
        .conditional_branches
        .iter()
        .map(|s| Site {
            at: s.offset,
            target: code.labels[&s.target],
            original: 6,
            size: 6,
            predicate: Some(s.predicate),
        })
        .chain(code.local_jumps.iter().map(|s| Site {
            at: s.offset,
            target: code.labels[&s.target],
            original: 4,
            size: 4,
            predicate: None,
        }))
        .collect();
    sites.sort_by_key(|s| s.at);
    let shift = |sites: &[Site], p: usize| -> usize {
        sites
            .iter()
            .take_while(|s| s.at + s.original <= p)
            .map(|s| s.original - s.size)
            .sum()
    };
    // No internal alignment: each reduction can only bring other targets closer.
    // Include this candidate's additional forward shrink, even after BRL -> BRA.
    loop {
        let mut changed = false;
        for i in 0..sites.len() {
            let s = &sites[i];
            for size in [2, 3] {
                if size >= s.size || size == 3 && s.predicate.is_some() {
                    continue;
                }
                let own = if s.target >= s.at + s.original {
                    s.size - size
                } else {
                    0
                };
                let delta = (s.target - shift(&sites, s.target) - own) as i64
                    - (s.at - shift(&sites, s.at) + size) as i64;
                if (size == 2 && i8::try_from(delta).is_ok())
                    || (size == 3 && i16::try_from(delta).is_ok())
                {
                    sites[i].size = size;
                    changed = true;
                    break;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let map = |p: usize| -> Result<usize, String> {
        if p > code.bytes.len()
            || sites
                .iter()
                .any(|s| s.at + s.size <= p && p < s.at + s.original)
        {
            return Err("metadata inside removed local transfer encoding".into());
        }
        Ok(p - shift(&sites, p))
    };
    let boundary = |p| -> Result<usize, String> {
        if p != 0 && !code.boundaries.contains(&p) {
            return Err("metadata is not an emission boundary".into());
        }
        map(p)
    };
    let mut bytes = Vec::with_capacity(map(code.bytes.len())?);
    let mut cursor = 0;
    for s in &sites {
        bytes.extend_from_slice(&code.bytes[cursor..s.at]);
        if s.size < s.original {
            let delta = map(s.target)? as i64 - (map(s.at)? + s.size) as i64;
            if s.size == 2 {
                bytes.extend([
                    s.predicate.unwrap_or(0x80),
                    i8::try_from(delta).map_err(|_| "unstable short layout")? as u8,
                ]);
            } else {
                bytes.push(0x82);
                bytes.extend(
                    i16::try_from(delta)
                        .map_err(|_| "unstable BRL layout")?
                        .to_le_bytes(),
                );
            }
        } else {
            bytes.extend_from_slice(&code.bytes[s.at..s.at + s.original]);
        }
        cursor = s.at + s.original;
    }
    bytes.extend_from_slice(&code.bytes[cursor..]);
    for s in &mut code.conditional_branches {
        s.short = sites
            .binary_search_by_key(&s.offset, |s| s.at)
            .map(|i| sites[i].size == 2)
            .unwrap();
        s.offset = map(s.offset)?;
    }
    for s in &mut code.local_jumps {
        s.encoding = match sites[sites.binary_search_by_key(&s.offset, |s| s.at).unwrap()].size {
            2 => JumpEncoding::Relative8,
            3 => JumpEncoding::Relative16,
            _ => JumpEncoding::Long,
        };
        s.offset = map(s.offset)?;
    }
    for pos in code.labels.values_mut() {
        *pos = boundary(*pos)?;
    }
    let owned: BTreeSet<_> = sites
        .iter()
        .filter(|s| s.size < s.original)
        .map(|s| s.at + s.original - 3)
        .collect();
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
    #[cfg(feature = "native65816-state-proof")]
    for effect in &mut code.instruction_effects {
        effect.start = boundary(effect.start)?;
        effect.end = boundary(effect.end)?;
    }
    if let Some(selected) = &mut code.selected {
        selected.remap(boundary)?;
    }
    code.boundaries = code
        .boundaries
        .iter()
        .map(|&p| map(p))
        .collect::<Result<_, _>>()?;
    code.bytes = bytes;
    validate_branches(&code, None)?;
    if let Some(selected) = &code.selected {
        selected.reconcile(&code)?;
    }
    Ok(code)
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
