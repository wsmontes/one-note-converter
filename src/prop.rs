//! Property sets ([MS-ONESTORE] 2.6.1, 2.6.6, 2.6.7) and the property ids that
//! carry OneNote semantics ([MS-ONE] 2.1.12).

use crate::reader::{slice_at, u16_at, u32_at, u64_at, ParseResult};
use std::collections::BTreeMap;

/// `PropertyID.type` values ([MS-ONESTORE] 2.6.6).
pub mod kind {
    pub const NO_DATA: u8 = 0x01;
    pub const BOOL: u8 = 0x02;
    pub const ONE_BYTE: u8 = 0x03;
    pub const TWO_BYTES: u8 = 0x04;
    pub const FOUR_BYTES: u8 = 0x05;
    pub const EIGHT_BYTES: u8 = 0x06;
    pub const LENGTH_PREFIXED: u8 = 0x07;
    pub const OBJECT_ID: u8 = 0x08;
    pub const ARRAY_OF_OBJECT_IDS: u8 = 0x09;
    pub const OBJECT_SPACE_ID: u8 = 0x0A;
    pub const ARRAY_OF_OBJECT_SPACE_IDS: u8 = 0x0B;
    pub const CONTEXT_ID: u8 = 0x0C;
    pub const ARRAY_OF_CONTEXT_IDS: u8 = 0x0D;
    pub const ARRAY_OF_PROPERTY_VALUES: u8 = 0x10;
    pub const PROPERTY_SET: u8 = 0x11;
}

/// Property ids used by the Markdown renderer ([MS-ONE] 2.1.12).
pub mod pid {
    pub const OUTLINE_ELEMENT_CHILD_LEVEL: u32 = 0x1C03;
    pub const IS_TITLE_TIME: u32 = 0x1C87;
    pub const IS_TITLE_DATE: u32 = 0x1CB5;
    pub const IS_TITLE_TEXT: u32 = 0x1CB4;
    pub const BOLD: u32 = 0x1C04;
    pub const ITALIC: u32 = 0x1C05;
    pub const UNDERLINE: u32 = 0x1C06;
    pub const CONTENT_CHILD_NODES: u32 = 0x1C1F;
    pub const ELEMENT_CHILD_NODES: u32 = 0x1C20;
    pub const RICH_TEXT_UNICODE: u32 = 0x1C22;
    pub const LIST_NODES: u32 = 0x1C26;
    pub const NUMBER_LIST_FORMAT: u32 = 0x1C1A;
    pub const PICTURE_CONTAINER: u32 = 0x1C3F;
    pub const EMBEDDED_FILE_CONTAINER: u32 = 0x1D9B;
    pub const EMBEDDED_FILE_NAME: u32 = 0x1D9C;
    pub const CACHED_TITLE_STRING: u32 = 0x1CF3;
    pub const CACHED_TITLE_STRING_FROM_PAGE: u32 = 0x1D3C;
    pub const STRUCTURE_ELEMENT_CHILD_NODES: u32 = 0x1D5F;
    pub const CHILD_GRAPH_SPACE_ELEMENT_NODES: u32 = 0x1D63;
    pub const IMAGE_FILENAME: u32 = 0x1DD7;
    pub const HYPERLINK_URL: u32 = 0x1E20;
    pub const IMAGE_ALT_TEXT: u32 = 0x1E58;
    pub const PARAGRAPH_STYLE: u32 = 0x342C;
    pub const NOTE_TAG_DEFINITION_OID: u32 = 0x3488;
    pub const NOTE_TAG_LABEL: u32 = 0x3468;
    pub const TOPOLOGY_CREATION_TIME_STAMP: u32 = 0x1C65;
    pub const PAGE_LEVEL: u32 = 0x1DFF;
    pub const AUTHOR: u32 = 0x1D75;
    pub const AUTHOR_MOST_RECENT: u32 = 0x1D79;
    pub const AUTHOR_ORIGINAL: u32 = 0x1D78;
    pub const LAST_MODIFIED_TIME_STAMP: u32 = 0x1D77;
    pub const CREATION_TIME_STAMP: u32 = 0x1D09;
    pub const LAST_MODIFIED_TIME: u32 = 0x1D7A;
    pub const TEXT_EXTENDED_ASCII: u32 = 0x3498;
    pub const HIDDEN: u32 = 0x1E16;
    pub const HYPERLINK: u32 = 0x1E14;
    pub const FONT_COLOR: u32 = 0x1C0C;
    pub const HIGHLIGHT: u32 = 0x1C0D;
    pub const STRIKETHROUGH: u32 = 0x1C07;
    pub const TEXT_RUN_FORMATTING: u32 = 0x1E13;
    pub const TEXT_RUN_INDEX: u32 = 0x1E12;
}

/// Object type ids ([MS-ONE] 2.1.13).
pub mod jcid {
    pub const SECTION_NODE: u32 = 0x0006_0007;
    pub const PAGE_SERIES_NODE: u32 = 0x0006_0008;
    pub const PAGE_NODE: u32 = 0x0006_000B;
    pub const OUTLINE_NODE: u32 = 0x0006_000C;
    pub const OUTLINE_ELEMENT_NODE: u32 = 0x0006_000D;
    pub const RICH_TEXT_NODE: u32 = 0x0006_000E;
    pub const IMAGE_NODE: u32 = 0x0006_0011;
    pub const LIST_NODE: u32 = 0x0006_0012;
    pub const TABLE_NODE: u32 = 0x0006_0022;
    pub const TABLE_ROW_NODE: u32 = 0x0006_0023;
    pub const TABLE_CELL_NODE: u32 = 0x0006_0024;
    pub const TITLE_NODE: u32 = 0x0006_002C;
    pub const PAGE_MANIFEST_NODE: u32 = 0x0006_0037;
    pub const PARAGRAPH_STYLE_OBJECT: u32 = 0x0012_004D;
    pub const EMBEDDED_FILE_NODE: u32 = 0x0006_0035;
    pub const INK_CONTAINER: u32 = 0x0006_0014;
    pub const TOC_CONTAINER: u32 = 0x0002_0001;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PropertyId(pub u32);

impl PropertyId {
    pub fn id(self) -> u32 {
        self.0 & 0x03FF_FFFF
    }

    pub fn kind(self) -> u8 {
        ((self.0 >> 26) & 0x1F) as u8
    }

    pub fn bool_value(self) -> bool {
        (self.0 >> 31) & 1 == 1
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    None,
    Bool(bool),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    Bytes(Vec<u8>),
    Oid(u32),
    Oids(Vec<u32>),
    Osid(u32),
    Osids(Vec<u32>),
    ContextId(u32),
    ContextIds(Vec<u32>),
    Array(Vec<PropertySet>),
    Set(Box<PropertySet>),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PropertySet {
    pub properties: Vec<(PropertyId, Value)>,
}

impl PropertySet {
    pub fn get(&self, id: u32) -> Option<&Value> {
        self.properties
            .iter()
            .find(|(pid, _)| pid.id() == id)
            .map(|(_, v)| v)
    }

    pub fn u8(&self, id: u32) -> Option<u8> {
        match self.get(id) {
            Some(Value::U8(v)) => Some(*v),
            _ => None,
        }
    }

    pub fn bytes(&self, id: u32) -> Option<&[u8]> {
        match self.get(id) {
            Some(Value::Bytes(v)) => Some(v),
            _ => None,
        }
    }

    pub fn oid(&self, id: u32) -> Option<u32> {
        match self.get(id) {
            Some(Value::Oid(v)) | Some(Value::Osid(v)) => Some(*v),
            _ => None,
        }
    }

    pub fn oids(&self, id: u32) -> Option<&[u32]> {
        match self.get(id) {
            Some(Value::Oids(v)) | Some(Value::Osids(v)) => Some(v),
            _ => None,
        }
    }

    pub fn bool(&self, id: u32) -> Option<bool> {
        match self.get(id) {
            Some(Value::Bool(v)) => Some(*v),
            _ => None,
        }
    }
}

/// Result of parsing an `ObjectSpaceObjectPropSet`.
#[derive(Debug, Clone, Default)]
pub struct ObjectPropSet {
    pub oids: Vec<u32>,
    pub osids: Vec<u32>,
    pub context_ids: Vec<u32>,
    pub properties: PropertySet,
    /// Bytes consumed from the start of the structure, before 8-byte padding.
    pub consumed: usize,
    /// Set when a property could not be decoded; the properties read before it
    /// are still returned, which keeps the object's text usable.
    pub error: Option<String>,
}

/// Cursors over the object, object-space and context id streams of an
/// `ObjectSpaceObjectPropSet`. Nested property sets (child sets and the
/// elements of `prtArrayOfPropertyValues`) consume from the same streams, in
/// document order, so these cursors must be shared with them.
struct Streams<'a> {
    oids: &'a [u32],
    osids: &'a [u32],
    contexts: &'a [u32],
    oid_cursor: usize,
    osid_cursor: usize,
    context_cursor: usize,
}

impl<'a> Streams<'a> {
    fn take_oid(&mut self, pid: PropertyId) -> ParseResult<u32> {
        let value = *self.oids.get(self.oid_cursor).ok_or_else(|| {
            format!("property {pid:?} wants an object id but the OIDs stream is empty")
        })?;
        self.oid_cursor += 1;
        Ok(value)
    }

    fn take_oids(&mut self, pid: PropertyId, count: usize) -> ParseResult<Vec<u32>> {
        let slice = self
            .oids
            .get(self.oid_cursor..self.oid_cursor.saturating_add(count))
            .ok_or_else(|| {
                format!(
                    "property {pid:?} wants {count} object ids, stream has {}",
                    self.oids.len()
                )
            })?;
        self.oid_cursor += count;
        Ok(slice.to_vec())
    }

    fn take_osid(&mut self, pid: PropertyId) -> ParseResult<u32> {
        let value = *self.osids.get(self.osid_cursor).ok_or_else(|| {
            format!("property {pid:?} wants an object space id but the OSIDs stream is empty")
        })?;
        self.osid_cursor += 1;
        Ok(value)
    }

    fn take_osids(&mut self, pid: PropertyId, count: usize) -> ParseResult<Vec<u32>> {
        let slice = self
            .osids
            .get(self.osid_cursor..self.osid_cursor.saturating_add(count))
            .ok_or_else(|| {
                format!(
                    "property {pid:?} wants {count} object space ids, stream has {}",
                    self.osids.len()
                )
            })?;
        self.osid_cursor += count;
        Ok(slice.to_vec())
    }

    fn take_context(&mut self, pid: PropertyId) -> ParseResult<u32> {
        let value = *self.contexts.get(self.context_cursor).ok_or_else(|| {
            format!("property {pid:?} wants a context id but the ContextIDs stream is empty")
        })?;
        self.context_cursor += 1;
        Ok(value)
    }

    fn take_contexts(&mut self, pid: PropertyId, count: usize) -> ParseResult<Vec<u32>> {
        let slice = self
            .contexts
            .get(self.context_cursor..self.context_cursor.saturating_add(count))
            .ok_or_else(|| {
                format!(
                    "property {pid:?} wants {count} context ids, stream has {}",
                    self.contexts.len()
                )
            })?;
        self.context_cursor += count;
        Ok(slice.to_vec())
    }
}

/// Parse an `ObjectSpaceObjectPropSet` at `offset`.
///
/// Stream layout is `OIDs | OSIDs? | ContextIDs? | PropertySet | padding`.
pub fn parse_object_prop_set(data: &[u8], offset: usize) -> ParseResult<ObjectPropSet> {
    let mut out = ObjectPropSet::default();
    let mut pos = offset;

    let header = u32_at(data, pos)?;
    let count = (header & 0xFF_FFFF) as usize;
    let osid_stream_absent = (header >> 31) & 1 == 1;
    pos += 4;
    for _ in 0..count {
        out.oids.push(u32_at(data, pos)?);
        pos += 4;
    }

    if !osid_stream_absent {
        let h = u32_at(data, pos)?;
        let count = (h & 0xFF_FFFF) as usize;
        let extended_streams = (h >> 30) & 1 == 1;
        pos += 4;
        for _ in 0..count {
            out.osids.push(u32_at(data, pos)?);
            pos += 4;
        }
        if extended_streams {
            let h = u32_at(data, pos)?;
            let count = (h & 0xFF_FFFF) as usize;
            pos += 4;
            for _ in 0..count {
                out.context_ids.push(u32_at(data, pos)?);
                pos += 4;
            }
        }
    }

    let mut streams = Streams {
        oids: &out.oids,
        osids: &out.osids,
        contexts: &out.context_ids,
        oid_cursor: 0,
        osid_cursor: 0,
        context_cursor: 0,
    };
    let (properties, end, error) = parse_property_set(data, pos, &mut streams);
    out.properties = properties;
    out.consumed = end - offset;
    out.error = error;
    Ok(out)
}

fn take<'a>(data: &'a [u8], pos: usize, len: usize) -> ParseResult<&'a [u8]> {
    slice_at(data, pos, len)
}

fn parse_value(
    data: &[u8],
    pos: usize,
    pid: PropertyId,
    streams: &mut Streams<'_>,
) -> ParseResult<(Value, usize)> {
    Ok(match pid.kind() {
        kind::NO_DATA => (Value::None, 0),
        kind::BOOL => (Value::Bool(pid.bool_value()), 0),
        kind::ONE_BYTE => (Value::U8(take(data, pos, 1)?[0]), 1),
        kind::TWO_BYTES => (Value::U16(u16_at(data, pos)?), 2),
        kind::FOUR_BYTES => (Value::U32(u32_at(data, pos)?), 4),
        kind::EIGHT_BYTES => (Value::U64(u64_at(data, pos)?), 8),
        kind::LENGTH_PREFIXED => {
            let len = u32_at(data, pos)? as usize;
            let bytes = take(data, pos + 4, len)?.to_vec();
            (Value::Bytes(bytes), 4 + len)
        }
        kind::OBJECT_ID => (Value::Oid(streams.take_oid(pid)?), 0),
        kind::ARRAY_OF_OBJECT_IDS => {
            let count = u32_at(data, pos)? as usize;
            (Value::Oids(streams.take_oids(pid, count)?), 4)
        }
        kind::OBJECT_SPACE_ID => (Value::Osid(streams.take_osid(pid)?), 0),
        kind::ARRAY_OF_OBJECT_SPACE_IDS => {
            let count = u32_at(data, pos)? as usize;
            (Value::Osids(streams.take_osids(pid, count)?), 4)
        }
        kind::CONTEXT_ID => (Value::ContextId(streams.take_context(pid)?), 0),
        kind::ARRAY_OF_CONTEXT_IDS => {
            let count = u32_at(data, pos)? as usize;
            (Value::ContextIds(streams.take_contexts(pid, count)?), 4)
        }
        kind::ARRAY_OF_PROPERTY_VALUES => {
            // prtArrayOfPropertyValues: cProperties(4) | prid(4) | Data, where a
            // single prid describes the type shared by every element, and Data
            // holds cProperties consecutive property sets.
            let count = u32_at(data, pos)? as usize;
            let mut p = pos + 4;
            if count > 0 {
                let prid = PropertyId(u32_at(data, p)?);
                p += 4;
                if prid.kind() != kind::PROPERTY_SET {
                    return Err(format!(
                        "array of {count} property values declares element type {:#x}, expected 0x11",
                        prid.kind()
                    ));
                }
            }
            let mut sets = Vec::with_capacity(count.min(4096));
            for _ in 0..count {
                let (set, end, error) = parse_property_set(data, p, streams);
                p = end;
                sets.push(set);
                if error.is_some() {
                    break;
                }
            }
            (Value::Array(sets), p - pos)
        }
        kind::PROPERTY_SET => {
            let (set, end, _) = parse_property_set(data, pos, streams);
            (Value::Set(Box::new(set)), end - pos)
        }
        other => {
            return Err(format!("property {pid:?} has unsupported type {other:#x}"));
        }
    })
}

/// Parse a `PropertySet` (2-byte property count, `rgPrids`, `rgData`).
/// Returns the set and the offset just past it. Nested sets share `streams`
/// with the containing `ObjectSpaceObjectPropSet`.
fn parse_property_set(
    data: &[u8],
    offset: usize,
    streams: &mut Streams<'_>,
) -> (PropertySet, usize, Option<String>) {
    let mut pos = offset;
    let count = match u16_at(data, pos) {
        Ok(count) => count as usize,
        Err(e) => return (PropertySet::default(), pos, Some(e)),
    };
    pos += 2;
    let mut prids = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        match u32_at(data, pos) {
            Ok(pid) => prids.push(PropertyId(pid)),
            Err(e) => return (PropertySet::default(), pos, Some(e)),
        }
        pos += 4;
    }
    let mut properties = Vec::with_capacity(count.min(4096));
    let mut error = None;
    for pid in prids {
        match parse_value(data, pos, pid, streams) {
            Ok((value, used)) => {
                pos += used;
                properties.push((pid, value));
            }
            Err(e) => {
                error = Some(format!(
                    "{e} (property {:#010x}, kind {:#04x}, data offset {pos:#x})",
                    pid.0,
                    pid.kind()
                ));
                break;
            }
        }
    }
    (PropertySet { properties }, pos, error)
}

/// Decode a `WzInAtom` (null-terminated UTF-16LE) payload.
pub fn decode_utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    clean_text(&String::from_utf16_lossy(&units))
}

/// Clean a UTF-16 payload and report, for every input unit, how many characters
/// of the cleaned string precede it. Field instructions span several units (and
/// can span text runs), so runs are sliced through this map instead of cleaning
/// each run on its own.
pub fn clean_units(units: &[u16]) -> (String, Vec<usize>) {
    let mut out = String::new();
    // `map[i]` is the number of cleaned characters produced before unit `i`;
    // `map[units.len()]` closes the string.
    let mut map: Vec<usize> = Vec::with_capacity(units.len() + 1);
    let keyword: Vec<u16> = "HYPERLINK ".encode_utf16().collect();
    let quote = '"' as u16;
    let mut index = 0usize;

    let push_char = |out: &mut String, unit: u16| {
        out.push(char::from_u32(unit as u32).unwrap_or('\u{FFFD}'));
    };

    while index < units.len() {
        map.push(out.chars().count());
        let unit = units[index];

        if ('\u{FDD0}' as u16..=('\u{FDEF}' as u16)).contains(&unit) {
            index += 1;
            let rest = &units[index..];
            if rest.starts_with(&keyword) {
                let mut cursor = keyword.len();
                if rest.get(cursor) == Some(&quote) {
                    cursor += 1;
                    while let Some(value) = rest.get(cursor) {
                        cursor += 1;
                        if *value == quote {
                            break;
                        }
                    }
                    for _ in 0..cursor {
                        map.push(out.chars().count());
                    }
                    index += cursor;
                }
            }
            continue;
        }

        if (unit < 0x20 && unit != 0x09 && unit != 0x0A) || unit == 0xFFFC {
            index += 1;
            continue;
        }

        if (0xD800..=0xDBFF).contains(&unit) {
            if let Some(next) = units.get(index + 1) {
                if (0xDC00..=0xDFFF).contains(next) {
                    let combined =
                        0x1_0000 + (((unit as u32 - 0xD800) << 10) | (*next as u32 - 0xDC00));
                    out.push(char::from_u32(combined).unwrap_or('\u{FFFD}'));
                    // Second unit of the pair maps to the same cleaned offset.
                    map.push(out.chars().count());
                    index += 2;
                    continue;
                }
            }
        }

        push_char(&mut out, unit);
        index += 1;
    }
    while map.len() <= units.len() {
        map.push(out.chars().count());
    }
    (out, map)
}

/// OneNote stores rich-text field codes inline: a noncharacter in
/// U+FDD0..=U+FDEF opens a field, followed by its instruction (for example
/// `HYPERLINK "https://…"`). Rendering the instruction verbatim leaks markup
/// into the Markdown, so it is dropped and only the field result is kept.
pub fn clean_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        match rest
            .char_indices()
            .find(|(_, c)| ('\u{FDD0}'..='\u{FDEF}').contains(c))
        {
            Some((position, marker)) => {
                out.push_str(&strip_controls(&rest[..position]));
                let after = &rest[position + marker.len_utf8()..];
                rest = match after.strip_prefix("HYPERLINK ") {
                    Some(instruction) => skip_quoted(instruction),
                    None => after,
                };
            }
            None => {
                out.push_str(&strip_controls(rest));
                return out;
            }
        }
    }
}

/// Skip a leading `"…"` literal and return what follows it.
fn skip_quoted(text: &str) -> &str {
    let Some(open) = text.find('"') else {
        return text;
    };
    let after_open = &text[open + 1..];
    match after_open.find('"') {
        Some(close) => &after_open[close + 1..],
        None => text,
    }
}

fn strip_controls(text: &str) -> String {
    text.chars()
        // U+FFFC marks an embedded object's position in the text; the object
        // itself is rendered on its own.
        .filter(|c| !(c.is_control() && *c != '\n' && *c != '\t') && *c != '\u{FFFC}')
        .collect()
}

/// Code page 1252 (the `Charset` default, 0) for the bytes that differ from
/// Latin-1; the rest of 0x20..=0xFF maps one to one.
const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{FFFD}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{FFFD}', '\u{017D}', '\u{FFFD}',
    '\u{FFFD}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{FFFD}', '\u{017E}', '\u{0178}',
];

/// Decode `TextExtendedAscii`: single-byte text in the paragraph's code page
/// (`Charset`, 0 = Windows-1252). 0xA0 is a non-breaking space and is emitted
/// as a regular space so the Markdown stays plain.
pub fn decode_extended_ascii(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if b == 0 {
            break;
        }
        match b {
            0xA0 => out.push(' '),
            b'\n' | b'\t' => out.push(b as char),
            0x20..=0x7F | 0xA1..=0xFF => out.push(b as char),
            0x80..=0x9F => out.push(CP1252_HIGH[(b - 0x80) as usize]),
            _ => {}
        }
    }
    clean_text(&out)
}

/// Display name for the JCID values the renderer recognizes.
pub fn jcid_name(jcid: u32) -> &'static str {
    match jcid {
        jcid::PAGE_NODE => "jcidPageNode",
        jcid::OUTLINE_NODE => "jcidOutlineNode",
        jcid::OUTLINE_ELEMENT_NODE => "jcidOutlineElementNode",
        jcid::RICH_TEXT_NODE => "jcidRichTextOENode",
        jcid::IMAGE_NODE => "jcidImageNode",
        jcid::LIST_NODE => "jcidNumberListNode",
        jcid::TABLE_NODE => "jcidTableNode",
        jcid::TABLE_ROW_NODE => "jcidTableRowNode",
        jcid::TABLE_CELL_NODE => "jcidTableCellNode",
        jcid::TITLE_NODE => "jcidTitleNode",
        jcid::PAGE_MANIFEST_NODE => "jcidPageManifestNode",
        jcid::PARAGRAPH_STYLE_OBJECT => "jcidParagraphStyleObject",
        0x0002_0030 => "jcidPageMetaData",
        0x0002_0031 => "jcidSectionMetaData",
        0x0002_0044 => "jcidRevisionMetaData",
        0x0002_0046 => "jcidVersionHistoryMetaData",
        0x0006_0007 => "jcidSectionNode",
        0x0006_0008 => "jcidPageSeriesNode",
        0x0006_0035 => "jcidEmbeddedFileNode",
        0x0006_0036 => "jcidEmbeddedFileContainer",
        0x0006_0014 => "jcidInkContainer",
        0x0006_003C => "jcidVersionHistoryContent",
        0x0006_003D => "jcidVersionProxy",
        0x0008_0036 => "jcidEmbeddedFileContainer",
        0x0008_0039 => "jcidPictureContainer14",
        0x0012_0001 => "jcidPersistablePropertyContainerForAuthor",
        _ => "jcid?",
    }
}

/// Property id display name, from the property table in [MS-ONE] 2.1.12.
pub fn pid_name(id: u32) -> &'static str {
    match id {
        0x1C00 => "LayoutTightLayout",
        0x1C01 => "PageWidth",
        0x1C02 => "PageHeight",
        0x1C03 => "OutlineElementChildLevel",
        0x1C04 => "Bold",
        0x1C05 => "Italic",
        0x1C06 => "Underline",
        0x1C07 => "Strikethrough",
        0x1C08 => "Superscript",
        0x1C09 => "Subscript",
        0x1C0A => "Font",
        0x1C0B => "FontSize",
        0x1C0C => "FontColor",
        0x1C0D => "Highlight",
        0x1C12 => "RgOutlineIndentDistance",
        0x1C13 => "BodyTextAlignment",
        0x1C14 => "OffsetFromParentHoriz",
        0x1C15 => "OffsetFromParentVert",
        0x1C1F => "ContentChildNodes",
        0x1C20 => "ElementChildNodes",
        0x1C22 => "RichEditTextUnicode",
        0x1C26 => "ListNodes",
        0x1C30 => "NotebookManagementEntityGuid",
        0x1C3B => "LanguageID",
        0x1C3E => "LayoutAlignmentInParent",
        0x1C3F => "PictureContainer",
        0x1C4C => "PageMarginTop",
        0x1C4D => "PageMarginBottom",
        0x1C4E => "PageMarginLeft",
        0x1C4F => "PageMarginRight",
        0x1C52 => "ListFont",
        0x1C65 => "TopologyCreationTimeStamp",
        0x1C84 => "LayoutAlignmentSelf",
        0x1C87 => "IsTitleTime",
        0x1C88 => "IsBoilerText",
        0x1C8B => "PageSize",
        0x1C8E => "PortraitPage",
        0x1C91 => "EnforceOutlineStructure",
        0x1CB2 => "CannotBeSelected",
        0x1CB4 => "IsTitleText",
        0x1CB5 => "IsTitleDate",
        0x1CB7 => "ListRestart",
        0x1CBD => "IsLayoutSizeSetByUser",
        0x1CCB => "ListSpacingMu",
        0x1CDB => "LayoutOutlineReservedWidth",
        0x1CDC => "LayoutResolveChildCollisions",
        0x1CDE => "IsReadOnly",
        0x1CEC => "LayoutMinimumOutlineWidth",
        0x1CF1 => "LayoutCollisionPriority",
        0x1CF3 => "CachedTitleString",
        0x1CF9 => "DescendantsCannotBeMoved",
        0x1CFE => "RichEditTextLangID",
        0x1CFF => "LayoutTightAlignment",
        0x1D01 => "Charset",
        0x1D09 => "CreationTimeStamp",
        0x1D0C => "Deletable",
        0x1D0E => "ListMSAAIndex",
        0x1D13 => "IsBackground",
        0x1D24 => "IRecordMedia",
        0x1D3C => "CachedTitleStringFromPage",
        0x1D57 => "RowCount",
        0x1D58 => "ColumnCount",
        0x1D5E => "TableBordersVisible",
        0x1D5F => "StructureElementChildNodes",
        0x1D63 => "ChildGraphSpaceElementNodes",
        0x1D66 => "TableColumnWidths",
        0x1D75 => "Author",
        0x1D77 => "LastModifiedTimeStamp",
        0x1D78 => "AuthorOriginal",
        0x1D79 => "AuthorMostRecent",
        0x1D7A => "LastModifiedTime",
        0x1D7C => "IsConflictPage",
        0x1D7D => "TableColumnsLocked",
        0x1D82 => "SchemaRevisionInOrderToRead",
        0x1D96 => "IsConflictObjectForRender",
        0x1D9B => "EmbeddedFileContainer",
        0x1D9C => "EmbeddedFileName",
        0x1D9D => "SourceFilepath",
        0x1D9E => "ConflictingUserName",
        0x1DD7 => "ImageFilename",
        0x1DDB => "IsConflictObjectForSelection",
        0x1DFF => "PageLevel",
        0x1E12 => "TextRunIndex",
        0x1E13 => "TextRunFormatting",
        0x1E14 => "Hyperlink",
        0x1E15 => "UnderlineType",
        0x1E16 => "Hidden",
        0x1E19 => "HyperlinkProtected",
        0x1E20 => "WzHyperlinkUrl",
        0x1E22 => "TextRunIsEmbeddedObject",
        0x1E26 => "CellShadingColor",
        0x1E58 => "ImageAltText",
        0x3401 => "MathFormatting",
        0x342C => "ParagraphStyle",
        0x342E => "ParagraphSpaceBefore",
        0x342F => "ParagraphSpaceAfter",
        0x3430 => "ParagraphLineSpacingExact",
        0x3442 => "MetaDataObjectsAboveGraphSpace",
        0x3458 => "TextRunDataObject",
        0x345A => "ParagraphStyleId",
        0x3462 => "HasVersionPages",
        0x3498 => "TextExtendedAscii",
        0x3499 => "TextRunData",
        0x34CB => "ImageUploadState",
        0x34CD => "PictureWidth",
        0x34CE => "PictureHeight",
        _ => "?",
    }
}

/// Aggregate JCID histogram helper used by `inspect`.
pub fn count_jcids(objects: impl Iterator<Item = u32>) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for jcid in objects {
        *out.entry(format!("{:#010x} {}", jcid, jcid_name(jcid)))
            .or_insert(0) += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn property_set_uses_a_two_byte_property_count() {
        let mut data = Vec::new();
        data.extend_from_slice(&0x8000_0001u32.to_le_bytes()); // OIDs: 1 entry, no OSID stream
        data.extend_from_slice(&39u32.to_le_bytes()); // OIDs.body
        data.extend_from_slice(&2u16.to_le_bytes()); // cProperties
        data.extend_from_slice(&0x8800_1C04u32.to_le_bytes()); // Bold = true
        data.extend_from_slice(&0x1400_1C01u32.to_le_bytes()); // PageWidth
        data.extend_from_slice(&0x1234u32.to_le_bytes()); // PageWidth value

        let parsed = parse_object_prop_set(&data, 0).expect("parses");
        assert_eq!(parsed.oids, vec![39]);
        assert_eq!(parsed.consumed, data.len(), "blob must be fully consumed");
        assert_eq!(parsed.properties.bool(pid::BOLD), Some(true));
        assert_eq!(
            parsed.properties.get(0x1C01),
            Some(&Value::U32(0x1234)),
            "four-byte property keeps its value"
        );
    }

    #[test]
    fn array_of_property_values_has_a_single_prid() {
        // cProperties(4) | prid(4, type 0x11) | two child property sets, each
        // holding one four-byte property.
        let mut data = Vec::new();
        data.extend_from_slice(&0x8000_0000u32.to_le_bytes()); // OIDs: none
        data.extend_from_slice(&1u16.to_le_bytes()); // parent cProperties
        data.extend_from_slice(&0x4000_3499u32.to_le_bytes()); // TextRunData
        data.extend_from_slice(&2u32.to_le_bytes()); // array elements
        data.extend_from_slice(&0x4400_0011u32.to_le_bytes()); // the single prid (type 0x11)
        for value in [0xAAu32, 0xBB] {
            data.extend_from_slice(&1u16.to_le_bytes()); // child cProperties
            data.extend_from_slice(&0x1400_1C01u32.to_le_bytes()); // PageWidth
            data.extend_from_slice(&value.to_le_bytes());
        }

        let parsed = parse_object_prop_set(&data, 0).expect("parses");
        assert_eq!(parsed.error, None);
        assert_eq!(parsed.consumed, data.len());
        match parsed.properties.get(0x3499) {
            Some(Value::Array(sets)) => {
                assert_eq!(sets.len(), 2);
                assert_eq!(sets[0].get(0x1C01), Some(&Value::U32(0xAA)));
                assert_eq!(sets[1].get(0x1C01), Some(&Value::U32(0xBB)));
            }
            other => panic!("expected an array, got {other:?}"),
        }
    }

    #[test]
    fn property_type_lives_in_the_high_bits() {
        let pid = PropertyId(0x1C00_1C22);
        assert_eq!(pid.id(), pid::RICH_TEXT_UNICODE);
        assert_eq!(pid.kind(), kind::LENGTH_PREFIXED);
        assert!(!pid.bool_value());
    }

    #[test]
    fn utf16_payload_stops_at_the_first_nul() {
        let mut bytes = Vec::new();
        for unit in [0x0048u16, 0x0069, 0x0000, 0x006A] {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode_utf16(&bytes), "Hi");
    }

    #[test]
    fn extended_ascii_uses_windows_1252_for_the_high_range() {
        // 0x93/0x94 are curly quotes and 0x96 an en dash in cp1252; Latin-1
        // would turn them into C1 controls and they would be dropped.
        assert_eq!(
            decode_extended_ascii(&[0x93, b'a', 0x94, b' ', 0x96]),
            "\u{201C}a\u{201D} \u{2013}"
        );
        // 0xA0 is a non-breaking space and is emitted as a plain space.
        assert_eq!(decode_extended_ascii(&[b'a', 0xA0, b'b']), "a b");
    }

    #[test]
    fn field_instructions_are_stripped_from_text() {
        let raw = "Watch the \u{FDD0}HYPERLINK \"http://example.test/v\"2 minute video";
        assert_eq!(clean_text(raw), "Watch the 2 minute video");
        assert_eq!(clean_text("plain"), "plain");
    }

    #[test]
    fn problem_characters_are_treated_as_field_openers() {
        // U+FDDF is the marker observed in OneNote 2010 files.
        let raw = "\u{FDDF}HYPERLINK \"http://x\"result";
        assert_eq!(clean_text(raw), "result");
    }
}
