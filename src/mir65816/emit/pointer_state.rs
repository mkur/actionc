//! Reuse only a pointer actually staged from a checked private stack source.
use super::super::effects::{self, Access, Memory};
use super::*;
use crate::mir65816::{Mir65816FrameObjectId, ParamId, abi};
use crate::nir::{ByteOffset, ByteSize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mir65816::emit) enum PointerOrigin {
    Parameter(ParamId),
    Frame(Mir65816FrameObjectId),
    Temporary(TempId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Resident {
    origin: PointerOrigin,
    source: Slot,
    scratch: u8,
    depth: i64,
    generation: u64,
}

impl TrackedEmitter65816 {
    pub(super) fn invalidate_pointer(&mut self) {
        self.pointer = None;
        self.pointer_generation += 1;
    }

    pub fn forget_pointer(&mut self) {
        self.request(Request::ForgetPointer, |this| this.invalidate_pointer());
    }

    pub fn allow_pointer_store(
        &mut self,
        contract: super::super::select::pointer_stores::Contract,
    ) {
        self.request(Request::AllowPointerStore(contract), |this| {
            assert_eq!(this.recording.source, Some(contract.site()));
            assert!(this.pointer_store.is_none());
            this.pointer_store = Some(contract);
        });
    }

    pub(super) fn pointer_effects(&mut self, effects: &effects::InstructionEffects) {
        let Some(pointer) = self.pointer else {
            return;
        };
        let overlap = |a: i64, n: i64, b: i64, m: i64| a < b + m && b < a + n;
        let clobbered = effects.environment_writes
            & (effects::env::S | effects::env::D | effects::env::DBR | effects::env::E)
            != 0
            || !matches!(effects.control, effects::Control::Next)
            || effects.memory.iter().any(|access| {
                if access.access == Access::Read {
                    return false;
                }
                match access.memory {
                    Memory::Stack {
                        displacement,
                        bytes,
                    } => overlap(
                        i64::from(displacement) - self.state.env.depth,
                        i64::from(bytes),
                        i64::from(pointer.source.offset) - pointer.depth,
                        3,
                    ),
                    Memory::DirectPage { offset, bytes } => overlap(
                        i64::from(offset),
                        i64::from(bytes),
                        i64::from(pointer.scratch),
                        3,
                    ),
                    Memory::IndirectLong {
                        pointer: slot,
                        indexed_y,
                        bytes,
                    } => {
                        let offset = if indexed_y {
                            match self.state.y {
                                Value::Constant(n, Width::Word) => Some(n),
                                _ => None,
                            }
                        } else {
                            Some(0)
                        };
                        !(slot == pointer.scratch
                            && self.state.env.current_domain
                            && self.state.env.native
                            && self.state.env.index == Width::Word
                            && self.state.env.depth == pointer.depth
                            && self
                                .pointer_store
                                .zip(offset)
                                .is_some_and(|(contract, offset)| {
                                    contract.covers(pointer.origin, pointer.source, offset, bytes)
                                }))
                    }
                    // Absolute, symbolic and indirect targets retain the same
                    // conservative may-alias rule as physical home analysis.
                    _ => true,
                }
            });
        if clobbered {
            self.invalidate_pointer();
        }
    }

    /// Both hit and miss go through the same typed, replayed request. A miss
    /// establishes the fact only after emitting all three real source bytes.
    pub fn stage_pointer(&mut self, origin: PointerOrigin, source: Slot, scratch: u8) -> bool {
        self.request(Request::StagePointer(origin, source, scratch), |this| {
            this.live();
            assert_eq!(source.width, 3);
            assert_eq!(this.state.env.index, Width::Word);
            assert!(this.state.env.current_domain && this.state.env.native);
            assert_eq!(this.state.delta(), 0);
            assert!(abi::scratch_contains(u32::from(scratch), 3));
            let offset = abi::stack::access_displacement(
                ByteOffset::new(source.offset.into()),
                ByteSize::new(3),
                ByteSize::ZERO,
            )
            .expect("preflighted pointer source")
            .get() as u8;
            let requested = Resident {
                origin,
                source,
                scratch,
                depth: this.state.env.depth,
                generation: this.pointer_generation,
            };
            if this.pointer == Some(requested) {
                return true;
            }
            this.a16();
            this.byte(ByteOp::LdaStack, offset);
            this.byte(ByteOp::StaDp, scratch);
            this.byte(ByteOp::LdaStack, offset + 1);
            this.byte(ByteOp::StaDp, scratch + 1);
            this.pointer = Some(Resident {
                generation: this.pointer_generation,
                ..requested
            });
            false
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_cache_uses_identities_and_invalidates_every_overlapping_write() {
        let mut e = TrackedEmitter65816::default();
        e.test_frame(12);
        let source = Slot {
            offset: 16,
            width: 3,
        };
        let origin = PointerOrigin::Parameter(ParamId(0));
        let ptr = abi::generated::DP_POINTER0_OFFSET as u8;
        assert!(!e.stage_pointer(origin, source, ptr));
        let at = e.position();
        assert!(e.stage_pointer(origin, source, ptr));
        assert_eq!(e.position(), at);
        e.word(WordOp::LdyImm, 3);
        e.byte(ByteOp::LdaIndirectY, ptr);
        e.byte(ByteOp::StaStack, 2); // disjoint private capture
        assert!(e.stage_pointer(origin, source, ptr));
        for byte in 0..3 {
            e.a8();
            e.byte(ByteOp::StaDp, ptr + byte);
            assert!(!e.stage_pointer(origin, source, ptr));
            e.a8();
            e.byte(ByteOp::StaStack, source.offset as u8 + byte);
            assert!(!e.stage_pointer(origin, source, ptr));
        }
        e.byte(ByteOp::StaIndirectY, ptr);
        assert!(!e.stage_pointer(origin, source, ptr));
        e.forget_pointer();
        assert!(!e.stage_pointer(origin, source, ptr));
        assert!(!e.stage_pointer(PointerOrigin::Temporary(TempId(1)), source, ptr));
        let join = e.label();
        e.mark(join);
        assert!(!e.stage_pointer(origin, source, ptr));
    }
}
