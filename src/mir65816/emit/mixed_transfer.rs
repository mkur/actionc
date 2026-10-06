//! Canonical typed emission and replay of a complete mixed edge schedule.
use super::super::mixed_copies::{Plan, Source, Step};
use super::*;

impl TrackedEmitter65816 {
    pub(in crate::mir65816::emit) fn mixed_edge(&mut self, plan: Plan, staging: Vec<Slot>) {
        self.request(Request::MixedEdge(plan.clone(), staging.clone()), |this| {
            assert_eq!(this.state.delta(), 0);
            this.a8();
            let load = |this: &mut Self, source: &Source, byte: u8| match source {
                Source::Home(h) => this.byte(
                    match h {
                        Location::Stack(_) => ByteOp::LdaStack,
                        Location::DirectPage(_) => ByteOp::LdaDp,
                    },
                    h.slot().offset as u8 + byte,
                ),
                Source::Staged(i) => this.byte(ByteOp::LdaStack, staging[*i].offset as u8 + byte),
                Source::Immediate(n) => {
                    this.byte(ByteOp::LdaImm, (n >> (8 * u32::from(byte))) as u8)
                }
            };
            let store = |this: &mut Self, h: Location, byte: u8| {
                this.byte(
                    match h {
                        Location::Stack(_) => ByteOp::StaStack,
                        Location::DirectPage(_) => ByteOp::StaDp,
                    },
                    h.slot().offset as u8 + byte,
                )
            };
            for step in &plan.steps {
                match step {
                    Step::Capture { source, pool } => {
                        for byte in 0..source.slot().width {
                            load(this, &Source::Home(*source), byte);
                            store(this, Location::Stack(staging[*pool]), byte);
                        }
                    }
                    Step::Move(m) => {
                        for byte in 0..m.bytes {
                            load(this, &m.source, byte);
                            store(this, m.destination, byte);
                        }
                    }
                }
            }
            // Preserve incoming hidden B and reproduce the old edge's final
            // byte/NZ even after reordering, an identity, or a cycle capture.
            if let Some(last) = plan.moves.last() {
                load(this, &Source::Home(last.destination), last.bytes - 1);
            }
        });
    }
}
