//! Checked stack arithmetic shared by planning and future instruction allocation.
use super::generated::*;
use crate::target::{ByteOffset, ByteSize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StackError {
    ArithmeticOverflow,
    FrameTooLarge { bytes: u32 },
    AccessOutOfRange { displacement: u32, width: u32 },
    ReservationOutOfBounds,
}

impl fmt::Display for StackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArithmeticOverflow => f.write_str("65816 stack arithmetic overflow"),
            Self::FrameTooLarge { bytes } => write!(
                f,
                "65816 native fixed frame requires {bytes} bytes; ABI v1 supports at most {FRAME_INITIAL_STRATEGY_MAX_FIXED_EXTENT}"
            ),
            Self::AccessOutOfRange {
                displacement,
                width,
            } => write!(
                f,
                "65816 stack-relative access at {displacement} with width {width} exceeds the supported displacement range 1..=255"
            ),
            Self::ReservationOutOfBounds => {
                f.write_str("65816 stack reservation exceeds its domain bounds or wraps bank zero")
            }
        }
    }
}

impl std::error::Error for StackError {}

pub fn fixed_extent(used_bytes: ByteSize) -> Result<ByteSize, StackError> {
    let bytes = super::align_up(used_bytes.get(), FRAME_FIXED_EXTENT_MULTIPLE)
        .ok_or(StackError::ArithmeticOverflow)?;
    if bytes > FRAME_INITIAL_STRATEGY_MAX_FIXED_EXTENT {
        return Err(StackError::FrameTooLarge { bytes });
    }
    Ok(ByteSize::new(bytes))
}

/// S moved downward by `reserved_below_body` bytes since the referenced body
/// home was established. Validate the last accessed byte, not just its start.
pub fn access_displacement(
    body_offset: ByteOffset,
    width: ByteSize,
    reserved_below_body: ByteSize,
) -> Result<ByteOffset, StackError> {
    let displacement = body_offset
        .get()
        .checked_add(reserved_below_body.get())
        .ok_or(StackError::ArithmeticOverflow)?;
    let last = displacement
        .checked_add(width.get().saturating_sub(1))
        .ok_or(StackError::ArithmeticOverflow)?;
    if width.is_zero()
        || displacement < FRAME_STACK_RELATIVE_MIN_DISPLACEMENT
        || last > FRAME_STACK_RELATIVE_MAX_LAST_BYTE_DISPLACEMENT
    {
        return Err(StackError::AccessOutOfRange {
            displacement,
            width: width.get(),
        });
    }
    Ok(ByteOffset::new(displacement))
}

pub fn incoming_displacement(
    frame: ByteSize,
    offset: ByteOffset,
    width: ByteSize,
) -> Result<ByteOffset, StackError> {
    let body_offset = frame
        .get()
        .checked_add(CALL_ENTRY_FIRST_ARGUMENT_OFFSET)
        .and_then(|base| base.checked_add(offset.get()))
        .ok_or(StackError::ArithmeticOverflow)?;
    access_displacement(ByteOffset::new(body_offset), width, ByteSize::ZERO)
}

/// A lower bound before allocation; include allocated spills in `frame` and
/// actual instruction temporaries in `transient` before reporting a final bound.
pub fn peak_below_entry(
    frame: ByteSize,
    outgoing: ByteSize,
    transfer: ByteSize,
    transient: ByteSize,
) -> Result<ByteSize, StackError> {
    let peak = frame
        .checked_add(outgoing)
        .and_then(|size| size.checked_add(transfer))
        .and_then(|size| size.checked_add(transient))
        .ok_or(StackError::ArithmeticOverflow)?;
    if peak.get() > u32::from(u16::MAX) {
        return Err(StackError::ArithmeticOverflow);
    }
    Ok(peak)
}

/// Host-side contract for a checked reservation; the emitter must implement
/// these unsigned bounds before changing S or pushing onto the domain stack.
pub fn reservation(
    current_s: u16,
    bytes: ByteSize,
    floor: u16,
    ceiling: u16,
) -> Result<u16, StackError> {
    let bytes = u16::try_from(bytes.get()).map_err(|_| StackError::ReservationOutOfBounds)?;
    if floor > ceiling || current_s > ceiling {
        return Err(StackError::ReservationOutOfBounds);
    }
    current_s
        .checked_sub(bytes)
        .filter(|&next| next >= floor)
        .ok_or(StackError::ReservationOutOfBounds)
}
