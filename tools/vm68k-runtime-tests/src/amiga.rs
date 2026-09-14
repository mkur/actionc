//! Bounded classic OS-call shim, deliberately independent of compiler vectors.
//! This validates adapter code; it is not an AmigaOS compatibility emulator.
use crate::{Machine, Memory, RunResult};
use r68k::{cpu::ConfiguredCore, interrupts::AutoInterruptController};
use std::collections::VecDeque;

pub const EXEC_BASE: u32 = 0x8000;
pub const DOS_BASE: u32 = 0xa000;
pub const OUTPUT_HANDLE: u32 = 0x101;
const STUBS: &[(u32, Call)] = &[
    (EXEC_BASE - 552, Call::OpenLibrary),
    (EXEC_BASE - 414, Call::CloseLibrary),
    (DOS_BASE - 60, Call::Output),
    (DOS_BASE - 48, Call::Write),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    OpenLibrary,
    CloseLibrary,
    Output,
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub call: Call,
    pub args: Vec<u32>,
}

#[derive(Debug, Default)]
pub struct Os {
    pub events: Vec<Event>,
    pub output: Vec<u8>,
    pub fail_open: bool,
    pub missing_output: bool,
    pub write_results: VecDeque<i32>,
    pub open_count: u32,
    pub close_count: u32,
}

impl Os {
    pub fn install(&self, machine: &mut Machine) -> Result<(), String> {
        for &(address, _) in STUBS {
            // TRAP #13; RTS; prefetch NOP. No compiler encoder is used here.
            let start = address & !3;
            let offset = (address - start) as usize;
            let mut bytes = [0x4e, 0x71].repeat(6);
            bytes[offset..offset + 4].copy_from_slice(&[0x4e, 0x4d, 0x4e, 0x75]);
            machine.cpu.mem.map(start, &bytes, false, true)?;
        }
        // The reset vector has already been consumed. Classic Amiga programs
        // see ExecBase at address four, protected from subsequent guest writes.
        machine.cpu.mem.bytes[4..8].copy_from_slice(&EXEC_BASE.to_be_bytes());
        Ok(())
    }

    pub fn run(&mut self, machine: &mut Machine, budget: u64) -> RunResult {
        machine.run_with_traps(budget, &mut |pc, vector, cpu| self.trap(pc, vector, cpu))
    }

    fn trap(
        &mut self,
        pc: u32,
        vector: u8,
        cpu: &mut ConfiguredCore<AutoInterruptController, Memory>,
    ) -> Result<bool, String> {
        if vector != 45 {
            return Ok(false);
        }
        let Some(&(_, call)) = STUBS.iter().find(|(address, _)| *address == pc) else {
            return Ok(false);
        };
        let expected_base = if matches!(call, Call::OpenLibrary | Call::CloseLibrary) {
            EXEC_BASE
        } else {
            DOS_BASE
        };
        if cpu.dar[14] != expected_base {
            return Err("incorrect Amiga library base in A6".into());
        }
        let (args, result) = match call {
            Call::OpenLibrary => {
                let name = cpu.dar[9];
                if cpu.mem.bytes(name, 12)? != b"dos.library\0" {
                    return Err("unexpected OpenLibrary name".into());
                }
                let result = if self.fail_open || cpu.dar[0] > 40 {
                    0
                } else {
                    DOS_BASE
                };
                if result != 0 {
                    self.open_count += 1;
                }
                (vec![name, cpu.dar[0]], result)
            }
            Call::CloseLibrary => {
                if cpu.dar[9] != DOS_BASE {
                    return Err("closing an unowned library".into());
                }
                if self.close_count == self.open_count {
                    return Err("unmatched CloseLibrary".into());
                }
                self.close_count += 1;
                (vec![cpu.dar[9]], 0xdead_beef)
            }
            Call::Output => (
                vec![],
                if self.missing_output {
                    0
                } else {
                    OUTPUT_HANDLE
                },
            ),
            Call::Write => {
                let (handle, pointer, count) = (cpu.dar[1], cpu.dar[2], cpu.dar[3]);
                if handle != OUTPUT_HANDLE {
                    return Err("Write changed the borrowed BPTR handle".into());
                }
                let count = i32::try_from(count).map_err(|_| "negative DOS Write count")?;
                let result = self.write_results.pop_front().unwrap_or(count);
                if result > count {
                    return Err("invalid injected Write result".into());
                }
                if result > 0 {
                    self.output.extend(cpu.mem.bytes(pointer, result as usize)?);
                }
                (vec![handle, pointer, count as u32], result as u32)
            }
        };
        self.events.push(Event { call, args });
        for r in [0, 1, 8, 9] {
            cpu.dar[r] = 0xdead_0000 | r as u32;
        }
        cpu.ccr_to_flags(0x1f);
        cpu.dar[0] = result;
        Ok(true)
    }
}
