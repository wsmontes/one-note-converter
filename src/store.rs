//! Revision store model: object spaces, revisions, object groups and objects.
//!
//! Traversal follows [MS-ONESTORE] 1.3.1:
//! root file node list -> object space manifest lists -> revision manifest lists
//! -> object group lists -> object declarations (BlobRef to a property set).

use crate::filenode::{self, id as fnid, read_chunk_reference, FileNode, FileNodeList};
use crate::guid::{ExtendedGuid, Guid};
use crate::header::Header;
use crate::prop::{self, ObjectPropSet, PropertySet};
use crate::reader::{u16_at, u32_at, u64_at, ParseResult};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, Clone)]
pub struct Object {
    pub oid: u32,
    pub jcid: u32,
    pub props: PropertySet,
    /// Size in bytes of the property set blob, and how much of it was consumed
    /// before padding. `blob - consumed` is always < 8 for a well-formed file.
    pub blob_size: usize,
    pub consumed: usize,
    /// Offset of the property set blob in the file.
    pub blob_offset: usize,
    /// Set when the property set was only partially decoded.
    pub parse_error: Option<String>,
    /// Text carried by superseded declarations of the same oid. Revision
    /// manifests belong to different contexts, and a later declaration can
    /// drop the text a previous one carried; keeping the older variants stops
    /// content from disappearing.
    pub text_variants: Vec<String>,
}

/// A file data object declaration (FileNodeID 0x072/0x073): points at binary
/// payload stored either in the file data store or beside the section file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileDataRef {
    pub oid: u32,
    pub jcid: u32,
    /// `<ifndf>{GUID}`, `<file>name.onebin` or `<invfdo>`.
    pub reference: String,
    /// Extension including the leading period, e.g. `.png`.
    pub extension: String,
}

impl FileDataRef {
    /// The FileDataStoreObject GUID, when the payload lives inside this file.
    pub fn store_guid(&self) -> Option<Guid> {
        self.reference
            .strip_prefix("<ifndf>")
            .and_then(|rest| Guid::parse(rest.trim()))
    }

    /// The onefiles sibling name, when the payload lives beside the section.
    pub fn sibling_file(&self) -> Option<&str> {
        self.reference
            .strip_prefix("<file>")
            .map(str::trim)
            .filter(|name| !name.is_empty())
    }
}

/// What an object space holds, inferred from the object types it declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceKind {
    /// `.one` section space: section node plus page series.
    Section,
    /// Page space: page manifest / page metadata.
    Page,
    /// Version history or conflict content, not part of the current section text.
    VersionHistory,
    Other,
}

impl std::fmt::Display for SpaceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            SpaceKind::Section => "section",
            SpaceKind::Page => "page",
            SpaceKind::VersionHistory => "version-history",
            SpaceKind::Other => "other",
        };
        f.write_str(name)
    }
}

/// A root object of a revision, as declared by `RootObjectReference3FND`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootObject {
    pub role: u32,
    pub oid: u32,
}

#[derive(Debug, Clone, Default)]
pub struct ObjectSpace {
    pub gosid: ExtendedGuid,
    pub roots: Vec<RootObject>,
    /// Revisions whose property sets are encrypted; their content is omitted.
    pub encrypted_revisions: usize,
    /// Root declarations from superseded revisions whose objects are gone.
    pub superseded_roots: usize,
    pub objects: BTreeMap<u32, Object>,
    pub file_data_refs: BTreeMap<u32, FileDataRef>,
    pub gid_table: BTreeMap<u32, Guid>,
    pub manifest_list_fragments: usize,
}

impl ObjectSpace {
    /// The role-1 root of the current revision: the section node for a section
    /// space, the page manifest for a page space.
    ///
    /// A space can hold revision manifests from several contexts at once (the
    /// page and its version history), so a page manifest wins over a version
    /// history content root when both are declared.
    pub fn content_root(&self) -> Option<u32> {
        let candidates = self
            .roots
            .iter()
            .filter(|root| root.role == 1 && self.objects.contains_key(&root.oid));
        let mut best: Option<(u8, u32)> = None;
        for root in candidates {
            let jcid = self.objects[&root.oid].jcid;
            let rank = match jcid {
                crate::prop::jcid::SECTION_NODE | crate::prop::jcid::PAGE_SERIES_NODE => 0,
                crate::prop::jcid::PAGE_MANIFEST_NODE => 1,
                0x0006_003C => 2,
                _ => 3,
            };
            if best.is_none_or(|(best_rank, _)| rank < best_rank) {
                best = Some((rank, root.oid));
            }
        }
        best.map(|(_, oid)| oid)
    }

    pub fn kind(&self) -> SpaceKind {
        if let Some(oid) = self.content_root() {
            match self.objects.get(&oid).map(|object| object.jcid) {
                Some(crate::prop::jcid::SECTION_NODE)
                | Some(crate::prop::jcid::PAGE_SERIES_NODE) => return SpaceKind::Section,
                Some(crate::prop::jcid::PAGE_MANIFEST_NODE) => return SpaceKind::Page,
                Some(0x0006_003C) => return SpaceKind::VersionHistory,
                _ => {}
            }
        }
        let (mut section, mut page, mut version_history) = (false, false, false);
        for object in self.objects.values() {
            match object.jcid {
                crate::prop::jcid::SECTION_NODE | crate::prop::jcid::PAGE_SERIES_NODE => {
                    section = true
                }
                crate::prop::jcid::PAGE_MANIFEST_NODE | 0x0002_0030 => page = true,
                0x0002_0046 | 0x0006_003C | 0x0006_003D => version_history = true,
                _ => {}
            }
        }
        if version_history {
            SpaceKind::VersionHistory
        } else if section {
            SpaceKind::Section
        } else if page {
            SpaceKind::Page
        } else {
            SpaceKind::Other
        }
    }
}

#[derive(Debug, Clone)]
pub struct Store {
    pub header: Header,
    pub spaces: Vec<ObjectSpace>,
    /// FileDataStoreObject payload ranges keyed by their GUID reference.
    pub file_data: BTreeMap<Guid, (usize, usize)>,
    /// Payloads that live inside a package store rather than at a file offset.
    pub blobs: BTreeMap<Guid, Vec<u8>>,
    pub file_size: usize,
    pub warnings: Vec<String>,
    /// Active node count per `FileNodeListID`, as declared by the transaction log.
    pub list_counts: HashMap<u32, usize>,
    /// Entries of the hashed chunk list ([MS-ONESTORE] 2.3.4), when present.
    pub hashed_chunks: Vec<filenode::HashedChunk>,
    /// Number of transaction log entries that were active.
    pub transactions: usize,
}

#[derive(Debug, Clone, Default)]
struct Revision {
    /// Chunk references to object group file node lists.
    group_refs: Vec<(u64, u32)>,
    /// `(root role, oidRoot)` pairs; the oid is resolved once the global
    /// identification table of the object space is known.
    roots: Vec<(u32, ExtendedGuid)>,
    /// `odcsDefault == 0x0002`: every property set in this revision is
    /// encrypted and MUST be ignored ([MS-ONESTORE] 2.5.7).
    encrypted: bool,
}

impl Store {
    pub fn parse(file: &[u8]) -> ParseResult<Store> {
        let header = Header::parse(file)?;
        let mut warnings = Vec::new();
        let root_ref = header.fcr_file_node_list_root;
        if !root_ref.is_valid() {
            return Err("header has no root file node list reference".to_string());
        }
        let (list_counts, transactions) = transaction_log_counts(file, &header);
        let root = filenode::parse_list_active(file, root_ref.stp, root_ref.cb, &list_counts)?;
        if std::env::var_os("ONENOTE_TRACE_NODES").is_some() {
            eprintln!(
                "root {} ({} fragments): {:?}",
                root.id,
                root.fragments,
                root.nodes
                    .iter()
                    .map(|n| (n.id, n.size))
                    .collect::<Vec<_>>()
            );
        }

        let mut space_specs: Vec<(ExtendedGuid, &FileNode)> = Vec::new();
        let mut root_gosid = None;
        let mut file_data_store_list = None;
        for node in &root.nodes {
            match node.id {
                fnid::FILE_DATA_STORE_LIST_REFERENCE => {
                    let (stp, cb, _) = read_chunk_reference(
                        file,
                        node.data_offset(),
                        node.stp_format,
                        node.cb_format,
                    )?;
                    file_data_store_list = Some((stp, cb));
                }
                fnid::OBJECT_SPACE_MANIFEST_ROOT => {
                    root_gosid = Some(ExtendedGuid::read(file, node.data_offset())?);
                }
                fnid::OBJECT_SPACE_MANIFEST_LIST_REFERENCE => {
                    let (stp, cb, len) = read_chunk_reference(
                        file,
                        node.data_offset(),
                        node.stp_format,
                        node.cb_format,
                    )?;
                    let gosid = ExtendedGuid::read(file, node.data_offset() + len)?;
                    let _ = (stp, cb);
                    space_specs.push((gosid, node));
                }
                _ => {}
            }
        }
        if space_specs.is_empty() {
            return Err("root file node list declares no object spaces".to_string());
        }

        let mut spaces = Vec::with_capacity(space_specs.len());
        for (gosid, node) in space_specs {
            let (stp, cb, _) =
                read_chunk_reference(file, node.data_offset(), node.stp_format, node.cb_format)?;
            let space = parse_object_space(file, gosid, stp, cb, &mut warnings, &list_counts)?;
            spaces.push(space);
        }
        if let Some(root_gosid) = root_gosid {
            if !spaces.iter().any(|s| s.gosid.guid == root_gosid.guid) {
                warnings.push(format!(
                    "root object space {root_gosid} is not present in the manifest list"
                ));
            }
        }

        let mut file_data = BTreeMap::new();
        if let Some((stp, cb)) = file_data_store_list {
            let list = filenode::parse_list_active(file, stp, cb, &list_counts)?;
            for node in &list.nodes {
                if node.id != fnid::FILE_DATA_STORE_OBJECT_REFERENCE {
                    continue;
                }
                let (ostp, ocb, len) = read_chunk_reference(
                    file,
                    node.data_offset(),
                    node.stp_format,
                    node.cb_format,
                )?;
                let guid = Guid::read(file, node.data_offset() + len)?;
                match parse_file_data_store_object(file, ostp, ocb) {
                    Ok(range) => {
                        file_data.insert(guid, range);
                    }
                    Err(message) => warnings.push(format!("file data object {guid}: {message}")),
                }
            }
        }

        let hashed_chunks = if header.fcr_hashed_chunk_list.is_valid() {
            let reference = header.fcr_hashed_chunk_list;
            match filenode::parse_list_active(file, reference.stp, reference.cb, &list_counts) {
                Ok(list) => filenode::hashed_chunks(file, &list),
                Err(message) => {
                    warnings.push(format!("hashed chunk list: {message}"));
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        Ok(Store {
            header,
            spaces,
            file_data,
            blobs: BTreeMap::new(),
            file_size: file.len(),
            warnings,
            list_counts,
            hashed_chunks,
            transactions,
        })
    }

    /// Resolve an object space CompactID seen inside `space` to an index into
    /// `self.spaces`, using that space's global identification table.
    pub fn resolve_space(&self, space: &ObjectSpace, compact_id: u32) -> Option<usize> {
        let guid = space.gid_table.get(&compact_id)?;
        self.spaces
            .iter()
            .position(|s| s.gosid.guid == *guid && s.gosid.n == space.gosid.n)
            .or_else(|| self.spaces.iter().position(|s| s.gosid.guid == *guid))
    }

    pub fn all_objects(&self) -> impl Iterator<Item = (&ObjectSpace, &Object)> {
        self.spaces
            .iter()
            .flat_map(|space| space.objects.values().map(move |obj| (space, obj)))
    }
}

/// Active node counts per file node list, derived from the transaction log
/// ([MS-ONESTORE] 2.3.3): only the first `cTransactionsInLog` sentinel-terminated
/// transactions are committed, and each entry declares the new node count of one
/// list. Everything after the last active sentinel is an incomplete transaction.
fn transaction_log_counts(file: &[u8], header: &Header) -> (HashMap<u32, usize>, usize) {
    let mut counts = HashMap::new();
    let reference = header.fcr_transaction_log;
    if !reference.is_valid() {
        return (counts, 0);
    }
    let limit = header.c_transactions_in_log as usize;
    let mut committed = 0usize;
    let mut next = Some((reference.stp, reference.cb));
    let mut seen = std::collections::BTreeSet::new();
    while let Some((stp, cb)) = next {
        if limit == 0 || committed >= limit || !seen.insert(stp) {
            break;
        }
        let Ok(start) = usize::try_from(stp) else {
            break;
        };
        let len = cb as usize;
        if len < 12 {
            break;
        }
        let entries = (len - 12) / 8;
        for index in 0..entries {
            let offset = start + index * 8;
            let (Ok(src), Ok(value)) = (
                crate::reader::u32_at(file, offset),
                crate::reader::u32_at(file, offset + 4),
            ) else {
                break;
            };
            if src == 1 {
                committed += 1;
                if committed >= limit {
                    break;
                }
            } else {
                counts.insert(src, value as usize);
            }
        }
        // `TransactionLogFragment` ends with a FileChunkReference64x32.
        let next_at = start + entries * 8;
        next = match (
            crate::reader::u64_at(file, next_at),
            crate::reader::u32_at(file, next_at + 8),
        ) {
            (Ok(next_stp), Ok(next_cb)) if next_stp != 0 && next_cb != 0 => {
                Some((next_stp, next_cb))
            }
            _ => None,
        };
    }
    (counts, committed)
}

fn parse_object_space(
    file: &[u8],
    gosid: ExtendedGuid,
    manifest_stp: u64,
    manifest_cb: u32,
    warnings: &mut Vec<String>,
    active: &HashMap<u32, usize>,
) -> ParseResult<ObjectSpace> {
    let manifest = filenode::parse_list_active(file, manifest_stp, manifest_cb, active)?;
    if std::env::var_os("ONENOTE_TRACE_NODES").is_some() {
        eprintln!(
            "manifest {} ({} fragments): {:?}",
            manifest.id,
            manifest.fragments,
            manifest.nodes.iter().map(|n| n.id).collect::<Vec<_>>()
        );
    }
    let mut output = ObjectSpace {
        gosid,
        manifest_list_fragments: manifest.fragments,
        ..ObjectSpace::default()
    };

    let mut revision_lists = Vec::new();
    for node in &manifest.nodes {
        match node.id {
            fnid::OBJECT_SPACE_MANIFEST_LIST_START => {
                let declared = ExtendedGuid::read(file, node.data_offset())?;
                if declared.guid != gosid.guid {
                    warnings.push(format!(
                        "object space manifest list start declares {declared}, manifest reference says {gosid}"
                    ));
                }
            }
            fnid::REVISION_MANIFEST_LIST_REFERENCE => {
                let (stp, cb, _) = read_chunk_reference(
                    file,
                    node.data_offset(),
                    node.stp_format,
                    node.cb_format,
                )?;
                revision_lists.push((stp, cb));
            }
            _ => {}
        }
    }

    // Revisions are deltas: the current object state is the union of every
    // declaration in the chain, applied in order (last write wins).
    let mut pending_roots: Vec<(u32, ExtendedGuid)> = Vec::new();
    for (stp, cb) in revision_lists {
        let list = filenode::parse_list_active(file, stp, cb, active)?;
        for revision in split_revisions(file, &list)? {
            if revision.encrypted {
                // Property sets of an encrypted revision MUST be ignored.
                output.encrypted_revisions += 1;
                continue;
            }
            pending_roots.extend(revision.roots.iter().copied());
            for (gstp, gcb) in revision.group_refs {
                let group = filenode::parse_list_active(file, gstp, gcb, active)?;
                let mut in_gid_table = false;
                for node in &group.nodes {
                    match node.id {
                        fnid::GLOBAL_ID_TABLE_START => in_gid_table = true,
                        fnid::GLOBAL_ID_TABLE_END => in_gid_table = false,
                        fnid::GLOBAL_ID_TABLE_ENTRY => {
                            let compact = u32_at(file, node.data_offset())?;
                            let guid = Guid::read(file, node.data_offset() + 4)?;
                            output.gid_table.insert(compact, guid);
                        }
                        fnid::OBJECT_DECLARATION_FILE_DATA_3_REF_COUNT
                        | fnid::OBJECT_DECLARATION_FILE_DATA_3_LARGE_REF_COUNT => {
                            let decl = parse_file_data_declaration(file, node)?;
                            if in_gid_table {
                                continue;
                            }
                            output.file_data_refs.insert(decl.oid, decl);
                        }
                        fnid::OBJECT_DECLARATION_2_REF_COUNT_FND
                        | fnid::OBJECT_DECLARATION_2_LARGE_REF_COUNT_FND
                        | fnid::READ_ONLY_OBJECT_DECLARATION_2_REF_COUNT
                        | fnid::READ_ONLY_OBJECT_DECLARATION_2_LARGE_REF_COUNT => {
                            if in_gid_table {
                                continue;
                            }
                            let object = match parse_declaration(file, node) {
                                Ok(object) => object,
                                Err(message) => {
                                    // A single unreadable object must not cost
                                    // the whole section.
                                    warnings.push(format!(
                                        "object space {gosid}: skipped declaration at {:#x}                                          (node {:#05x}): {message}",
                                        node.data_offset(),
                                        node.id
                                    ));
                                    continue;
                                }
                            };
                            if let Some(error) = &object.parse_error {
                                warnings.push(format!(
                                    "object space {gosid}: object {} ({}) decoded partially: {error}",
                                    object.oid,
                                    crate::prop::jcid_name(object.jcid)
                                ));
                            }
                            match output.objects.get(&object.oid) {
                                Some(existing) => {
                                    // Carry the superseded declaration's text
                                    // forward: a later revision (often from a
                                    // different context) can drop it.
                                    let mut variants = existing.text_variants.clone();
                                    if let Some(text) = object_text(existing) {
                                        if !variants.contains(&text) {
                                            variants.push(text);
                                        }
                                    }
                                    for text in &object.text_variants {
                                        if !variants.contains(text) {
                                            variants.push(text.clone());
                                        }
                                    }
                                    let mut merged = object;
                                    merged.text_variants = variants;
                                    output.objects.insert(merged.oid, merged);
                                }
                                None => {
                                    output.objects.insert(object.oid, object);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // A root's oidRoot is an ExtendedGUID naming an entry of the global
    // identification table plus an object instance: CompactID = (index << 8) | n.
    pending_roots.dedup();
    let mut superseded_roots = 0usize;
    let mut unknown_root_guids = 0usize;
    for (role, oid_root) in pending_roots {
        let Some(index) = output
            .gid_table
            .iter()
            .find(|(_, guid)| **guid == oid_root.guid)
            .map(|(index, _)| *index)
        else {
            unknown_root_guids += 1;
            continue;
        };
        let oid = (index << 8) | (oid_root.n & 0xFF);
        if output.objects.contains_key(&oid) {
            output.roots.push(RootObject { role, oid });
        } else {
            // Expected for revisions that were superseded: their root objects
            // are no longer declared. Counted, not reported one by one.
            superseded_roots += 1;
        }
    }
    {
        let mut seen = std::collections::HashSet::new();
        output
            .roots
            .retain(|root| seen.insert((root.role, root.oid)));
    }
    output.superseded_roots = superseded_roots + unknown_root_guids;
    if output.encrypted_revisions > 0 {
        warnings.push(format!(
            "object space {gosid}: {} encrypted revision(s) skipped; their content is omitted",
            output.encrypted_revisions
        ));
    }
    if output.objects.is_empty() {
        warnings.push(format!("object space {gosid} has no object declarations"));
    }
    Ok(output)
}

fn split_revisions(file: &[u8], list: &FileNodeList) -> ParseResult<Vec<Revision>> {
    if std::env::var_os("ONENOTE_TRACE_NODES").is_some() {
        eprintln!(
            "list {} ({} fragments): {:?}",
            list.id,
            list.fragments,
            list.nodes.iter().map(|n| n.id).collect::<Vec<_>>()
        );
    }
    let mut revisions: Vec<Revision> = Vec::new();
    for node in &list.nodes {
        match node.id {
            fnid::REVISION_MANIFEST_START_4
            | fnid::REVISION_MANIFEST_START_6
            | fnid::REVISION_MANIFEST_START_7 => {
                // START_4 (`.onetoc2`): rid(20) | ridDependent(20) |
                // timeCreation(8) | RevisionRole(4) | odcsDefault(2).
                // START_6: rid(20) | ridDependent(20) | role(4) | odcs(2).
                // START_7 embeds a START_6 block.
                let odcs_offset = if node.id == fnid::REVISION_MANIFEST_START_4 {
                    52
                } else {
                    44
                };
                let odcs_default = u16_at(file, node.data_offset() + odcs_offset)?;
                revisions.push(Revision {
                    group_refs: Vec::new(),
                    roots: Vec::new(),
                    encrypted: odcs_default == 0x0002,
                });
            }
            fnid::ROOT_OBJECT_REFERENCE => {
                // oidRoot is an ExtendedGUID: guid(16) + n(4), then RootRole(4).
                let guid = Guid::read(file, node.data_offset())?;
                let n = u32_at(file, node.data_offset() + 16)?;
                let role = u32_at(file, node.data_offset() + 20)?;
                if let Some(revision) = revisions.last_mut() {
                    revision.roots.push((role, ExtendedGuid { guid, n }));
                }
            }
            fnid::OBJECT_GROUP_LIST_REFERENCE => {
                let (stp, cb, _) = read_chunk_reference(
                    file,
                    node.data_offset(),
                    node.stp_format,
                    node.cb_format,
                )?;
                if let Some(rev) = revisions.last_mut() {
                    rev.group_refs.push((stp, cb));
                }
            }
            _ => {}
        }
    }
    Ok(revisions)
}

fn parse_declaration(file: &[u8], node: &FileNode) -> ParseResult<Object> {
    let (stp, cb, ref_len) =
        read_chunk_reference(file, node.data_offset(), node.stp_format, node.cb_format)?;
    let body = node.data_offset() + ref_len;
    let oid = u32_at(file, body)?;
    let jcid = u32_at(file, body + 4)?;
    let start = usize::try_from(stp)
        .map_err(|_| format!("object {oid} blob offset {stp} exceeds addressable size"))?;
    let end = start.saturating_add(cb as usize);
    if end > file.len() {
        return Err(format!(
            "object {oid}: blob {start:#x} + {cb:#x} runs past the end of the {:#x}-byte file",
            file.len()
        ));
    }
    // Bound the parse to the blob: running past it would read the next
    // object's bytes and invent properties.
    let blob = prop::parse_object_prop_set(&file[start..end], 0)?;
    let ObjectPropSet {
        properties,
        consumed,
        ..
    } = blob;
    Ok(Object {
        oid,
        jcid,
        props: properties,
        blob_size: cb as usize,
        consumed,
        blob_offset: start,
        parse_error: blob.error,
        text_variants: Vec::new(),
    })
}

/// The file data payload for a stored GUID, if this file carries it.
pub fn file_data_bytes<'a>(store: &'a Store, file: &'a [u8], guid: Guid) -> Option<&'a [u8]> {
    if let Some(bytes) = store.blobs.get(&guid) {
        return Some(bytes);
    }
    let (offset, len) = *store.file_data.get(&guid)?;
    file.get(offset..offset + len)
}

fn parse_file_data_declaration(file: &[u8], node: &FileNode) -> ParseResult<FileDataRef> {
    // oid(4) | jcid(4) | cRef(1 or 4) | FileDataReference | Extension
    let body = node.data_offset();
    let oid = u32_at(file, body)?;
    let jcid = u32_at(file, body + 4)?;
    let c_ref_len = if node.id == fnid::OBJECT_DECLARATION_FILE_DATA_3_LARGE_REF_COUNT {
        4
    } else {
        1
    };
    let (reference, used) = read_storage_string(file, body + 8 + c_ref_len)?;
    let (extension, _) = read_storage_string(file, body + 8 + c_ref_len + used)?;
    Ok(FileDataRef {
        oid,
        jcid,
        reference,
        extension,
    })
}

fn read_storage_string(file: &[u8], offset: usize) -> ParseResult<(String, usize)> {
    let cch = u32_at(file, offset)? as usize;
    let bytes = crate::reader::slice_at(file, offset + 4, cch * 2)?;
    Ok((crate::prop::decode_utf16(bytes), 4 + cch * 2))
}

const FILE_DATA_HEADER_GUID: &str = "{BDE316E7-2665-4511-A4C4-8D4D0B7A9EAC}";
const FILE_DATA_FOOTER_GUID: &str = "{71FBA722-0F79-4A0B-BB13-899256426B24}";

fn parse_file_data_store_object(file: &[u8], stp: u64, cb: u32) -> ParseResult<(usize, usize)> {
    let start = usize::try_from(stp).map_err(|_| format!("offset {stp} too large"))?;
    let end = start
        .checked_add(cb as usize)
        .ok_or_else(|| "file data object overflows address space".to_string())?;
    let header = Guid::read(file, start)?;
    let expected = Guid::parse(FILE_DATA_HEADER_GUID)
        .ok_or_else(|| "internal: bad header guid literal".to_string())?;
    if header != expected {
        return Err(format!("bad FileDataStoreObject header {header}"));
    }
    let length = u64_at(file, start + 16)? as usize;
    let data_start = start + 36; // guidHeader(16) + cbLength(8) + unused(4) + reserved(8)
    if data_start + length > end {
        return Err(format!(
            "FileDataStoreObject declares {length} bytes of data but only {} fit",
            end.saturating_sub(data_start)
        ));
    }
    let footer = Guid::read(file, end - 16)?;
    let expected = Guid::parse(FILE_DATA_FOOTER_GUID)
        .ok_or_else(|| "internal: bad footer guid literal".to_string())?;
    if footer != expected {
        return Err(format!("bad FileDataStoreObject footer {footer}"));
    }
    Ok((data_start, length))
}

/// Number of objects reachable from `root` through every child collection,
/// a direct check that no page content is left behind.
pub fn reachable_from(space: &ObjectSpace, root: u32) -> usize {
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![root];
    while let Some(oid) = stack.pop() {
        if !seen.insert(oid) {
            continue;
        }
        let Some(object) = space.objects.get(&oid) else {
            continue;
        };
        for id in [
            crate::prop::pid::ELEMENT_CHILD_NODES,
            crate::prop::pid::CONTENT_CHILD_NODES,
            crate::prop::pid::STRUCTURE_ELEMENT_CHILD_NODES,
        ] {
            if let Some(ids) = object.props.oids(id) {
                stack.extend(ids.iter().copied());
            }
        }
    }
    seen.len()
}

/// Objects whose property set could not be attributed to a current revision.
/// Used by `inspect` to report coverage without inventing data.
pub fn count_blob_padding(store: &Store) -> BTreeMap<usize, usize> {
    let mut out = BTreeMap::new();
    for (_, obj) in store.all_objects() {
        let pad = obj.blob_size.saturating_sub(obj.consumed);
        *out.entry(pad).or_insert(0) += 1;
    }
    out
}

/// Object ids referenced from a property set that do not exist in the same
/// object space. A well-formed file has none.
pub fn dangling_references(store: &Store) -> BTreeSet<(Guid, u32)> {
    let mut out = BTreeSet::new();
    for space in &store.spaces {
        for obj in space.objects.values() {
            for (_, value) in &obj.props.properties {
                // Object id 0 is written as "no object" in some properties.
                let resolves = |id: &u32| {
                    *id == 0
                        || space.objects.contains_key(id)
                        || space.file_data_refs.contains_key(id)
                };
                match value {
                    prop::Value::Oid(id) => {
                        if !resolves(id) {
                            out.insert((space.gosid.guid, *id));
                        }
                    }
                    prop::Value::Oids(ids) => {
                        for id in ids {
                            if !resolves(id) {
                                out.insert((space.gosid.guid, *id));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

/// Text carried by an object: the current declaration's own text, else the
/// text of a superseded declaration of the same object.
pub fn object_text(obj: &Object) -> Option<String> {
    if let Some(bytes) = obj.props.bytes(prop::pid::RICH_TEXT_UNICODE) {
        let text = prop::decode_utf16(bytes);
        if !text.trim().is_empty() {
            return Some(text);
        }
    }
    if let Some(bytes) = obj.props.bytes(prop::pid::TEXT_EXTENDED_ASCII) {
        let text = prop::decode_extended_ascii(bytes);
        if !text.trim().is_empty() {
            return Some(text);
        }
    }
    obj.text_variants
        .iter()
        .find(|text| !text.trim().is_empty())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filenode::{id as fnid, FileNode, FileNodeList};

    fn revision_list(odcs_default: u16) -> (Vec<u8>, FileNodeList) {
        let mut file = vec![0u8; 64];
        // rid(20) | ridDependent(20) | RevisionRole(4) | odcsDefault(2) at offset 8.
        file[8 + 40..8 + 44].copy_from_slice(&1u32.to_le_bytes());
        file[8 + 44..8 + 46].copy_from_slice(&odcs_default.to_le_bytes());
        let list = FileNodeList {
            id: 0x20,
            fragments: 1,
            truncated: None,
            nodes: vec![FileNode {
                id: fnid::REVISION_MANIFEST_START_6,
                size: 50,
                at: 4,
                stp_format: 0,
                cb_format: 0,
            }],
        };
        (file, list)
    }

    #[test]
    fn encrypted_revisions_are_flagged_and_their_groups_dropped() {
        let (file, list) = revision_list(0x0002);
        let revisions = split_revisions(&file, &list).expect("parses");
        assert_eq!(revisions.len(), 1);
        assert!(revisions[0].encrypted);
        assert!(revisions[0].group_refs.is_empty());
    }

    #[test]
    fn clear_revisions_are_not_flagged() {
        let (file, list) = revision_list(0x0000);
        let revisions = split_revisions(&file, &list).expect("parses");
        assert_eq!(revisions.len(), 1);
        assert!(!revisions[0].encrypted);
    }
}
