//! Immediately returned native call results stay in their declared ABI lanes.
use super::*;

#[cfg(test)]
#[path = "call_return_tests.rs"]
mod tests;

impl Builder<'_> {
    /// Called only for the final operation, before the adjacent terminator.
    /// Preflight still validates the reserved result home, but no store or home
    /// definition is published. The normal source spans and call effects remain.
    #[cfg(test)]
    pub(super) fn call_return(
        &mut self,
        op: &Mir65816Op,
        terminator: &Mir65816Terminator,
        input_counts: &BTreeMap<TempId, usize>,
    ) -> Result<bool, String> {
        let Mir65816Op::Call {
            target,
            args,
            result: Some((dest, bytes)),
            plan,
            ..
        } = op
        else {
            return Ok(false);
        };
        if !call_flow::returns(
            self.routine,
            op,
            terminator,
            input_counts.get(dest).copied().unwrap_or(0),
        ) {
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
