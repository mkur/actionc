//! Immediately returned native call results stay in their declared ABI lanes.
use super::*;

#[cfg(test)]
#[path = "call_return_tests.rs"]
mod tests;

impl Builder<'_> {
    /// Called only for the final operation, before the adjacent terminator.
    /// Preflight still validates the reserved result home, but no store or home
    /// definition is published. The normal source spans and call effects remain.
    pub(super) fn call_return(
        &mut self,
        op: &Mir65816Op,
        terminator: &Mir65816Terminator,
        input_counts: &BTreeMap<TempId, usize>,
    ) -> Result<bool, String> {
        let (
            Mir65816Op::Call {
                target,
                args,
                result: Some((dest, bytes)),
                plan,
                ..
            },
            Mir65816Terminator::Return {
                value: Some(Mir65816Value::Temp(value, returned_bytes)),
                ..
            },
        ) = (op, terminator)
        else {
            return Ok(false);
        };
        if dest != value
            || bytes != returned_bytes
            || input_counts.get(dest) != Some(&1)
            || !matches!(
                target,
                Mir65816CallTarget::Direct(_)
                    | Mir65816CallTarget::Helper(_)
                    | Mir65816CallTarget::Runtime(_)
            )
            || plan.result != self.routine.result_home
            || !matches!(
                (bytes.get(), plan.result),
                (
                    1,
                    Some(Mir65816AbiHome::NativeResult(
                        abi::ResultLocation::A8ZeroExtended
                    ))
                ) | (
                    2,
                    Some(Mir65816AbiHome::NativeResult(abi::ResultLocation::A16))
                )
            )
        {
            return Ok(false);
        }
        // The ordinary call preflight checks the complete native contract,
        // arguments, reserved result home and S delta before emitting anything.
        self.call_with_result(
            target,
            args,
            Some((*dest, *bytes)),
            plan,
            CallResultUse::Return,
        )?;
        Ok(true)
    }
}
