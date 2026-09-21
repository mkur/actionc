//! Native machine facts. Values are immutable identities, never mutable aliases.
use super::{Slot, TempId};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    Byte,
    Word,
}
impl Width {
    pub(super) fn bytes(self) -> u8 {
        if self == Self::Byte { 1 } else { 2 }
    }
    pub(super) fn mask(self) -> u16 {
        if self == Self::Byte { 0xff } else { 0xffff }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value {
    Unknown,
    Constant(u16, Width),
    Opaque(u64, Width),
    /// Entry S plus a checked constant. Used only for selected stack equations.
    StackAddress(i32),
}
impl Value {
    pub(super) fn width(self) -> Option<Width> {
        match self {
            Self::Unknown => None,
            Self::Constant(_, w) | Self::Opaque(_, w) => Some(w),
            Self::StackAddress(_) => Some(Width::Word),
        }
    }
    pub(super) fn matches(self, other: Self) -> bool {
        self != Self::Unknown && self == other
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Environment {
    pub m: Width,
    pub index: Width,
    pub native: bool,
    pub decimal: Option<bool>,
    pub dbr: Option<u8>,
    pub current_domain: bool,
    pub irq_preserved: bool,
    /// Positive down from invocation entry. Includes transfer pushes.
    pub depth: i32,
    pub anchor: Option<i32>,
    pub pushes: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Home {
    pub generation: u64,
    pub value: Value,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct AdjacentWord {
    pub temp: TempId,
    pub slot: Slot,
    pub value: Value,
    pub generation: u64,
    pub cursor: (usize, usize),
}
#[derive(Clone, Debug)]
pub(super) struct State65816 {
    pub env: Environment,
    pub a: Value,
    pub x: Value,
    pub y: Value,
    pub nz: Value,
    pub carry: Option<bool>,
    pub overflow: Option<bool>,
    /// Permission from an explicit width request; revoked at every label.
    pub mode_permission: bool,
    pub adjacent: Option<AdjacentWord>,
    pub homes: BTreeMap<(u16, u8), Home>,
    private_ranges: BTreeSet<(u16, u8)>,
    next: u64,
    pub peak: i32,
}
impl Default for State65816 {
    fn default() -> Self {
        Self {
            env: Environment {
                m: Width::Word,
                index: Width::Word,
                native: true,
                decimal: Some(false),
                dbr: Some(0),
                current_domain: true,
                irq_preserved: true,
                depth: 0,
                anchor: None,
                pushes: 0,
            },
            a: Value::Unknown,
            x: Value::Unknown,
            y: Value::Unknown,
            nz: Value::Unknown,
            carry: None,
            overflow: None,
            mode_permission: false,
            adjacent: None,
            homes: BTreeMap::new(),
            private_ranges: BTreeSet::new(),
            next: 0,
            peak: 0,
        }
    }
}
impl State65816 {
    pub fn fresh(&mut self, width: Width) -> Value {
        self.next += 1;
        Value::Opaque(self.next, width)
    }
    pub fn constant(value: u16, width: Width) -> Value {
        Value::Constant(value & width.mask(), width)
    }
    pub fn values_barrier(&mut self) {
        self.a = Value::Unknown;
        self.x = Value::Unknown;
        self.y = Value::Unknown;
        self.nz = Value::Unknown;
        self.carry = None;
        self.overflow = None;
        self.homes.clear();
        self.adjacent = None;
    }
    pub fn delta(&self) -> u32 {
        assert_eq!(self.env.pushes, 0, "frame addressing during transfer");
        self.env.anchor.map_or(0, |anchor| {
            u32::try_from(self.env.depth - anchor).expect("frame already released")
        })
    }
    pub fn register_home(&mut self, slot: Slot) {
        self.private_ranges.insert((slot.offset, slot.width));
    }
    pub fn stack_key(&self, offset: u8, width: Width) -> Option<(u16, u8)> {
        let offset = i32::from(offset) - i32::try_from(self.delta()).unwrap();
        u16::try_from(offset).ok().map(|o| (o, width.bytes()))
    }
    pub fn read_stack(&mut self, offset: u8, width: Width) -> Value {
        self.stack_key(offset, width)
            .and_then(|k| self.homes.get(&k))
            .map(|h| h.value)
            .unwrap_or_else(|| self.fresh(width))
    }
    pub fn write_stack(&mut self, offset: u8, width: Width) {
        if let Some(key) = self.stack_key(offset, width) {
            self.homes.retain(|&(start, len), _| {
                u32::from(start) + u32::from(len) <= u32::from(key.0)
                    || u32::from(key.0) + u32::from(key.1) <= u32::from(start)
            });
            if self.private_ranges.contains(&key) && self.a.width() == Some(width) {
                self.next += 1;
                self.homes.insert(
                    key,
                    Home {
                        generation: self.next,
                        value: self.a,
                    },
                );
            }
        } else {
            self.homes.clear();
        }
    }
    pub fn unknown_write(&mut self) {
        self.homes.clear();
    }
    pub fn load_a(&mut self, value: Value) {
        self.a = value;
        self.nz = value;
    }
    pub fn narrow(&mut self, value: Value, width: Width) -> Value {
        match value {
            Value::Constant(v, _) => Self::constant(v, width),
            v if v.width() == Some(width) => v,
            _ => self.fresh(width),
        }
    }
    pub fn status(&mut self, mask: u8, set: bool) {
        if mask & 0x20 != 0 {
            let width = if set { Width::Byte } else { Width::Word };
            if self.env.m != width {
                self.a = Value::Unknown;
            }
            self.env.m = width;
        }
        if mask & 0x10 != 0 {
            let width = if set { Width::Byte } else { Width::Word };
            if set {
                self.x = self.narrow(self.x, width);
                self.y = self.narrow(self.y, width);
            } else if self.env.index != width {
                self.x = Value::Unknown;
                self.y = Value::Unknown;
            }
            self.env.index = width;
        }
        if mask & 1 != 0 {
            self.carry = Some(set);
        }
        if mask & 0x40 != 0 {
            self.overflow = Some(set);
        }
        if mask & 0x82 != 0 {
            self.nz = Value::Unknown;
        }
        if mask & 8 != 0 {
            self.env.decimal = Some(set);
        }
        if mask & 4 != 0 {
            self.env.irq_preserved = false;
        }
    }
    pub fn arithmetic(&mut self, rhs: Value, subtract: bool) {
        let width = self.env.m;
        let carry = self.carry;
        let left = self.a;
        let result = match (left, rhs, carry, self.env.decimal) {
            (Value::StackAddress(s), Value::Constant(v, Width::Word), Some(c), Some(false))
                if width == Width::Word && c == subtract =>
            {
                Value::StackAddress(if subtract {
                    s - i32::from(v)
                } else {
                    s + i32::from(v)
                })
            }
            (Value::Constant(a, w), Value::Constant(b, v), Some(c), Some(false))
                if w == width && v == width =>
            {
                let operand = if subtract { b ^ width.mask() } else { b };
                let sum = u32::from(a) + u32::from(operand) + u32::from(c);
                let result = sum as u16 & width.mask();
                self.carry = Some(sum > u32::from(width.mask()));
                let sign = (width.mask() >> 1) + 1;
                self.overflow = Some((!(a ^ operand) & (a ^ result) & sign) != 0);
                self.load_a(Self::constant(result, width));
                return;
            }
            _ => self.fresh(width),
        };
        self.carry = None;
        self.overflow = None;
        self.load_a(result);
    }
    pub fn compare(&mut self, rhs: Value) {
        match (self.a, rhs) {
            (Value::Constant(a, w), Value::Constant(b, v)) if w == self.env.m && v == w => {
                self.nz = Self::constant(a.wrapping_sub(b), w);
                self.carry = Some(a >= b);
            }
            _ => {
                self.nz = self.fresh(self.env.m);
                self.carry = None;
            }
        }
    }
    pub fn call_return(&mut self) {
        assert_eq!((self.env.m, self.env.index), (Width::Word, Width::Word));
        self.values_barrier();
        self.env.decimal = Some(false);
        self.env.dbr = Some(0);
        self.env.current_domain = true;
        self.a = self.fresh(Width::Word);
        self.x = self.fresh(Width::Word);
    }
    pub fn push(&mut self, bytes: u8) {
        self.adjacent = None;
        self.env.pushes += bytes;
        self.env.depth += i32::from(bytes);
        self.peak = self.peak.max(self.env.depth);
    }
    pub fn publish_word(&mut self, temp: TempId, slot: Slot, cursor: (usize, usize)) {
        self.adjacent = None;
        if self.delta() != 0 || slot.width != 2 || self.env.m != Width::Word {
            return;
        }
        let Some(home) = self.homes.get(&(slot.offset, slot.width)) else {
            return;
        };
        assert!(
            self.a.width() == Some(Width::Word)
                && self.a.matches(home.value)
                && self.nz.matches(self.a),
            "producer must already prove A16/home/NZ"
        );
        self.adjacent = Some(AdjacentWord {
            temp,
            slot,
            value: home.value,
            generation: home.generation,
            cursor,
        });
    }
    pub fn consume_word(
        &mut self,
        temp: Option<TempId>,
        slot: Option<Slot>,
        offset: Option<u8>,
        cursor: Option<(usize, usize)>,
    ) -> bool {
        let Some(fact) = self.adjacent.take() else {
            return false;
        };
        temp == Some(fact.temp)
            && slot == Some(fact.slot)
            && offset.map(u16::from) == Some(fact.slot.offset)
            && self.delta() == 0
            && cursor == Some(fact.cursor)
            && self.env.m == Width::Word
            && self.a.matches(fact.value)
            && self.nz.matches(fact.value)
            && self
                .homes
                .get(&(fact.slot.offset, 2))
                .is_some_and(|h| h.generation == fact.generation && h.value.matches(fact.value))
    }
}
