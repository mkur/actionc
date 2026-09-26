//! Adjacent sole-use top-bit AND / zero comparison / branch selection.
use super::*;

pub(super) struct TopBitCondition {
    pub source: WordOperand,
    pub bytes: u8,
    pub destination: u8,
    pub negative: bool,
}
fn constant(v: &Mir65816Value) -> Option<u32> {
    match v {
        Mir65816Value::U8(v) => Some((*v).into()),
        Mir65816Value::U16(v) => Some((*v).into()),
        Mir65816Value::U24(v) | Mir65816Value::U32(v) => Some(*v),
        _ => None,
    }
}
pub(super) fn plan(
    b: &Builder<'_>,
    block: &Mir65816Block,
    counts: &BTreeMap<TempId, usize>,
) -> Result<Option<Condition>, String> {
    let [
        ..,
        Mir65816Op::Binary {
            dest: masked,
            width,
            operation: NirBinaryOp::And,
            left,
            right,
            ..
        },
        Mir65816Op::Compare {
            dest,
            width: compare_width,
            operation,
            signed: _,
            left: a,
            right: z,
        },
    ] = block.ops.as_slice()
    else {
        return Ok(None);
    };
    let bytes = width.get() as u8;
    if !matches!(bytes, 1 | 2)
        || *width != *compare_width
        || counts.get(masked) != Some(&1)
        || counts.get(dest) != Some(&1)
        || !matches!(operation, NirCompareOp::Eq | NirCompareOp::Ne)
    {
        return Ok(None);
    }
    if !matches!(&block.terminator,Mir65816Terminator::Branch{condition:Mir65816Value::Temp(id,w),..} if id==dest && *w==ByteSize::ONE)
    {
        return Ok(None);
    }
    let is_masked =
        |v: &Mir65816Value| matches!(v,Mir65816Value::Temp(t,w) if t==masked && *w==*width);
    if !((is_masked(a) && constant(z) == Some(0)) || (is_masked(z) && constant(a) == Some(0))) {
        return Ok(None);
    }
    let mask = 1u32 << (8 * bytes - 1);
    let value = if constant(left) == Some(mask) {
        right
    } else if constant(right) == Some(mask) {
        left
    } else {
        return Ok(None);
    };
    let Mir65816Value::Temp(_, w) = value else {
        return Ok(None);
    };
    if *w != *width {
        return Ok(None);
    }
    // The AND and compare definitions still participate in complete validation.
    let result = b.temp(*masked)?;
    if result.slot().width != bytes {
        return Err("top-bit mask result width mismatch".into());
    }
    b.check_transfer(result.into(), result.into(), bytes)?;
    let Some(condition) = b.condition(*dest, bytes, false, *operation, a, z)? else {
        return Ok(None);
    };
    let Some(Memory::Stack(offset)) = b.value_memory(value)? else {
        return Ok(None);
    };
    b.displacement(offset, u32::from(bytes) - 1)?;
    let source = WordOperand::Stack(b.displacement(offset, 0)?);
    Ok(Some(Condition::TopBit(TopBitCondition {
        source,
        bytes,
        destination: condition.destination(),
        negative: *operation == NirCompareOp::Ne,
    })))
}
impl Builder<'_> {
    pub(super) fn branch_on_top_bit(&mut self, c: &TopBitCondition, yes: Label, dispatch: bool) {
        self.code.barrier();
        if c.bytes == 1 {
            self.code.a8();
        } else {
            self.code.a16();
        }
        self.edge_load(c.source);
        self.code.a16(); // REP leaves the tested N flag intact, including A8.
        let predicate = if c.negative {
            Branch::Minus
        } else {
            Branch::Plus
        };
        if dispatch {
            self.code.dispatch(predicate, yes);
        } else {
            self.code.branch(predicate, yes);
        }
    }
}

#[cfg(test)]
#[path = "top_bit_tests.rs"]
mod tests;
