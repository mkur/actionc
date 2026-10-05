//! One exhaustive logical operand census, shared with physical home allocation.
use crate::mir65816::*;

#[derive(Default)]
pub(crate) struct Census<'a> {
    pub inputs: Vec<&'a Mir65816Value>,
    pub definition: Option<(TempId, ByteSize)>,
}

impl<'a> Census<'a> {
    fn address(&mut self, address: &'a Mir65816Address) {
        if let Mir65816AddressBase::Indirect(value) = &address.base {
            self.inputs.push(value);
        }
        if let Some(index) = &address.index {
            self.inputs.push(&index.value);
        }
    }
}

pub(crate) fn operation(op: &Mir65816Op) -> Census<'_> {
    let mut c = Census::default();
    match op {
        Mir65816Op::Load {
            dest,
            width,
            address,
            ..
        }
        | Mir65816Op::AddressOf {
            dest,
            width,
            address,
        } => {
            c.definition = Some((*dest, *width));
            c.address(address);
        }
        Mir65816Op::Store { address, value, .. } => {
            c.address(address);
            c.inputs.push(value);
        }
        Mir65816Op::Copy {
            destination,
            source,
            ..
        } => {
            c.address(destination);
            c.address(source);
        }
        Mir65816Op::Unary {
            dest, width, value, ..
        } => {
            c.definition = Some((*dest, *width));
            c.inputs.push(value);
        }
        Mir65816Op::Cast {
            dest, to, value, ..
        } => {
            c.definition = Some((*dest, *to));
            c.inputs.push(value);
        }
        Mir65816Op::PointerOffset {
            dest,
            width,
            base,
            offset,
            ..
        } => {
            c.definition = Some((*dest, *width));
            c.inputs.extend([base, offset]);
        }
        Mir65816Op::Binary {
            dest,
            width,
            left,
            right,
            ..
        } => {
            c.definition = Some((*dest, *width));
            c.inputs.extend([left, right]);
        }
        Mir65816Op::Compare {
            dest, left, right, ..
        } => {
            c.definition = Some((*dest, ByteSize::ONE));
            c.inputs.extend([left, right]);
        }
        Mir65816Op::Call {
            target,
            args,
            result,
            ..
        } => {
            match target {
                Mir65816CallTarget::Indirect(value, _) => c.inputs.push(value),
                Mir65816CallTarget::Direct(_)
                | Mir65816CallTarget::Helper(_)
                | Mir65816CallTarget::Builtin(_)
                | Mir65816CallTarget::Runtime(_) => {}
            }
            c.inputs.extend(args);
            c.definition = *result;
        }
    }
    c
}

pub(crate) fn edges(term: &Mir65816Terminator) -> Vec<&Mir65816Edge> {
    match term {
        Mir65816Terminator::Goto(edge) => vec![edge],
        Mir65816Terminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![then_edge, else_edge],
        Mir65816Terminator::Fallthrough
        | Mir65816Terminator::Return { .. }
        | Mir65816Terminator::Exit
        | Mir65816Terminator::ArithmeticFault => vec![],
    }
}

pub(crate) fn terminator(term: &Mir65816Terminator) -> Census<'_> {
    let mut c = Census::default();
    match term {
        Mir65816Terminator::Branch { condition, .. } => c.inputs.push(condition),
        Mir65816Terminator::Return { value, .. } => c.inputs.extend(value),
        Mir65816Terminator::Goto(_)
        | Mir65816Terminator::Fallthrough
        | Mir65816Terminator::Exit
        | Mir65816Terminator::ArithmeticFault => {}
    }
    for edge in edges(term) {
        c.inputs.extend(&edge.args);
    }
    c
}

/// Ordered logical successors. Parallel branch edges remain distinct.
pub(crate) fn successors(routine: &Mir65816Routine, index: usize) -> Result<Vec<BlockId>, String> {
    let block = &routine.blocks[index];
    if matches!(block.terminator, Mir65816Terminator::Fallthrough) {
        let next = routine
            .blocks
            .get(index + 1)
            .ok_or("unresolved terminal fallthrough")?;
        if !next.params.is_empty() {
            return Err("fallthrough cannot supply block parameters".into());
        }
        Ok(vec![next.id])
    } else {
        Ok(edges(&block.terminator)
            .into_iter()
            .map(|e| e.target)
            .collect())
    }
}
