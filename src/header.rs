//! The 1024-byte revision store file header ([MS-ONESTORE] 2.3.1).
//!
//! Offsets below are fixed by the field order in the specification; the last
//! field (`rgbReserved`, 728 bytes) puts the header end exactly at 0x400.

use crate::guid::Guid;
use crate::reader::{u32_at, u64_at, ParseResult};

pub const HEADER_SIZE: usize = 1024;

/// `{7B5C52E4-D88C-4DA7-AEB1-5378D02996D3}` — section file.
pub const GUID_FILE_TYPE_ONE: Guid = Guid([
    0xE4, 0x52, 0x5C, 0x7B, 0x8C, 0xD8, 0xA7, 0x4D, 0xAE, 0xB1, 0x53, 0x78, 0xD0, 0x29, 0x96, 0xD3,
]);

/// `{43FF2FA1-EFD9-4C76-9EE2-10EA5722765F}` — table of contents file.
pub const GUID_FILE_TYPE_ONETOC2: Guid = Guid([
    0xA1, 0x2F, 0xFF, 0x43, 0xD9, 0xEF, 0x76, 0x4C, 0x9E, 0xE2, 0x10, 0xEA, 0x57, 0x22, 0x76, 0x5F,
]);

/// `{109ADD3F-911B-49F5-A5D0-1791EDC8AED8}` — OneNote 2010+ revision store.
pub const GUID_FILE_FORMAT_REVISION_STORE: Guid = Guid([
    0x3F, 0xDD, 0x9A, 0x10, 0x1B, 0x91, 0xF5, 0x49, 0xA5, 0xD0, 0x17, 0x91, 0xED, 0xC8, 0xAE, 0xD8,
]);

/// `{638DE92F-A6D4-4BC1-9A36-B3FC2511A5B7}` — package store ([MS-ONESTORE] 2.8.1): the same
/// revision store wrapped in [MS-FSSHTTPB] data elements. Not implemented.
pub const GUID_FILE_FORMAT_PACKAGE_STORE: Guid = Guid([
    0x2F, 0xE9, 0x8D, 0x63, 0xD4, 0xA6, 0xC1, 0x4B, 0x9A, 0x36, 0xB3, 0xFC, 0x25, 0x11, 0xA5, 0xB7,
]);

/// A FileChunkReference64x32: 64-bit file offset plus 32-bit byte count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkRef {
    pub stp: u64,
    pub cb: u32,
}

impl ChunkRef {
    pub fn read(data: &[u8], offset: usize) -> ParseResult<Self> {
        Ok(ChunkRef {
            stp: u64_at(data, offset)?,
            cb: u32_at(data, offset + 8)?,
        })
    }

    /// `fcrNil`: the reference does not exist.
    pub fn is_nil(&self) -> bool {
        self.stp == u64::MAX
    }

    /// `fcrZero`: the reference is explicitly empty.
    pub fn is_zero(&self) -> bool {
        self.stp == 0 && self.cb == 0
    }

    pub fn is_valid(&self) -> bool {
        !self.is_nil() && !self.is_zero()
    }
}

#[derive(Debug, Clone)]
pub struct Header {
    pub guid_file_type: Guid,
    pub guid_file: Guid,
    /// Non-nil only for the FSSHTTPB-packaged variant of a revision store.
    pub guid_legacy_file_version: Guid,
    pub guid_file_format: Guid,
    /// `ffvLastCodeThatWroteToThisFile`; 0x2A = OneNote 2010 format, 0x1B = onetoc2.
    pub ffv_last_code: u32,
    pub c_transactions_in_log: u32,
    pub guid_ancestor: Guid,
    pub crc_name: u32,
    pub fcr_hashed_chunk_list: ChunkRef,
    pub fcr_transaction_log: ChunkRef,
    pub fcr_file_node_list_root: ChunkRef,
    pub fcr_free_chunk_list: ChunkRef,
    pub cb_expected_file_length: u64,
    pub cb_free_space_in_free_chunk_list: u64,
    pub guid_file_version: Guid,
    pub n_file_version_generation: u64,
    pub bn_created: u32,
    pub bn_last_wrote: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerKind {
    /// `.one` section in the plain revision store layout ([MS-ONESTORE] 2.3.1).
    RevisionStore,
    /// `.onetoc2` table of contents.
    TableOfContents,
    /// A section wrapped in the [MS-FSSHTTPB] package store ([MS-ONESTORE] 2.8.1),
    /// which is how OneNote for Windows and OneDrive deliver modern sections.
    PackageStore,
    /// A revision store whose header says its payload is packaged.
    PackagedRevisionStore,
    Unknown,
}

impl Header {
    pub fn parse(data: &[u8]) -> ParseResult<Header> {
        if data.len() < HEADER_SIZE {
            return Err(format!(
                "file is {} bytes; a revision store header needs at least {HEADER_SIZE}",
                data.len()
            ));
        }
        Ok(Header {
            guid_file_type: Guid::read(data, 0x00)?,
            guid_file: Guid::read(data, 0x10)?,
            guid_legacy_file_version: Guid::read(data, 0x20)?,
            guid_file_format: Guid::read(data, 0x30)?,
            ffv_last_code: u32_at(data, 0x40)?,
            c_transactions_in_log: u32_at(data, 0x60)?,
            guid_ancestor: Guid::read(data, 0x80)?,
            crc_name: u32_at(data, 0x90)?,
            fcr_hashed_chunk_list: ChunkRef::read(data, 0x94)?,
            fcr_transaction_log: ChunkRef::read(data, 0xA0)?,
            fcr_file_node_list_root: ChunkRef::read(data, 0xAC)?,
            fcr_free_chunk_list: ChunkRef::read(data, 0xB8)?,
            cb_expected_file_length: u64_at(data, 0xC4)?,
            cb_free_space_in_free_chunk_list: u64_at(data, 0xCC)?,
            guid_file_version: Guid::read(data, 0xD4)?,
            n_file_version_generation: u64_at(data, 0xE4)?,
            bn_created: u32_at(data, 0x118)?,
            bn_last_wrote: u32_at(data, 0x11C)?,
        })
    }

    pub fn kind(&self) -> ContainerKind {
        if self.guid_file_type == GUID_FILE_TYPE_ONETOC2 {
            return ContainerKind::TableOfContents;
        }
        if self.guid_file_format == GUID_FILE_FORMAT_PACKAGE_STORE {
            return ContainerKind::PackageStore;
        }
        if self.guid_file_type == GUID_FILE_TYPE_ONE
            && self.guid_file_format == GUID_FILE_FORMAT_REVISION_STORE
        {
            // A non-nil legacy version marks the packaged variant of the same
            // format, not a plain desktop revision store.
            if !self.guid_legacy_file_version.is_zero() {
                return ContainerKind::PackagedRevisionStore;
            }
            return ContainerKind::RevisionStore;
        }
        ContainerKind::Unknown
    }

    /// Human-readable OneNote generation implied by `ffvLastCodeThatWroteToThisFile`.
    pub fn writer_generation(&self) -> &'static str {
        match self.ffv_last_code {
            0x1B => "OneNote 2007 (onetoc2 layout)",
            0x1C => "OneNote 2010",
            0x22 => "OneNote 2013",
            0x2A => "OneNote 2010 or later",
            _ => "unknown",
        }
    }

    pub fn file_length_matches(&self, actual: usize) -> bool {
        self.cb_expected_file_length == actual as u64
    }

    /// Field-by-field size of the header; documents that it ends at 0x400.
    #[cfg(test)]
    pub fn declared_size() -> usize {
        let mut total = 0usize;
        // guidFileType, guidFile, guidLegacyFileVersion, guidFileFormat
        total += 4 * 16;
        total += 4 * 4; // ffv*
        total += 8 + 8 + 4 + 4 + 8 + 8 + 4; // legacy chunk refs and counters
        total += 4; // four 1-byte flags
        total += 16 + 4; // guidAncestor, crcName
        total += 4 * 12; // hashed, transaction log, root, free chunk lists
        total += 8 + 8; // cbExpectedFileLength, cbFreeSpaceInFreeChunkList
        total += 16 + 8 + 16; // guidFileVersion, nFileVersionGeneration, guidDenyReadFileVersion
        total += 4 + 12 + 12; // grfDebugLogFlags, fcrDebugLog, fcrAllocVerificationFreeChunkList
        total += 4 * 4; // bnCreated .. bnNewestWritten
        total += 728; // rgbReserved
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_layout_totals_1024() {
        assert_eq!(Header::declared_size(), HEADER_SIZE);
        // Last field starts at 0x128 and fills the rest of the header.
        assert_eq!(HEADER_SIZE - 0x128, 728);
    }

    #[test]
    fn known_guid_bytes_decode_to_spec_strings() {
        assert_eq!(
            GUID_FILE_TYPE_ONE.to_string(),
            "{7B5C52E4-D88C-4DA7-AEB1-5378D02996D3}"
        );
        assert_eq!(
            GUID_FILE_FORMAT_REVISION_STORE.to_string(),
            "{109ADD3F-911B-49F5-A5D0-1791EDC8AED8}"
        );
        assert_eq!(
            GUID_FILE_FORMAT_PACKAGE_STORE.to_string(),
            "{638DE92F-A6D4-4BC1-9A36-B3FC2511A5B7}"
        );
    }

    #[test]
    fn chunk_reference_sentinels() {
        assert!(ChunkRef {
            stp: u64::MAX,
            cb: 0
        }
        .is_nil());
        assert!(ChunkRef { stp: 0, cb: 0 }.is_zero());
        assert!(ChunkRef {
            stp: 0x400,
            cb: 0x400
        }
        .is_valid());
    }
}
