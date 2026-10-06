//! Canonical aggregate byte-transfer protocol. Both complete addresses are
//! captured before entry; replay reconstructs direction, extent and all loops.
use super::super::resources::{COPY_COUNT, COPY_SOURCE, PTR};
use super::*;

impl TrackedEmitter65816 {
    pub(in crate::mir65816::emit) fn aggregate_copy(&mut self, bytes: u32, overlap_safe: bool) {
        self.request(
            Request::AggregateCopy {
                bytes,
                overlap_safe,
            },
            |this| {
                assert!(bytes > 0 && bytes < 1 << 24);
                assert_eq!(this.state.delta(), 0);
                this.a8();
                let forward = this.label();
                let backward = this.label();
                let done = this.label();
                if overlap_safe {
                    for i in (0..3).rev() {
                        this.byte(ByteOp::LdaDp, PTR + i);
                        this.byte(ByteOp::CmpDp, COPY_SOURCE + i);
                        this.branch(Branch::CarryClear, forward);
                        this.branch(Branch::NotEqual, backward);
                    }
                    this.jump(done); // identical source/destination
                    this.mark(backward);
                    this.step_pointer(PTR, false, bytes - 1);
                    this.step_pointer(COPY_SOURCE, false, bytes - 1);
                    this.aggregate_loop(bytes, true);
                    this.jump(done);
                }
                this.mark(forward);
                this.aggregate_loop(bytes, false);
                this.mark(done);
            },
        );
    }

    pub(in crate::mir65816::emit) fn step_pointer(
        &mut self,
        pointer: u8,
        subtract: bool,
        amount: u32,
    ) {
        self.op(if subtract { Implied::Sec } else { Implied::Clc });
        for i in 0..3 {
            self.byte(ByteOp::LdaDp, pointer + i);
            self.byte(
                if subtract {
                    ByteOp::SbcImm
                } else {
                    ByteOp::AdcImm
                },
                (amount >> (i * 8)) as u8,
            );
            self.byte(ByteOp::StaDp, pointer + i);
        }
    }

    fn aggregate_loop(&mut self, bytes: u32, backward: bool) {
        for i in 0..3 {
            self.byte(ByteOp::LdaImm, (bytes >> (i * 8)) as u8);
            self.byte(ByteOp::StaDp, COPY_COUNT + i);
        }
        let again = self.label();
        self.mark(again);
        self.byte(ByteOp::LdaIndirect, COPY_SOURCE);
        self.byte(ByteOp::StaIndirect, PTR);
        self.step_pointer(PTR, backward, 1);
        self.step_pointer(COPY_SOURCE, backward, 1);
        self.step_pointer(COPY_COUNT, true, 1);
        self.byte(ByteOp::LdaDp, COPY_COUNT);
        self.byte(ByteOp::OraDp, COPY_COUNT + 1);
        self.byte(ByteOp::OraDp, COPY_COUNT + 2);
        self.branch(Branch::NotEqual, again);
    }
}
