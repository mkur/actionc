//! Bounded, isolated MC68000 test machine. Guest instructions are executed by
//! pinned r68k, never by a compiler-IR interpreter.
pub mod artifacts;

use r68k::cpu::{Callbacks, ConfiguredCore, Core, Cycles, Exception, ProcessingState};
use r68k::interrupts::AutoInterruptController;
use r68k::ram::{ADDRBUS_MASK, AddressBus, AddressSpace};
use std::cell::RefCell;
use std::collections::VecDeque;

pub const MEMORY_SIZE: usize = 0x10_0000;
pub const TRAMPOLINE: u32 = 0x400;
pub const STACK_BOTTOM: u32 = 0xe0000;
pub const STACK_TOP: u32 = 0xff000;
const READ: u8 = 1;
const WRITE: u8 = 2;
const EXECUTE: u8 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryViolation {
    pub address: u32,
    pub write: bool,
    pub instruction: bool,
}

#[derive(Clone)]
pub struct Memory {
    bytes: Vec<u8>,
    permissions: Vec<u8>,
    violation: RefCell<Option<MemoryViolation>>,
    trace: RefCell<Option<(std::ops::Range<u32>, Vec<(u32, bool)>)>>,
}

impl Default for Memory {
    fn default() -> Self {
        Self {
            // Only explicit BSS and stack mapping are initialized by the loader.
            bytes: vec![0xcd; MEMORY_SIZE],
            permissions: vec![0; MEMORY_SIZE],
            violation: RefCell::new(None),
            trace: RefCell::new(None),
        }
    }
}

impl Memory {
    /// Opt-in byte bus observations for volatile-access regression tests.
    pub fn trace_range(&self, range: std::ops::Range<u32>) {
        *self.trace.borrow_mut() = Some((range, Vec::new()));
    }
    pub fn take_trace(&self) -> Vec<(u32, bool)> {
        self.trace
            .borrow_mut()
            .take()
            .map(|(_, events)| events)
            .unwrap_or_default()
    }
    fn record_access(&self, address: u32, write: bool) {
        if let Some((range, events)) = self.trace.borrow_mut().as_mut() {
            if range.contains(&address) {
                events.push((address, write));
            }
        }
    }

    pub fn map(
        &mut self,
        address: u32,
        bytes: &[u8],
        writable: bool,
        executable: bool,
    ) -> Result<(), String> {
        let end = (address as usize)
            .checked_add(bytes.len())
            .ok_or("mapping overflow")?;
        let range = address as usize..end;
        let permissions = self
            .permissions
            .get_mut(range.clone())
            .ok_or("mapping outside RAM")?;
        if permissions.iter().any(|p| *p != 0) {
            return Err(format!("overlapping memory mapping at {address:08x}"));
        }
        permissions
            .fill(READ | if writable { WRITE } else { 0 } | if executable { EXECUTE } else { 0 });
        self.bytes[range].copy_from_slice(bytes);
        Ok(())
    }

    pub fn bytes(&self, address: u32, len: usize) -> Result<&[u8], String> {
        let end = (address as usize).checked_add(len).ok_or("read overflow")?;
        let range = address as usize..end;
        if self
            .permissions
            .get(range.clone())
            .is_none_or(|p| p.iter().any(|p| p & READ == 0))
        {
            return Err(format!("host read of unmapped memory at {address:08x}"));
        }
        Ok(&self.bytes[range])
    }

    pub fn write(&mut self, address: u32, bytes: &[u8]) -> Result<(), String> {
        let end = (address as usize)
            .checked_add(bytes.len())
            .ok_or("write overflow")?;
        let range = address as usize..end;
        if self
            .permissions
            .get(range.clone())
            .is_none_or(|p| p.iter().any(|p| p & WRITE == 0))
        {
            return Err(format!(
                "host write of protected/unmapped memory at {address:08x}"
            ));
        }
        self.bytes[range].copy_from_slice(bytes);
        Ok(())
    }

    fn allowed(&self, address: u32, write: bool, instruction: bool) -> bool {
        let required = if write {
            WRITE
        } else if instruction {
            EXECUTE
        } else {
            READ
        };
        let allowed = self
            .permissions
            .get(address as usize)
            .is_some_and(|p| p & required != 0);
        if !allowed {
            self.violation.borrow_mut().get_or_insert(MemoryViolation {
                address,
                write,
                instruction,
            });
        }
        allowed
    }
}

impl AddressBus for Memory {
    fn copy_from(&mut self, other: &Self) {
        *self = other.clone();
    }
    fn read_byte(&self, space: AddressSpace, address: u32) -> u32 {
        let address = address & ADDRBUS_MASK;
        self.record_access(address, false);
        if self.allowed(address, false, space.fc() & 2 != 0) {
            self.bytes[address as usize] as u32
        } else {
            0
        }
    }
    fn read_word(&self, space: AddressSpace, address: u32) -> u32 {
        self.read_byte(space, address) << 8 | self.read_byte(space, address.wrapping_add(1))
    }
    fn read_long(&self, space: AddressSpace, address: u32) -> u32 {
        self.read_word(space, address) << 16 | self.read_word(space, address.wrapping_add(2))
    }
    fn write_byte(&mut self, _: AddressSpace, address: u32, value: u32) {
        let address = address & ADDRBUS_MASK;
        self.record_access(address, true);
        if self.allowed(address, true, false) {
            self.bytes[address as usize] = value as u8;
        }
    }
    fn write_word(&mut self, space: AddressSpace, address: u32, value: u32) {
        self.write_byte(space, address, value >> 8);
        self.write_byte(space, address.wrapping_add(1), value);
    }
    fn write_long(&mut self, space: AddressSpace, address: u32, value: u32) {
        self.write_word(space, address, value >> 16);
        self.write_word(space, address.wrapping_add(2), value);
    }
}

#[derive(Default)]
struct Exceptions(Option<Exception>);
impl Callbacks for Exceptions {
    fn exception_callback(
        &mut self,
        _: &mut impl Core,
        ex: Exception,
    ) -> r68k::cpu::Result<Cycles> {
        self.0 = Some(ex);
        // Consume the one-cycle step budget; do not dispatch guest vectors or
        // return zero (which could execute another instruction in this call).
        Ok(Cycles(1))
    }
}

#[derive(Debug, Clone)]
pub enum Outcome {
    Completed,
    Exception(Exception),
    MemoryViolation(MemoryViolation),
    Stopped,
    Halted,
    BudgetExhausted,
    AbiViolation(String),
}

#[derive(Debug, Clone)]
pub struct Step {
    pub pc: u32,
    pub opcode: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct RunResult {
    pub outcome: Outcome,
    /// Attempted CPU instructions, including a faulting instruction or trap.
    pub steps: u64,
    /// r68k cycles; intercepted exceptions consume one synthetic cycle.
    pub cycles: u64,
    pub pc: u32,
    pub registers: [u32; 16],
    pub history: VecDeque<Step>,
}

impl RunResult {
    pub fn assert_completed(&self) {
        assert!(matches!(self.outcome, Outcome::Completed), "{self:#?}");
    }
}

pub struct Machine {
    pub cpu: ConfiguredCore<AutoInterruptController, Memory>,
    initial_registers: [u32; 16],
}

impl Machine {
    pub fn new(mut memory: Memory, entry: u32) -> Result<Self, String> {
        if entry & 1 != 0 || entry > ADDRBUS_MASK {
            return Err("invalid entry address".into());
        }
        // JSR absolute long; TRAP #15. Padding permits r68k's legal prefetch.
        let mut trampoline = vec![0x4e, 0xb9];
        trampoline.extend_from_slice(&entry.to_be_bytes());
        trampoline.extend_from_slice(&[0x4e, 0x4f, 0x4e, 0x71, 0x4e, 0x71]);
        memory.map(TRAMPOLINE, &trampoline, false, true)?;
        let mut vectors = vec![0; 0x400];
        vectors[..4].copy_from_slice(&STACK_TOP.to_be_bytes());
        vectors[4..8].copy_from_slice(&TRAMPOLINE.to_be_bytes());
        // r68k fetches reset vectors through its program-space accessor.
        memory.map(0, &vectors, false, true)?;
        memory.map(
            STACK_BOTTOM,
            &vec![0xa7; (STACK_TOP - STACK_BOTTOM) as usize],
            true,
            false,
        )?;
        let mut cpu = ConfiguredCore::new_with(0, AutoInterruptController::new(), memory);
        cpu.reset();
        cpu.mem.permissions[..0x400].fill(READ);
        for (index, value) in cpu.dar[..15].iter_mut().enumerate() {
            *value = 0x13570000 | (index as u32 * 0x101);
        }
        let initial_registers = cpu.dar;
        Ok(Self {
            cpu,
            initial_registers,
        })
    }

    pub fn run(&mut self, instruction_budget: u64) -> RunResult {
        let mut history = VecDeque::new();
        let mut steps = 0;
        let mut cycles = 0;
        let outcome = loop {
            if let Some(fault) = self.cpu.mem.violation.borrow().clone() {
                break Outcome::MemoryViolation(fault);
            }
            match self.cpu.processing_state {
                ProcessingState::Stopped => break Outcome::Stopped,
                ProcessingState::Halted => break Outcome::Halted,
                _ => {}
            }
            if steps == instruction_budget {
                break Outcome::BudgetExhausted;
            }
            let pc = self.cpu.pc;
            let opcode = self
                .cpu
                .mem
                .bytes(pc & ADDRBUS_MASK, 2)
                .ok()
                .map(|b| u16::from_be_bytes([b[0], b[1]]));
            if history.len() == 32 {
                history.pop_front();
            }
            history.push_back(Step { pc, opcode });
            let mut exceptions = Exceptions::default();
            // A positive one-cycle budget executes at most one instruction;
            // instructions cost more than one cycle. This is not execute(steps).
            cycles += self.cpu.execute_with_state(1, &mut exceptions).0 as u64;
            steps += 1;
            if let Some(fault) = self.cpu.mem.violation.borrow().clone() {
                break Outcome::MemoryViolation(fault);
            }
            if let Some(exception) = exceptions.0 {
                if matches!(exception, Exception::Trap(47, _)) && pc == TRAMPOLINE + 6 {
                    let changed = (2..8)
                        .chain(10..16)
                        .find(|&i| self.cpu.dar[i] != self.initial_registers[i]);
                    break match changed {
                        Some(i) => Outcome::AbiViolation(format!(
                            "register {}{} changed: {:08x} -> {:08x}",
                            if i < 8 { 'D' } else { 'A' },
                            i % 8,
                            self.initial_registers[i],
                            self.cpu.dar[i]
                        )),
                        None => Outcome::Completed,
                    };
                }
                break Outcome::Exception(exception);
            }
        };
        RunResult {
            outcome,
            steps,
            cycles,
            pc: self.cpu.pc,
            registers: self.cpu.dar,
            history,
        }
    }
}

impl Machine {
    pub fn from_image(image: &actionc::mir68k::image::NativeImage) -> Result<Self, String> {
        image.verify()?;
        let mut memory = Memory::default();
        for segment in &image.segments {
            memory.map(
                segment.address,
                &segment.bytes,
                segment.writable,
                segment.executable,
            )?;
        }
        for zero in &image.zero_fill {
            memory.map(
                zero.address,
                &vec![0; zero.size as usize],
                zero.writable,
                false,
            )?;
        }
        Self::new(memory, image.entry)
    }

    pub fn read_scalar(&self, symbol: &actionc::mir68k::image::Symbol) -> Result<u32, String> {
        let width = symbol
            .ty
            .as_ref()
            .and_then(|t| t.width)
            .ok_or("symbol has no scalar type")?
            .get() as usize;
        if symbol.array.is_some()
            || symbol.size != width as u32
            || !matches!(width, 1 | 2 | 4)
            || !matches!(
                symbol.ty.as_ref().unwrap().kind,
                actionc::nir::NirTypeKind::Integer(_)
                    | actionc::nir::NirTypeKind::Bool
                    | actionc::nir::NirTypeKind::Pointer { .. }
                    | actionc::nir::NirTypeKind::Callable { .. }
            )
        {
            return Err("symbol is not a supported scalar".into());
        }
        Ok(self
            .cpu
            .mem
            .bytes(symbol.address()?, width)?
            .iter()
            .fold(0, |n, b| (n << 8) | u32::from(*b)))
    }

    pub fn write_scalar(
        &mut self,
        symbol: &actionc::mir68k::image::Symbol,
        value: u32,
    ) -> Result<(), String> {
        let width = symbol
            .ty
            .as_ref()
            .and_then(|t| t.width)
            .ok_or("symbol has no scalar type")?
            .get() as usize;
        if symbol.array.is_some()
            || symbol.size != width as u32
            || !matches!(width, 1 | 2 | 4)
            || !matches!(
                symbol.ty.as_ref().unwrap().kind,
                actionc::nir::NirTypeKind::Integer(_)
                    | actionc::nir::NirTypeKind::Bool
                    | actionc::nir::NirTypeKind::Pointer { .. }
                    | actionc::nir::NirTypeKind::Callable { .. }
            )
        {
            return Err("symbol is not a supported scalar".into());
        }
        self.cpu
            .mem
            .write(symbol.address()?, &value.to_be_bytes()[4 - width..])
    }
}

impl Machine {
    fn array_layout(
        &self,
        symbol: &actionc::mir68k::image::Symbol,
    ) -> Result<(u32, u32, u32, u32), String> {
        let array = symbol.array.as_ref().ok_or("symbol is not an array")?;
        let count = array.count.ok_or("array has no known element count")?;
        if !matches!(array.element_width, 1 | 2 | 4) || array.stride < array.element_width {
            return Err("unsupported array element layout".into());
        }
        // The emitted backing address describes initial storage. A mutable
        // descriptor may have been rebound since the image was loaded.
        let base = if array.descriptor {
            u32::from_be_bytes(
                self.cpu
                    .mem
                    .bytes(symbol.address()?, 4)?
                    .try_into()
                    .unwrap(),
            )
        } else {
            array.backing_address.unwrap_or(symbol.address()?)
        };
        base.checked_add(
            count
                .checked_mul(array.stride)
                .ok_or("array extent overflow")?,
        )
        .ok_or("array address overflow")?;
        Ok((base, count, array.stride, array.element_width))
    }
    pub fn read_array(&self, symbol: &actionc::mir68k::image::Symbol) -> Result<Vec<u32>, String> {
        let (base, count, stride, width) = self.array_layout(symbol)?;
        (0..count)
            .map(|i| {
                self.cpu
                    .mem
                    .bytes(base + i * stride, width as usize)
                    .map(|b| b.iter().fold(0, |n, b| (n << 8) | u32::from(*b)))
            })
            .collect()
    }
    pub fn write_array(
        &mut self,
        symbol: &actionc::mir68k::image::Symbol,
        values: &[u32],
    ) -> Result<(), String> {
        let (base, count, stride, width) = self.array_layout(symbol)?;
        if values.len() != count as usize {
            return Err("array input length does not match symbol count".into());
        }
        for (index, value) in values.iter().enumerate() {
            self.cpu.mem.write(
                base + index as u32 * stride,
                &value.to_be_bytes()[4 - width as usize..],
            )?;
        }
        Ok(())
    }
}
