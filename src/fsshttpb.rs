//! Reader for the [MS-FSSHTTPB] package store that wraps a revision store.
//!
//! OneNote for Windows and OneDrive deliver sections as a package store
//! ([MS-ONESTORE] 2.8.1): four GUIDs, a zero word, a `OneNotePackaging` stream
//! object header, then a `DataElementPackage` whose data elements describe the
//! same object model the plain revision store stores directly.
//!
//! This module parses the wrapper into data elements; `packaging` turns those
//! elements into the object model the renderer consumes.
//!
//! # Format notes
//!
//! Everything below was decoded from the published specifications and checked
//! against the sample corpus, so this module can be maintained without going
//! back to either source.
//!
//! Packaging header ([MS-ONESTORE] 2.8.1), in order:
//! `guidFileType(16) guidFile(16) guidLegacyFileVersion(16) guidFileFormat(16)
//! zero(4) ObjectHeader(OneNotePackaging 0x7A) storageIndex(ExGuid)
//! cellSchema(16) DataElementPackage`.
//!
//! Stream object headers ([MS-FSSHTTPB] 2.2.1.5), selected by the low two bits
//! of the first byte:
//!
//! | bits 0-1 | form | layout |
//! | --- | --- | --- |
//! | `00` | 16-bit start | bit 2 compound, bits 3-8 type, bits 9-15 length |
//! | `10` | 32-bit start | bit 2 compound, bits 3-16 type, bits 17-31 length (`0x7FFF` → CompactU64 follows) |
//! | `01` | 8-bit end | bits 2-7 type |
//! | `11` | 16-bit end | bits 2-15 type |
//!
//! `CompactU64` ([MS-FSSHTTPB] 2.2.1.4) takes its width from the first byte's
//! lowest set bit: `0` → zero (1 byte); `1` → 1 byte (`>>1`); `2` → 2 bytes
//! (`>>2`); `4` → 3 bytes (`>>3`); `8` → 4 (`>>4`); `16` → 5 (`>>5`);
//! `32` → 6 (`>>6`); `64` → 7 (`>>7`); `128` → 1 byte then a full `u64`.
//!
//! `ExGuid` ([MS-FSSHTTPB] 2.2.1.7): `0` → nil; low three bits `100` → 5-bit
//! value (`>>3`) then GUID; low six bits `100000` → 10-bit value (next byte
//! `<<2` plus `>>6`) then GUID; low seven bits `1000000` → 17-bit value (u16
//! `<<1` plus `>>7`) then GUID; `0x80` → u32 value then GUID. Arrays are a
//! `CompactU64` count followed by that many entries. `SerialNumber` is a u8
//! tag (0 → nil) then GUID plus u64. `CellId` is two `ExGuid`s.
//!
//! Data element package ([MS-FSSHTTPB] 2.2.1.12): header `DataElementPackage`,
//! one zero byte, then `DataElement`s until an end header, each being header
//! `DataElement`, `ExGuid` id, `SerialNumber`, `CompactU64` element type, then
//! the body:
//!
//! | type | body |
//! | --- | --- |
//! | `0x01` storage index | mapping entries until end (no extra header) |
//! | `0x02` storage manifest | header `StorageManifest`, GUID, then `StorageManifestRoot` roots |
//! | `0x03` cell manifest | header `CellManifest`, `ExGuid`, end |
//! | `0x04` revision manifest | header `RevisionManifest`, rev + base `ExGuid`, then root/group-reference entries |
//! | `0x05` object group | header `ObjectGroupDeclaration`, declarations, end, optional metadata block, header `ObjectGroupData`, objects, two ends |
//! | `0x06` fragment | header `DataElementFragment`, id, size, offset, length, bytes |
//! | `0x0A` object data blob | header `ObjectDataBlob`, `CompactU64` size, bytes |
//!
//! Object group data entries are a `CompactU64`-counted array of `ExGuid`
//! references, a `CompactU64`-counted array of `CellId`s, then the payload that
//! the declaration's `data_size` announces.
//!
//! # Provenance
//!
//! Structures above come from [MS-FSSHTTPB]/[MS-ONESTORE]. Behaviour was
//! cross-checked against `onenote-parser` (msiemens, MPL-2.0) as an independent
//! implementation. This code is an independent implementation of the
//! specifications, not a port of that project.

use crate::guid::Guid;
use crate::reader::{u16_at, u32_at, u64_at, ParseResult};

/// Stream object types we need ([MS-FSSHTTPB] 2.2.1.5.1/2).
pub mod object_type {
    pub const DATA_ELEMENT: u16 = 0x01;
    pub const OBJECT_DATA_BLOB: u16 = 0x02;
    pub const OBJECT_GROUP_DATA_EXCLUDED: u16 = 0x03;
    pub const OBJECT_GROUP_DATA_BLOB: u16 = 0x05;
    pub const STORAGE_MANIFEST_ROOT: u16 = 0x07;
    pub const REVISION_MANIFEST_ROOT: u16 = 0x0A;
    pub const CELL_MANIFEST: u16 = 0x0B;
    pub const STORAGE_MANIFEST: u16 = 0x0C;
    pub const STORAGE_INDEX_REVISION_MAPPING: u16 = 0x0D;
    pub const STORAGE_INDEX_CELL_MAPPING: u16 = 0x0E;
    pub const STORAGE_INDEX_MANIFEST_MAPPING: u16 = 0x11;
    pub const DATA_ELEMENT_PACKAGE: u16 = 0x15;
    pub const OBJECT_GROUP_DATA_OBJECT: u16 = 0x16;
    pub const OBJECT_GROUP_OBJECT: u16 = 0x18;
    pub const REVISION_MANIFEST_GROUP_REFERENCE: u16 = 0x19;
    pub const REVISION_MANIFEST: u16 = 0x1A;
    pub const OBJECT_GROUP_BLOB_REFERENCE: u16 = 0x1C;
    pub const OBJECT_GROUP_DECLARATION: u16 = 0x1D;
    pub const OBJECT_GROUP_DATA: u16 = 0x1E;
    pub const DATA_ELEMENT_FRAGMENT: u16 = 0x06A;
    pub const OBJECT_GROUP_METADATA: u16 = 0x078;
    pub const OBJECT_GROUP_METADATA_BLOCK: u16 = 0x079;
    pub const ONENOTE_PACKAGING: u16 = 0x7A;
}

/// Cursor over a byte slice with the primitive reads FSSHTTPB needs.
pub struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Cursor { data, pos: 0 }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn need(&self, len: usize) -> ParseResult<()> {
        if self.remaining() < len {
            return Err(format!(
                "need {len} bytes at {:#x}, structure is {:#x} bytes",
                self.pos,
                self.data.len()
            ));
        }
        Ok(())
    }

    pub fn u8(&mut self) -> ParseResult<u8> {
        let value = crate::reader::u8_at(self.data, self.pos)?;
        self.pos += 1;
        Ok(value)
    }

    pub fn u16(&mut self) -> ParseResult<u16> {
        let value = u16_at(self.data, self.pos)?;
        self.pos += 2;
        Ok(value)
    }

    pub fn u32(&mut self) -> ParseResult<u32> {
        let value = u32_at(self.data, self.pos)?;
        self.pos += 4;
        Ok(value)
    }

    pub fn u64(&mut self) -> ParseResult<u64> {
        let value = u64_at(self.data, self.pos)?;
        self.pos += 8;
        Ok(value)
    }

    pub fn peek_u8(&self) -> ParseResult<u8> {
        crate::reader::u8_at(self.data, self.pos)
    }

    pub fn bytes(&mut self, len: usize) -> ParseResult<&'a [u8]> {
        self.need(len)?;
        let slice = crate::reader::slice_at(self.data, self.pos, len)?;
        self.pos += len;
        Ok(slice)
    }

    pub fn skip(&mut self, len: usize) -> ParseResult<()> {
        self.need(len)?;
        self.pos += len;
        Ok(())
    }

    pub fn guid(&mut self) -> ParseResult<Guid> {
        let guid = Guid::read(self.data, self.pos)?;
        self.pos += 16;
        Ok(guid)
    }

    /// `CompactU64` ([MS-FSSHTTPB] 2.2.1.4): the number of leading zero bits of
    /// the first byte selects the width.
    pub fn compact_u64(&mut self) -> ParseResult<u64> {
        let first = self.peek_u8()?;
        if first == 0 {
            self.pos += 1;
            return Ok(0);
        }
        if first & 0b1 != 0 {
            return Ok((self.u8()? >> 1) as u64);
        }
        if first & 0b10 != 0 {
            return Ok((self.u16()? >> 2) as u64);
        }
        if first & 0b100 != 0 {
            self.need(3)?;
            let b = self.bytes(3)?;
            let value = u32::from_le_bytes([b[0], b[1], b[2], 0]);
            return Ok((value >> 3) as u64);
        }
        if first & 0b1000 != 0 {
            let value = self.u32()?;
            return Ok((value >> 4) as u64);
        }
        if first & 0b1_0000 != 0 {
            self.need(5)?;
            let b = self.bytes(5)?;
            let value = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], 0, 0, 0]);
            return Ok(value >> 5);
        }
        if first & 0b10_0000 != 0 {
            self.need(6)?;
            let b = self.bytes(6)?;
            let value = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], 0, 0]);
            return Ok(value >> 6);
        }
        if first & 0b100_0000 != 0 {
            self.need(7)?;
            let b = self.bytes(7)?;
            let value = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], 0]);
            return Ok(value >> 7);
        }
        if first & 0b1000_0000 != 0 {
            self.pos += 1;
            return self.u64();
        }
        Err(format!("invalid CompactU64 first byte {first:#04x}"))
    }
}

/// An extended GUID: 16-byte GUID plus a value whose width varies
/// ([MS-FSSHTTPB] 2.2.1.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ExGuid {
    pub guid: Guid,
    pub value: u32,
}

impl ExGuid {
    pub fn parse(cursor: &mut Cursor<'_>) -> ParseResult<ExGuid> {
        let first = cursor.u8()?;
        if first == 0 {
            return Ok(ExGuid::default());
        }
        if first & 0b111 == 0b100 {
            let value = (first >> 3) as u32;
            return Ok(ExGuid {
                guid: cursor.guid()?,
                value,
            });
        }
        if first & 0b11_1111 == 0b10_0000 {
            let low = (cursor.u8()? as u16) << 2 | (first >> 6) as u16;
            return Ok(ExGuid {
                guid: cursor.guid()?,
                value: low as u32,
            });
        }
        if first & 0b111_1111 == 0b100_0000 {
            let value = (cursor.u16()? as u32) << 1 | (first >> 7) as u32;
            return Ok(ExGuid {
                guid: cursor.guid()?,
                value,
            });
        }
        if first == 0b1000_0000 {
            let value = cursor.u32()?;
            return Ok(ExGuid {
                guid: cursor.guid()?,
                value,
            });
        }
        Err(format!(
            "invalid ExGuid first byte {first:#010b} at {:#x}",
            cursor.position() - 1
        ))
    }

    pub fn parse_array(cursor: &mut Cursor<'_>) -> ParseResult<Vec<ExGuid>> {
        let count = cursor.compact_u64()? as usize;
        let mut out = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            out.push(ExGuid::parse(cursor)?);
        }
        Ok(out)
    }

    pub fn is_nil(&self) -> bool {
        self.guid.is_zero() && self.value == 0
    }
}

/// A serial number ([MS-FSSHTTPB] 2.2.1.9): a GUID plus a 64-bit counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SerialNumber {
    pub guid: Guid,
    pub serial: u64,
}

impl SerialNumber {
    pub fn parse(cursor: &mut Cursor<'_>) -> ParseResult<SerialNumber> {
        if cursor.u8()? == 0 {
            return Ok(SerialNumber::default());
        }
        let guid = cursor.guid()?;
        let serial = cursor.u64()?;
        Ok(SerialNumber { guid, serial })
    }
}

/// A cell identity: two extended GUIDs ([MS-FSSHTTPB] 2.2.1.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellId(pub ExGuid, pub ExGuid);

impl CellId {
    pub fn parse(cursor: &mut Cursor<'_>) -> ParseResult<CellId> {
        Ok(CellId(ExGuid::parse(cursor)?, ExGuid::parse(cursor)?))
    }

    pub fn parse_array(cursor: &mut Cursor<'_>) -> ParseResult<Vec<CellId>> {
        let count = cursor.compact_u64()? as usize;
        let mut out = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            out.push(CellId::parse(cursor)?);
        }
        Ok(out)
    }
}

/// A stream object header ([MS-FSSHTTPB] 2.2.1.5).
#[derive(Debug, Clone, Copy)]
pub struct ObjectHeader {
    pub compound: bool,
    pub object_type: u16,
    pub length: u64,
}

impl ObjectHeader {
    /// 16-bit header: bits 0-1 = 0b00, bit 2 = compound, bits 3-8 = type.
    pub fn parse_16(cursor: &mut Cursor<'_>) -> ParseResult<ObjectHeader> {
        let word = cursor.u16()?;
        if word & 0b11 != 0b00 {
            return Err(format!("not a 16-bit object header: {word:#06x}"));
        }
        Ok(ObjectHeader {
            compound: word & 0b100 != 0,
            object_type: (word >> 3) & 0x3F,
            length: (word >> 9) as u64,
        })
    }

    /// 32-bit header: bits 0-1 = 0b10, bit 2 = compound, bits 3-16 = type.
    pub fn parse_32(cursor: &mut Cursor<'_>) -> ParseResult<ObjectHeader> {
        let word = cursor.u32()?;
        if word & 0b11 != 0b10 {
            return Err(format!("not a 32-bit object header: {word:#010x}"));
        }
        let mut length = (word >> 17) as u64;
        if length == 0x7FFF {
            length = cursor.compact_u64()?;
        }
        Ok(ObjectHeader {
            compound: word & 0b100 != 0,
            object_type: ((word >> 3) & 0x3FFF) as u16,
            length,
        })
    }

    /// Either width, chosen by the low two bits of the first byte.
    pub fn parse(cursor: &mut Cursor<'_>) -> ParseResult<ObjectHeader> {
        match cursor.peek_u8()? & 0b11 {
            0b00 => ObjectHeader::parse_16(cursor),
            0b10 => ObjectHeader::parse_32(cursor),
            other => Err(format!("unexpected object header type {other:#04b}")),
        }
    }

    pub fn expect(cursor: &mut Cursor<'_>, object_type: u16) -> ParseResult<ObjectHeader> {
        let header = ObjectHeader::parse(cursor)?;
        if header.object_type != object_type {
            return Err(format!(
                "expected object type {object_type:#x}, found {:#x}",
                header.object_type
            ));
        }
        Ok(header)
    }

    /// 8-bit end header: bits 0-1 = 0b01, bits 2-7 = type.
    pub fn end_8(cursor: &mut Cursor<'_>) -> ParseResult<u16> {
        let byte = cursor.u8()?;
        if byte & 0b11 != 0b01 {
            return Err(format!("not an 8-bit end header: {byte:#04x}"));
        }
        Ok((byte >> 2) as u16)
    }

    /// 16-bit end header: bits 0-1 = 0b11, bits 2-15 = type.
    pub fn end_16(cursor: &mut Cursor<'_>) -> ParseResult<u16> {
        let word = cursor.u16()?;
        if word & 0b11 != 0b11 {
            return Err(format!("not a 16-bit end header: {word:#06x}"));
        }
        Ok(word >> 2)
    }

    /// True when the next bytes close `object_type`. Accepts both end widths,
    /// because object types above 0x3F only fit the 16-bit form.
    pub fn has_end(cursor: &Cursor<'_>, object_type: u16) -> ParseResult<bool> {
        let byte = cursor.peek_u8()?;
        if byte & 0b11 == 0b01 {
            return Ok((byte >> 2) as u16 == object_type && object_type <= 0x3F);
        }
        if byte & 0b11 == 0b11 && cursor.remaining() >= 2 {
            let word = u16_at(cursor.data, cursor.pos)?;
            return Ok(word >> 2 == object_type);
        }
        Ok(false)
    }
}

#[derive(Debug, Clone, Default)]
pub struct StorageIndex {
    pub manifest_mappings: Vec<(ExGuid, SerialNumber)>,
    pub cell_mappings: Vec<(CellId, ExGuid, SerialNumber)>,
    pub revision_mappings: Vec<(ExGuid, ExGuid, SerialNumber)>,
}

#[derive(Debug, Clone, Default)]
pub struct StorageManifest {
    pub id: Guid,
    pub roots: Vec<(ExGuid, CellId)>,
}

#[derive(Debug, Clone, Default)]
pub struct RevisionManifest {
    pub id: ExGuid,
    pub base_id: ExGuid,
    pub roots: Vec<(ExGuid, ExGuid)>,
    pub groups: Vec<ExGuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declaration {
    Object {
        object_id: ExGuid,
        partition_id: u64,
        data_size: u64,
        object_refs: u64,
        cell_refs: u64,
    },
    Blob {
        object_id: ExGuid,
        blob_id: ExGuid,
        partition_id: u64,
        object_refs: u64,
        cell_refs: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupObject {
    Data {
        objects: Vec<ExGuid>,
        cells: Vec<CellId>,
        data: Vec<u8>,
    },
    Excluded {
        objects: Vec<ExGuid>,
        cells: Vec<CellId>,
        size: u64,
    },
    BlobReference {
        objects: Vec<ExGuid>,
        cells: Vec<CellId>,
        blob: ExGuid,
    },
}

#[derive(Debug, Clone, Default)]
pub struct ObjectGroup {
    pub declarations: Vec<Declaration>,
    pub objects: Vec<GroupObject>,
}

/// A parsed package store: the data elements of one file.
#[derive(Debug, Clone, Default)]
pub struct Package {
    pub storage_index: Option<StorageIndex>,
    pub storage_manifests: Vec<StorageManifest>,
    /// (data element id, revision id it points at)
    pub cell_manifests: Vec<(ExGuid, ExGuid)>,
    pub revision_manifests: Vec<RevisionManifest>,
    pub object_groups: Vec<(ExGuid, ObjectGroup)>,
    pub blobs: Vec<(ExGuid, Vec<u8>)>,
    pub fragments: Vec<(ExGuid, u64, Vec<u8>)>,
    pub data_element_count: usize,
}

impl Package {
    /// Total declared objects across every object group.
    pub fn declaration_count(&self) -> usize {
        self.object_groups
            .iter()
            .map(|(_, group)| group.declarations.len())
            .sum()
    }

    pub fn object_bytes(&self) -> usize {
        self.object_groups
            .iter()
            .flat_map(|(_, group)| group.objects.iter())
            .map(|object| match object {
                GroupObject::Data { data, .. } => data.len(),
                _ => 0,
            })
            .sum()
    }

    pub fn find_group(&self, id: ExGuid) -> Option<&ObjectGroup> {
        self.object_groups
            .iter()
            .find(|(group_id, _)| *group_id == id)
            .map(|(_, group)| group)
    }
}

/// Parse a package store file: packaging header plus its data element package.
pub fn parse(data: &[u8]) -> ParseResult<Package> {
    let mut cursor = Cursor::new(data);
    let _file_type = cursor.guid()?;
    let _file = cursor.guid()?;
    let _legacy_version = cursor.guid()?;
    let format = cursor.guid()?;
    if format != crate::header::GUID_FILE_FORMAT_PACKAGE_STORE {
        return Err(format!("not a package store: file format {format}"));
    }
    let reserved = cursor.u32()?;
    if reserved != 0 {
        return Err(format!("packaging padding is {reserved:#x}, expected zero"));
    }
    ObjectHeader::expect(&mut cursor, object_type::ONENOTE_PACKAGING)?;
    let _storage_index_id = ExGuid::parse(&mut cursor)?;
    let _cell_schema = cursor.guid()?;
    parse_data_element_package(&mut cursor)
}

fn parse_data_element_package(cursor: &mut Cursor<'_>) -> ParseResult<Package> {
    ObjectHeader::expect(cursor, object_type::DATA_ELEMENT_PACKAGE)?;
    let padding = cursor.u8()?;
    if padding != 0 {
        return Err(format!("data element package padding is {padding:#x}"));
    }

    let mut package = Package::default();
    while !ObjectHeader::has_end(cursor, object_type::DATA_ELEMENT_PACKAGE)? {
        parse_data_element(cursor, &mut package)?;
    }
    let closed = ObjectHeader::end_8(cursor).or_else(|_| ObjectHeader::end_16(cursor))?;
    if closed != object_type::DATA_ELEMENT_PACKAGE {
        return Err(format!("data element package closed by type {closed:#x}"));
    }
    Ok(package)
}

fn trace(cursor: &Cursor<'_>, message: &str) {
    if std::env::var_os("ONENOTE_TRACE").is_some() {
        eprintln!("[{:#06x}] {message}", cursor.position());
    }
}

fn parse_data_element(cursor: &mut Cursor<'_>, package: &mut Package) -> ParseResult<()> {
    let element_start = cursor.position();
    ObjectHeader::expect(cursor, object_type::DATA_ELEMENT)?;
    let id = ExGuid::parse(cursor)?;
    let _serial = SerialNumber::parse(cursor)?;
    let element_type = cursor.compact_u64()?;
    package.data_element_count += 1;
    trace(
        cursor,
        &format!("element {element_start:#06x} type {element_type:#x} body starts"),
    );

    match element_type {
        0x01 => {
            let index = parse_storage_index(cursor)?;
            package.storage_index.get_or_insert(index);
        }
        0x02 => package
            .storage_manifests
            .push(parse_storage_manifest(cursor)?),
        0x03 => {
            // CellManifest: object header, the cell's ExGuid, then the end header.
            ObjectHeader::expect(cursor, object_type::CELL_MANIFEST)?;
            let cell = ExGuid::parse(cursor)?;
            ObjectHeader::close_data_element(cursor)?;
            package.cell_manifests.push((id, cell));
        }
        0x04 => package
            .revision_manifests
            .push(parse_revision_manifest(cursor)?),
        0x05 => package
            .object_groups
            .push((id, parse_object_group(cursor)?)),
        0x06 => package.fragments.push(parse_fragment(cursor)?),
        0x0A => package.blobs.push((id, parse_blob(cursor)?)),
        other => return Err(format!("invalid data element type {other:#x}")),
    }
    trace(
        cursor,
        &format!("element type {element_type:#x} {element_start:#06x}.."),
    );
    Ok(())
}

impl ObjectHeader {
    /// Close a data element: 8- or 16-bit end header of type `DataElement`.
    fn close_data_element(cursor: &mut Cursor<'_>) -> ParseResult<()> {
        let closed = ObjectHeader::end_8(cursor).or_else(|_| ObjectHeader::end_16(cursor))?;
        if closed != object_type::DATA_ELEMENT {
            return Err(format!("data element closed by type {closed:#x}"));
        }
        Ok(())
    }
}

fn parse_storage_index(cursor: &mut Cursor<'_>) -> ParseResult<StorageIndex> {
    let mut index = StorageIndex::default();
    loop {
        if ObjectHeader::has_end(cursor, object_type::DATA_ELEMENT)? {
            break;
        }
        let header = ObjectHeader::parse(cursor)?;
        match header.object_type {
            object_type::STORAGE_INDEX_MANIFEST_MAPPING => {
                let mapping = ExGuid::parse(cursor)?;
                index
                    .manifest_mappings
                    .push((mapping, SerialNumber::parse(cursor)?));
            }
            object_type::STORAGE_INDEX_CELL_MAPPING => {
                let cell = CellId::parse(cursor)?;
                let id = ExGuid::parse(cursor)?;
                index
                    .cell_mappings
                    .push((cell, id, SerialNumber::parse(cursor)?));
            }
            object_type::STORAGE_INDEX_REVISION_MAPPING => {
                let id = ExGuid::parse(cursor)?;
                let revision = ExGuid::parse(cursor)?;
                index
                    .revision_mappings
                    .push((id, revision, SerialNumber::parse(cursor)?));
            }
            other => return Err(format!("unexpected storage index object {other:#x}")),
        }
    }
    ObjectHeader::close_data_element(cursor)?;
    Ok(index)
}

fn parse_storage_manifest(cursor: &mut Cursor<'_>) -> ParseResult<StorageManifest> {
    ObjectHeader::expect(cursor, object_type::STORAGE_MANIFEST)?;
    let id = cursor.guid()?;
    let mut manifest = StorageManifest {
        id,
        roots: Vec::new(),
    };
    loop {
        if ObjectHeader::has_end(cursor, object_type::DATA_ELEMENT)? {
            break;
        }
        ObjectHeader::expect(cursor, object_type::STORAGE_MANIFEST_ROOT)?;
        let root = ExGuid::parse(cursor)?;
        let cell = CellId::parse(cursor)?;
        manifest.roots.push((root, cell));
    }
    ObjectHeader::close_data_element(cursor)?;
    Ok(manifest)
}

fn parse_revision_manifest(cursor: &mut Cursor<'_>) -> ParseResult<RevisionManifest> {
    ObjectHeader::expect(cursor, object_type::REVISION_MANIFEST)?;
    let id = ExGuid::parse(cursor)?;
    let base_id = ExGuid::parse(cursor)?;
    let mut manifest = RevisionManifest {
        id,
        base_id,
        roots: Vec::new(),
        groups: Vec::new(),
    };
    loop {
        if ObjectHeader::has_end(cursor, object_type::DATA_ELEMENT)? {
            break;
        }
        let header = ObjectHeader::parse(cursor)?;
        match header.object_type {
            object_type::REVISION_MANIFEST_ROOT => {
                let root_id = ExGuid::parse(cursor)?;
                let object_id = ExGuid::parse(cursor)?;
                manifest.roots.push((root_id, object_id));
            }
            object_type::REVISION_MANIFEST_GROUP_REFERENCE => {
                manifest.groups.push(ExGuid::parse(cursor)?);
            }
            other => return Err(format!("unexpected revision manifest object {other:#x}")),
        }
    }
    ObjectHeader::close_data_element(cursor)?;
    Ok(manifest)
}

fn parse_object_group(cursor: &mut Cursor<'_>) -> ParseResult<ObjectGroup> {
    let mut group = ObjectGroup::default();

    ObjectHeader::expect(cursor, object_type::OBJECT_GROUP_DECLARATION)?;
    while !ObjectHeader::has_end(cursor, object_type::OBJECT_GROUP_DECLARATION)? {
        let header = ObjectHeader::parse(cursor)?;
        match header.object_type {
            object_type::OBJECT_GROUP_OBJECT => {
                let object_id = ExGuid::parse(cursor)?;
                let partition_id = cursor.compact_u64()?;
                let data_size = cursor.compact_u64()?;
                let object_refs = cursor.compact_u64()?;
                let cell_refs = cursor.compact_u64()?;
                group.declarations.push(Declaration::Object {
                    object_id,
                    partition_id,
                    data_size,
                    object_refs,
                    cell_refs,
                });
            }
            object_type::OBJECT_GROUP_DATA_BLOB => {
                let object_id = ExGuid::parse(cursor)?;
                let blob_id = ExGuid::parse(cursor)?;
                let partition_id = cursor.compact_u64()?;
                let object_refs = cursor.compact_u64()?;
                let cell_refs = cursor.compact_u64()?;
                group.declarations.push(Declaration::Blob {
                    object_id,
                    blob_id,
                    partition_id,
                    object_refs,
                    cell_refs,
                });
            }
            other => return Err(format!("unexpected object group declaration {other:#x}")),
        }
    }
    let closed = ObjectHeader::end_8(cursor).or_else(|_| ObjectHeader::end_16(cursor))?;
    if closed != object_type::OBJECT_GROUP_DECLARATION {
        return Err(format!("object group declarations closed by {closed:#x}"));
    }

    // A metadata block is optional; the group object data always follows it.
    let header = ObjectHeader::parse(cursor)?;
    if header.object_type == object_type::OBJECT_GROUP_METADATA_BLOCK {
        while !ObjectHeader::has_end(cursor, object_type::OBJECT_GROUP_METADATA_BLOCK)? {
            ObjectHeader::expect(cursor, object_type::OBJECT_GROUP_METADATA)?;
            let _frequency = cursor.compact_u64()?;
        }
        let closed = ObjectHeader::end_8(cursor).or_else(|_| ObjectHeader::end_16(cursor))?;
        if closed != object_type::OBJECT_GROUP_METADATA_BLOCK {
            return Err(format!("metadata block closed by {closed:#x}"));
        }
        ObjectHeader::expect(cursor, object_type::OBJECT_GROUP_DATA)?;
    } else if header.object_type != object_type::OBJECT_GROUP_DATA {
        return Err(format!(
            "expected object group data, found {:#x}",
            header.object_type
        ));
    }

    while !ObjectHeader::has_end(cursor, object_type::OBJECT_GROUP_DATA)? {
        let header = ObjectHeader::parse(cursor)?;
        match header.object_type {
            object_type::OBJECT_GROUP_DATA_OBJECT => {
                let objects = ExGuid::parse_array(cursor)?;
                let cells = CellId::parse_array(cursor)?;
                let size = cursor.compact_u64()? as usize;
                let data = cursor.bytes(size)?.to_vec();
                group.objects.push(GroupObject::Data {
                    objects,
                    cells,
                    data,
                });
            }
            object_type::OBJECT_GROUP_DATA_EXCLUDED => {
                let objects = ExGuid::parse_array(cursor)?;
                let cells = CellId::parse_array(cursor)?;
                let size = cursor.compact_u64()?;
                group.objects.push(GroupObject::Excluded {
                    objects,
                    cells,
                    size,
                });
            }
            object_type::OBJECT_GROUP_BLOB_REFERENCE => {
                let objects = ExGuid::parse_array(cursor)?;
                let cells = CellId::parse_array(cursor)?;
                let blob = ExGuid::parse(cursor)?;
                group.objects.push(GroupObject::BlobReference {
                    objects,
                    cells,
                    blob,
                });
            }
            other => return Err(format!("unexpected object group data {other:#x}")),
        }
    }
    let closed = ObjectHeader::end_8(cursor).or_else(|_| ObjectHeader::end_16(cursor))?;
    if closed != object_type::OBJECT_GROUP_DATA {
        return Err(format!("object group data closed by {closed:#x}"));
    }
    let closed = ObjectHeader::end_8(cursor).or_else(|_| ObjectHeader::end_16(cursor))?;
    if closed != object_type::DATA_ELEMENT {
        return Err(format!("object group closed by {closed:#x}"));
    }
    Ok(group)
}

fn parse_fragment(cursor: &mut Cursor<'_>) -> ParseResult<(ExGuid, u64, Vec<u8>)> {
    ObjectHeader::expect(cursor, object_type::DATA_ELEMENT_FRAGMENT)?;
    let id = ExGuid::parse(cursor)?;
    let size = cursor.compact_u64()?;
    let _offset = cursor.compact_u64()?;
    let _length = cursor.compact_u64()?;
    let data = cursor.bytes(size as usize)?.to_vec();
    Ok((id, size, data))
}

fn parse_blob(cursor: &mut Cursor<'_>) -> ParseResult<Vec<u8>> {
    ObjectHeader::expect(cursor, object_type::OBJECT_DATA_BLOB)?;
    let size = cursor.compact_u64()? as usize;
    let data = cursor.bytes(size)?.to_vec();
    ObjectHeader::close_data_element(cursor)?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_u64_all_widths() {
        // 1-byte form: the low bit is the selector 0b1, the value is the rest.
        let data = [(5u8 << 1) | 0b1];
        assert_eq!(Cursor::new(&data).compact_u64().unwrap(), 5);
        // 2-byte form: the low two bits of the word carry the selector 0b10.
        let raw = ((0x1234u16 << 2) | 0b10).to_le_bytes();
        assert_eq!(Cursor::new(&raw).compact_u64().unwrap(), 0x1234);
        // zero
        assert_eq!(Cursor::new(&[0u8]).compact_u64().unwrap(), 0);
        // 8-byte form: 0x80 then the u64
        let mut data = vec![0x80];
        data.extend_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
        assert_eq!(
            Cursor::new(&data).compact_u64().unwrap(),
            0x1122_3344_5566_7788
        );
    }

    #[test]
    fn object_headers_decode_type_and_length() {
        // 16-bit header, type 0x15 (DataElementPackage), length 0: 0x15 << 3
        let data = ((object_type::DATA_ELEMENT_PACKAGE as u16) << 3).to_le_bytes();
        let mut cursor = Cursor::new(&data);
        let header = ObjectHeader::parse_16(&mut cursor).unwrap();
        assert_eq!(header.object_type, object_type::DATA_ELEMENT_PACKAGE);
        assert!(!header.compound);

        // 32-bit header, type 0x7A, length 0
        let word = 0b10 | (0x7Au32 << 3);
        let data = word.to_le_bytes();
        let mut cursor = Cursor::new(&data);
        let header = ObjectHeader::parse_32(&mut cursor).unwrap();
        assert_eq!(header.object_type, object_type::ONENOTE_PACKAGING);
    }

    #[test]
    fn end_headers_close_the_right_type() {
        let byte = ((object_type::DATA_ELEMENT as u8) << 2) | 0b01;
        let data = [byte];
        let mut cursor = Cursor::new(&data);
        assert_eq!(
            ObjectHeader::end_8(&mut cursor).unwrap(),
            object_type::DATA_ELEMENT
        );

        let word = ((object_type::OBJECT_GROUP_METADATA_BLOCK) << 2) | 0b11;
        let data = word.to_le_bytes();
        let mut cursor = Cursor::new(&data);
        assert_eq!(
            ObjectHeader::end_16(&mut cursor).unwrap(),
            object_type::OBJECT_GROUP_METADATA_BLOCK
        );
    }

    #[test]
    fn ex_guid_short_form_keeps_guid_and_value() {
        // Short form: low three bits 0b100, value in the high five bits.
        let mut data = vec![(1u8 << 3) | 0b100];
        data.extend_from_slice(&[0x11; 16]);
        let mut cursor = Cursor::new(&data);
        let parsed = ExGuid::parse(&mut cursor).unwrap();
        assert_eq!(parsed.value, 1);
        assert_eq!(parsed.guid.0, [0x11; 16]);

        let mut cursor = Cursor::new(&[0u8]);
        assert!(ExGuid::parse(&mut cursor).unwrap().is_nil());
    }
}
