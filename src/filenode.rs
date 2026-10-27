//! File node lists, fragments and the packed FileNode header ([MS-ONESTORE] 2.4).
//!
//! A `FileNodeListFragment` is `header(16) | nodes | padding | nextFragment(12) | footer(8)`.
//! The node stream ends at `fragment_end - 20`, and the fragment ends with the
//! 0x8BC215C38233BA4B footer. Nodes are variable length and are *not* aligned.

use crate::reader::{u32_at, u64_at, ParseResult};
use std::collections::BTreeMap;

pub const FRAGMENT_MAGIC: u64 = 0xA456_7AB1_F5F7_F4C4;
pub const FRAGMENT_FOOTER: u64 = 0x8BC2_15C3_8233_BA4B;
pub const CHUNK_TERMINATOR: u16 = 0x0FF;

/// `FileNode.FileNodeID` values this converter understands.
pub mod id {
    pub const OBJECT_SPACE_MANIFEST_ROOT: u16 = 0x004;
    pub const OBJECT_SPACE_MANIFEST_LIST_REFERENCE: u16 = 0x008;
    pub const OBJECT_SPACE_MANIFEST_LIST_START: u16 = 0x00C;
    pub const REVISION_MANIFEST_LIST_REFERENCE: u16 = 0x010;
    pub const REVISION_MANIFEST_START_4: u16 = 0x01B;
    pub const REVISION_MANIFEST_START_6: u16 = 0x01E;
    pub const REVISION_MANIFEST_START_7: u16 = 0x01F;
    pub const GLOBAL_ID_TABLE_START: u16 = 0x022;
    pub const GLOBAL_ID_TABLE_ENTRY: u16 = 0x024;
    pub const GLOBAL_ID_TABLE_END: u16 = 0x028;
    pub const ROOT_OBJECT_REFERENCE: u16 = 0x05A;
    pub const OBJECT_DECLARATION_FILE_DATA_3_REF_COUNT: u16 = 0x072;
    pub const OBJECT_DECLARATION_FILE_DATA_3_LARGE_REF_COUNT: u16 = 0x073;
    pub const FILE_DATA_STORE_LIST_REFERENCE: u16 = 0x090;
    pub const FILE_DATA_STORE_OBJECT_REFERENCE: u16 = 0x094;
    pub const OBJECT_DECLARATION_2_REF_COUNT_FND: u16 = 0x0A4;
    pub const OBJECT_DECLARATION_2_LARGE_REF_COUNT_FND: u16 = 0x0A5;
    pub const OBJECT_GROUP_LIST_REFERENCE: u16 = 0x0B0;
    pub const READ_ONLY_OBJECT_DECLARATION_2_REF_COUNT: u16 = 0x0C4;
    pub const READ_ONLY_OBJECT_DECLARATION_2_LARGE_REF_COUNT: u16 = 0x0C5;
    pub const HASHED_CHUNK_DESCRIPTOR: u16 = 0x0C2;
}

/// One entry of a file node list. `at` is the offset of the 4-byte node header,
/// so the `fnd` payload starts at `at + 4`.
#[derive(Debug, Clone)]
pub struct FileNode {
    pub id: u16,
    /// Total size of the node, including its 4-byte header.
    pub size: usize,
    pub at: usize,
    pub stp_format: u8,
    pub cb_format: u8,
}

impl FileNode {
    /// Offset of the `fnd` payload; the 4-byte node header precedes it.
    pub fn data_offset(&self) -> usize {
        self.at + 4
    }
}

/// A `HashedChunkDescriptor2FND` node ([MS-ONESTORE] 2.3.4.1).
#[derive(Debug, Clone)]
pub struct HashedChunk {
    /// Chunk reference to the `ObjectSpaceObjectPropSet` it describes.
    pub stp: u64,
    pub cb: u32,
    /// MD5 of that property set.
    pub hash: [u8; 16],
}

impl HashedChunk {
    pub fn hash_hex(&self) -> String {
        self.hash.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

/// Collect the `HashedChunkDescriptor2FND` entries of a hashed chunk list.
pub fn hashed_chunks(file: &[u8], list: &FileNodeList) -> Vec<HashedChunk> {
    let mut chunks = Vec::new();
    for node in &list.nodes {
        if node.id != id::HASHED_CHUNK_DESCRIPTOR {
            continue;
        }
        let offset = node.data_offset();
        let Ok((stp, cb, used)) =
            read_chunk_reference(file, offset, node.stp_format, node.cb_format)
        else {
            continue;
        };
        let Some(bytes) = file.get(offset + used..offset + used + 16) else {
            continue;
        };
        let mut hash = [0u8; 16];
        hash.copy_from_slice(bytes);
        chunks.push(HashedChunk { stp, cb, hash });
    }
    chunks
}

#[derive(Debug, Clone, Default)]
pub struct FileNodeList {
    pub id: u32,
    pub nodes: Vec<FileNode>,
    pub fragments: usize,
    /// Set when the transaction log declared a shorter active length.
    pub truncated: Option<usize>,
}

/// A `FileNodeChunkReference`: `stp` then `cb`, each sized by the node header.
/// Compressed values (formats 2 and 3) are stored divided by 8.
pub fn read_chunk_reference(
    file: &[u8],
    offset: usize,
    stp_format: u8,
    cb_format: u8,
) -> ParseResult<(u64, u32, usize)> {
    let stp_len = match stp_format {
        0 => 8,
        1 => 4,
        2 => 2,
        3 => 4,
        other => return Err(format!("invalid StpFormat {other}")),
    };
    let cb_len = match cb_format {
        0 => 4,
        1 => 8,
        2 => 1,
        3 => 2,
        other => return Err(format!("invalid CbFormat {other}")),
    };
    let raw_stp = match stp_len {
        8 => u64_at(file, offset)?,
        4 => u32_at(file, offset)? as u64,
        2 => crate::reader::u16_at(file, offset)? as u64,
        _ => unreachable!(),
    };
    let stp = match stp_format {
        2 | 3 => raw_stp.wrapping_mul(8),
        _ => raw_stp,
    };
    let raw_cb = match cb_len {
        4 => u32_at(file, offset + stp_len)? as u64,
        8 => u64_at(file, offset + stp_len)?,
        1 => crate::reader::u8_at(file, offset + stp_len)? as u64,
        2 => crate::reader::u16_at(file, offset + stp_len)? as u64,
        _ => unreachable!(),
    };
    let cb = match cb_format {
        2 | 3 => raw_cb.wrapping_mul(8),
        _ => raw_cb,
    };
    let cb = u32::try_from(cb).map_err(|_| format!("chunk size {cb} exceeds 32 bits"))?;
    Ok((stp, cb, stp_len + cb_len))
}

fn parse_fragment(
    file: &[u8],
    stp: u64,
    cb: u32,
    out: &mut Vec<FileNode>,
) -> ParseResult<Option<(u64, u32)>> {
    let start = usize::try_from(stp).map_err(|_| format!("fragment offset {stp} too large"))?;
    let len = cb as usize;
    if len < 16 + 20 {
        return Err(format!("fragment at {start:#x} is too small ({len} bytes)"));
    }
    let magic = u64_at(file, start)?;
    if magic != FRAGMENT_MAGIC {
        return Err(format!(
            "fragment at {start:#x}: bad magic {magic:#018x}, expected {FRAGMENT_MAGIC:#018x}"
        ));
    }
    let footer = u64_at(file, start + len - 8)?;
    if footer != FRAGMENT_FOOTER {
        return Err(format!(
            "fragment at {start:#x}: bad footer {footer:#018x}, expected {FRAGMENT_FOOTER:#018x}"
        ));
    }
    let next = (
        u64_at(file, start + len - 20)?,
        u32_at(file, start + len - 12)?,
    );
    let limit = start + len - 20;
    let mut pos = start + 16;
    while pos + 4 <= limit {
        let word = u32_at(file, pos)?;
        let node_id = (word & 0x3FF) as u16;
        let size = ((word >> 10) & 0x1FFF) as usize;
        let stp_format = ((word >> 23) & 0x3) as u8;
        let cb_format = ((word >> 25) & 0x3) as u8;
        if node_id == CHUNK_TERMINATOR {
            break;
        }
        if size < 4 {
            // The last fragment of a file node list carries no ChunkTerminatorFND;
            // its node stream simply stops and the rest of the fragment is padding.
            break;
        }
        if pos + size > limit {
            return Err(format!(
                "node at {pos:#x} (id {node_id:#05x}, size {size}) overruns fragment end {limit:#x}"
            ));
        }
        out.push(FileNode {
            id: node_id,
            size,
            at: pos,
            stp_format,
            cb_format,
        });
        pos += size;
    }
    Ok(if next.0 == u64::MAX || next.1 == 0 {
        None
    } else {
        Some(next)
    })
}

/// Parse a file node list, following its fragment chain to the end.
pub fn parse_list(file: &[u8], stp: u64, cb: u32) -> ParseResult<FileNodeList> {
    parse_list_active(file, stp, cb, &std::collections::HashMap::new())
}

/// Parse a file node list and drop nodes that the transaction log does not
/// consider active ([MS-ONESTORE] 2.4: content past the active length belongs to
/// an incomplete transaction). `active` maps `FileNodeListID` to node counts.
pub fn parse_list_active(
    file: &[u8],
    stp: u64,
    cb: u32,
    active: &std::collections::HashMap<u32, usize>,
) -> ParseResult<FileNodeList> {
    let mut list = FileNodeList::default();
    let mut fragment = Some((stp, cb));
    let mut seen: BTreeMap<u64, ()> = BTreeMap::new();
    while let Some((stp, cb)) = fragment {
        if seen.insert(stp, ()).is_some() {
            return Err(format!("fragment chain loops at {stp:#x}"));
        }
        if list.fragments == 0 {
            list.id = u32_at(file, stp as usize + 8)?;
        }
        fragment = parse_fragment(file, stp, cb, &mut list.nodes)?;
        list.fragments += 1;
    }
    if std::env::var_os("ONENOTE_TRACE_NODES").is_some() {
        eprintln!(
            "list {:#x}: parsed {} nodes, log declares {}",
            list.id,
            list.nodes.len(),
            active
                .get(&list.id)
                .map(|count| count.to_string())
                .unwrap_or_else(|| "-".to_string())
        );
    }
    if let Some(&count) = active.get(&list.id) {
        if count < list.nodes.len() {
            // Nodes are appended at the end of a list, so the active prefix is
            // the committed content and the tail is an incomplete transaction.
            if std::env::var_os("ONENOTE_TRACE_NODES").is_some() {
                eprintln!(
                    "list {:#x}: {} nodes parsed, transaction log declares {count} active",
                    list.id,
                    list.nodes.len()
                );
            }
            list.nodes.truncate(count);
            list.truncated = Some(count);
        }
    }
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build one fragment: header(16) | nodes | next(12) | footer(8).
    fn fragment(list_id: u32, nodes: &[Vec<u8>]) -> Vec<u8> {
        let body: usize = nodes.iter().map(Vec::len).sum();
        let mut data = Vec::new();
        data.extend_from_slice(&FRAGMENT_MAGIC.to_le_bytes());
        data.extend_from_slice(&list_id.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        for node in nodes {
            data.extend_from_slice(node);
        }
        data.extend_from_slice(&0u64.to_le_bytes()); // next fragment: none
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&FRAGMENT_FOOTER.to_le_bytes());
        assert_eq!(data.len(), 16 + body + 20);
        data
    }

    fn node(id: u16, payload: &[u8]) -> Vec<u8> {
        let size = 4 + payload.len();
        let word = (id as u32) | ((size as u32) << 10);
        let mut bytes = word.to_le_bytes().to_vec();
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn transaction_log_bounds_the_active_nodes() {
        let data = fragment(0x10, &[node(0x004, &[]), node(0x024, &[])]);
        let all = parse_list(&data, 0, data.len() as u32).unwrap();
        assert_eq!(all.nodes.len(), 2);
        assert_eq!(all.truncated, None);

        let mut active = std::collections::HashMap::new();
        active.insert(0x10u32, 1usize);
        let bounded = parse_list_active(&data, 0, data.len() as u32, &active).unwrap();
        assert_eq!(bounded.id, 0x10);
        assert_eq!(bounded.nodes.len(), 1);
        assert_eq!(bounded.truncated, Some(1));
    }

    #[test]
    fn hashed_chunk_descriptors_are_decoded() {
        // BlobRef as FileChunkReference64x32 (stp 8 bytes, cb 4 bytes) + MD5.
        let mut payload = 0x1400u64.to_le_bytes().to_vec();
        payload.extend_from_slice(&0x20u32.to_le_bytes());
        payload.extend_from_slice(&[0xAB; 16]);
        let data = fragment(0x1EF8, &[node(id::HASHED_CHUNK_DESCRIPTOR, &payload)]);
        let list = parse_list(&data, 0, data.len() as u32).unwrap();
        let chunks = hashed_chunks(&data, &list);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].stp, 0x1400);
        assert_eq!(chunks[0].cb, 0x20);
        assert_eq!(chunks[0].hash_hex(), "abababababababababababababababab");
    }
}
