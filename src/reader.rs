//! Bounds-checked little-endian primitives over a byte slice.
//!
//! Every read in the parser goes through these helpers so that a corrupt or
//! truncated file produces an `Err` instead of a panic.

pub type ParseResult<T> = Result<T, String>;

pub fn slice_at(data: &[u8], offset: usize, len: usize) -> ParseResult<&[u8]> {
    let end = offset
        .checked_add(len)
        .ok_or_else(|| format!("offset {offset:#x} + {len} overflows"))?;
    data.get(offset..end).ok_or_else(|| {
        format!(
            "need {len} bytes at {offset:#x}, file is {:#x} bytes",
            data.len()
        )
    })
}

pub fn u8_at(data: &[u8], offset: usize) -> ParseResult<u8> {
    Ok(slice_at(data, offset, 1)?[0])
}

pub fn u16_at(data: &[u8], offset: usize) -> ParseResult<u16> {
    let b = slice_at(data, offset, 2)?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

pub fn u32_at(data: &[u8], offset: usize) -> ParseResult<u32> {
    let b = slice_at(data, offset, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

pub fn u64_at(data: &[u8], offset: usize) -> ParseResult<u64> {
    let b = slice_at(data, offset, 8)?;
    Ok(u64::from_le_bytes([
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
    ]))
}
