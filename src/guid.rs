//! GUID and ExtendedGUID types as laid out by [MS-DTYP] and [MS-ONESTORE] 2.2.1.

use crate::reader::{slice_at, ParseResult};
use std::fmt;

/// A 16-byte GUID stored in the mixed-endian layout used on disk
/// (Data1/Data2/Data3 little-endian, Data4 big-endian).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Guid(pub [u8; 16]);

/// Identity of a revision store object space: a GUID plus a generation count.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ExtendedGuid {
    pub guid: Guid,
    pub n: u32,
}

impl Guid {
    pub fn read(data: &[u8], offset: usize) -> ParseResult<Self> {
        let b = slice_at(data, offset, 16)?;
        let mut raw = [0u8; 16];
        raw.copy_from_slice(b);
        Ok(Guid(raw))
    }

    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|&b| b == 0)
    }

    /// Parse the canonical `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}` form.
    pub fn parse(text: &str) -> Option<Guid> {
        let trimmed = text.trim().trim_start_matches('{').trim_end_matches('}');
        let hex: Vec<&str> = trimmed.split('-').collect();
        if hex.len() != 5 {
            return None;
        }
        let (d4, d5) = (hex[3], hex[4]);
        if hex[0].len() != 8
            || hex[1].len() != 4
            || hex[2].len() != 4
            || d4.len() != 4
            || d5.len() != 12
        {
            return None;
        }
        let mut bytes = [0u8; 16];
        bytes[0..4].copy_from_slice(&u32::from_str_radix(hex[0], 16).ok()?.to_le_bytes());
        bytes[4..6].copy_from_slice(&u16::from_str_radix(hex[1], 16).ok()?.to_le_bytes());
        bytes[6..8].copy_from_slice(&u16::from_str_radix(hex[2], 16).ok()?.to_le_bytes());
        bytes[8..10].copy_from_slice(&u16::from_str_radix(d4, 16).ok()?.to_be_bytes());
        for i in 0..6 {
            bytes[10 + i] = u8::from_str_radix(&d5[i * 2..i * 2 + 2], 16).ok()?;
        }
        Some(Guid(bytes))
    }
}

impl ExtendedGuid {
    pub fn read(data: &[u8], offset: usize) -> ParseResult<Self> {
        let guid = Guid::read(data, offset)?;
        let n = crate::reader::u32_at(data, offset + 16)?;
        Ok(ExtendedGuid { guid, n })
    }
}

impl fmt::Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = &self.0;
        write!(
            f,
            "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
            u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            u16::from_le_bytes([b[4], b[5]]),
            u16::from_le_bytes([b[6], b[7]]),
            b[8],
            b[9],
            b[10],
            b[11],
            b[12],
            b[13],
            b[14],
            b[15]
        )
    }
}

impl fmt::Debug for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for ExtendedGuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.guid, self.n)
    }
}

impl fmt::Debug for ExtendedGuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
