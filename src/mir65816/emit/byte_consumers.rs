//! Adjacent sole-use BYTE loads consumed in A by Eq/Ne. No storage borrowing.
use super::*;

pub(super) fn plan(
    b: &Builder<'_>,
    block: &Mir65816Block,
    counts: &BTreeMap<TempId, usize>,
) -> Result<BTreeMap<usize, Condition>, String> {
    let mut out = BTreeMap::new();
    for (i, ops) in block.ops.windows(2).enumerate() {
        let Mir65816Op::Load {
            dest: loaded,
            width,
            volatile: false,
            ..
        } = &ops[0]
        else {
            continue;
        };
        if *width != ByteSize::ONE || counts.get(loaded) != Some(&1) {
            continue;
        }
        let Location::Stack(home) = b.temp(*loaded)? else {
            continue;
        };
        if home.width != 1 {
            return Err("BYTE consumer has incomplete load home".into());
        }
        b.displacement(home.offset.into(), 0)?;
        let Mir65816Op::Compare {
            dest,
            width,
            signed,
            operation,
            left,
            right,
        } = &ops[1]
        else {
            continue;
        };
        if *width != ByteSize::ONE || !matches!(operation, NirCompareOp::Eq | NirCompareOp::Ne) {
            continue;
        }
        let is_loaded = |v: &Mir65816Value| matches!(v, Mir65816Value::Temp(id,w) if id==loaded && *w==ByteSize::ONE);
        let (left, right) = if is_loaded(left) {
            (left, right)
        } else if is_loaded(right) {
            (right, left)
        } else {
            continue;
        };
        let Some(Condition::Byte(mut condition)) =
            b.condition(*dest, 1, *signed, *operation, left, right)?
        else {
            continue;
        };
        // Preflight all operands before omitting either private transfer. The
        // exact load remains at its source site and no operation intervenes.
        condition.left_in_a = true;
        out.insert(i + 1, Condition::Byte(condition));
    }
    Ok(out)
}
