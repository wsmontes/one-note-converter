//! Translate a parsed [MS-FSSHTTPB] package into the object model the
//! renderer already consumes.
//!
//! A packaged section stores each object twice: a metadata partition (4) whose
//! first word is the object's `JCID`, and an object-data partition (1) holding
//! the same `ObjectSpaceObjectPropSet` a plain revision store would. Object
//! identities are extended GUIDs, while the property set streams hold local
//! indices; the real references come from the object group's arrays, so the
//! streams are remapped before the property set is decoded.

use crate::fsshttpb::{
    self, CellId, Declaration as ElementDeclaration, ExGuid, GroupObject, Package,
};
use crate::guid::{ExtendedGuid, Guid};
use crate::header::Header;
use crate::prop;
use crate::reader::{u32_at, ParseResult};
use crate::store::{FileDataRef, Object, ObjectSpace, RootObject, Store};
use std::collections::{BTreeMap, HashMap};

const ROOT_ROLE_GUID: &str = "{4A3717F8-1C14-49E7-9526-81D942DE1741}";
const PARTITION_OBJECT_DATA: u64 = 1;
const PARTITION_METADATA: u64 = 4;

struct ObjectData<'a> {
    data: &'a [u8],
    objects: &'a [ExGuid],
    cells: &'a [CellId],
    blob: Option<ExGuid>,
}

/// Build a `Store` from a package store file.
pub fn load_store(data: &[u8], header: Header, package: &Package) -> ParseResult<Store> {
    let root_role_guid =
        Guid::parse(ROOT_ROLE_GUID).ok_or_else(|| "internal: bad root role guid".to_string())?;
    let mut ids: HashMap<ExGuid, u32> = HashMap::new();
    let mut next_id: u32 = 1;
    let mut blobs: BTreeMap<Guid, Vec<u8>> = BTreeMap::new();
    let mut warnings: Vec<String> = Vec::new();

    for (id, bytes) in &package.blobs {
        blobs.insert(id.guid, bytes.clone());
    }

    // Every declared object gets a stable small id so the renderer can keep
    // using u32 identities inside one object space.
    for (_, group) in &package.object_groups {
        for declaration in &group.declarations {
            let object_id = match declaration {
                ElementDeclaration::Object { object_id, .. }
                | ElementDeclaration::Blob { object_id, .. } => *object_id,
            };
            ids.entry(object_id).or_insert_with(|| {
                let id = next_id;
                next_id += 1;
                id
            });
        }
    }

    let Some(storage_index) = package.storage_index.as_ref() else {
        return Err("package has no storage index".to_string());
    };

    let mut spaces = Vec::new();
    let trace = std::env::var_os("ONENOTE_TRACE").is_some();
    for (cell, mapping_id, serial) in &storage_index.cell_mappings {
        let _ = serial;
        let Some(revision_id) = resolve_revision(package, storage_index, *mapping_id) else {
            if trace {
                eprintln!(
                    "no revision for cell ({:?},{:?}) mapping {:?}",
                    cell.0, cell.1, mapping_id
                );
            }
            warnings.push(format!("cell {:?} has no revision manifest", cell.0));
            continue;
        };
        if trace {
            eprintln!(
                "cell space {:?}/{}, revision {:?}",
                cell.1.guid, cell.1.value, revision_id
            );
        }

        let mut declarations: HashMap<(ExGuid, u64), ObjectData<'_>> = HashMap::new();
        let mut roots: Vec<(u32, ExGuid)> = Vec::new();
        let mut revision = Some(revision_id);
        let mut depth = 0;
        while let Some(id) = revision {
            if depth > 64 {
                warnings.push("revision chain too deep".to_string());
                break;
            }
            depth += 1;
            let Some(manifest) = package.revision_manifests.iter().find(|m| m.id == id) else {
                break;
            };
            for (role_id, object_id) in &manifest.roots {
                if role_id.guid == root_role_guid && role_id.value >= 1 && role_id.value <= 4 {
                    roots.push((role_id.value, *object_id));
                }
            }
            for group_id in &manifest.groups {
                let Some(group) = package.find_group(*group_id) else {
                    if trace {
                        eprintln!("  missing group {group_id:?}");
                    }
                    warnings.push(format!("missing object group {group_id:?}"));
                    continue;
                };
                if trace {
                    eprintln!(
                        "  group {group_id:?}: {} declarations, {} objects",
                        group.declarations.len(),
                        group.objects.len()
                    );
                    for (index, (declaration, object)) in group
                        .declarations
                        .iter()
                        .zip(group.objects.iter())
                        .enumerate()
                    {
                        let partition = match declaration {
                            ElementDeclaration::Object {
                                partition_id,
                                data_size,
                                ..
                            } => {
                                format!("obj partition {partition_id} size {data_size}")
                            }
                            ElementDeclaration::Blob { partition_id, .. } => {
                                format!("blob partition {partition_id}")
                            }
                        };
                        let kind = match object {
                            GroupObject::Data { objects, data, .. } => {
                                format!("data(ids {} bytes {})", objects.len(), data.len())
                            }
                            GroupObject::Excluded { size, .. } => format!("excluded {size}"),
                            GroupObject::BlobReference { .. } => "blobref".to_string(),
                        };
                        if index < 6 {
                            if let GroupObject::Data { data, .. } = object {
                                eprintln!(
                                    "    [{index}] first bytes {:02x?}",
                                    &data[..data.len().min(14)]
                                );
                            }
                        }
                        if index < 4 {
                            eprintln!("    [{index}] {partition} / {kind}");
                        }
                    }
                }
                collect_group(group, &mut declarations);
            }
            revision = if manifest.base_id.is_nil() {
                None
            } else {
                resolve_revision(package, storage_index, manifest.base_id)
            };
        }

        let gosid = ExtendedGuid {
            guid: cell.1.guid,
            n: cell.1.value,
        };
        let mut space = ObjectSpace {
            gosid,
            ..ObjectSpace::default()
        };

        for (object_id, metadata) in declarations
            .iter()
            .filter(|((_, p), _)| *p == PARTITION_METADATA)
        {
            let Some(data_decl) = declarations.get(&(object_id.0, PARTITION_OBJECT_DATA)) else {
                warnings.push(format!(
                    "object {:?} has metadata but no data",
                    object_id.0.guid
                ));
                continue;
            };
            if metadata.data.len() < 4 {
                warnings.push(format!(
                    "object {:?} metadata is truncated",
                    object_id.0.guid
                ));
                continue;
            }
            let jcid = u32_at(metadata.data, 0)?;
            let oid = ids.get(&object_id.0).copied().unwrap_or(0);

            let mapped = map_streams(data_decl, &mut ids, &mut next_id)?;
            let parsed = prop::parse_object_prop_set(&mapped, 0)?;
            let object = Object {
                oid,
                jcid,
                props: parsed.properties,
                blob_size: mapped.len(),
                consumed: parsed.consumed,
                blob_offset: 0,
                parse_error: parsed.error,
                text_variants: Vec::new(),
            };
            space.objects.insert(oid, object);
        }

        // Blob declarations carry embedded file payloads (pictures, attachments).
        for ((object_id, _partition), declaration) in &declarations {
            if let Some(blob) = declaration.blob {
                if let Some(oid) = ids.get(object_id).copied() {
                    let extension = blobs
                        .get(&blob.guid)
                        .map(|bytes| sniff_extension(bytes))
                        .unwrap_or_default();
                    space.file_data_refs.insert(
                        oid,
                        FileDataRef {
                            oid,
                            jcid: 0,
                            reference: format!("<ifndf>{blob:?}"),
                            extension,
                        },
                    );
                }
            }
        }

        for (role, object_id) in roots {
            if let Some(oid) = ids.get(&object_id).copied() {
                if space.objects.contains_key(&oid) {
                    space.roots.push(RootObject { role, oid });
                }
            }
        }
        {
            let mut seen = std::collections::HashSet::new();
            space
                .roots
                .retain(|root| seen.insert((root.role, root.oid)));
        }
        // Cross-space references: each referenced cell names another space.
        for declaration in declarations.values() {
            for cell in declaration.cells {
                if cell.1 == gosid_exguid(&gosid) {
                    continue;
                }
                if let Some(guid) = Guid::parse(&cell.1.guid.to_string()) {
                    space.gid_table.insert(0, guid);
                }
            }
        }
        spaces.push(space);
    }

    if spaces.is_empty() {
        return Err("package contains no object spaces".to_string());
    }

    Ok(Store {
        header,
        spaces,
        file_data: BTreeMap::new(),
        blobs,
        file_size: data.len(),
        warnings,
        list_counts: HashMap::new(),
        hashed_chunks: Vec::new(),
        transactions: 0,
    })
}

fn gosid_exguid(gosid: &ExtendedGuid) -> ExGuid {
    ExGuid {
        guid: gosid.guid,
        value: gosid.n,
    }
}

fn collect_group<'a>(
    group: &'a fsshttpb::ObjectGroup,
    out: &mut HashMap<(ExGuid, u64), ObjectData<'a>>,
) {
    for (declaration, object) in group.declarations.iter().zip(group.objects.iter()) {
        let (partition, object_id) = match declaration {
            ElementDeclaration::Object {
                object_id,
                partition_id,
                ..
            } => (*partition_id, *object_id),
            ElementDeclaration::Blob {
                object_id,
                partition_id,
                ..
            } => (*partition_id, *object_id),
        };
        match object {
            GroupObject::Data {
                objects,
                cells,
                data,
            } => {
                out.insert(
                    (object_id, partition),
                    ObjectData {
                        data,
                        objects,
                        cells,
                        blob: None,
                    },
                );
            }
            GroupObject::BlobReference {
                objects,
                cells,
                blob,
            } => {
                out.insert(
                    (object_id, partition),
                    ObjectData {
                        data: &[],
                        objects,
                        cells,
                        blob: Some(*blob),
                    },
                );
            }
            GroupObject::Excluded { .. } => {}
        }
    }
}

/// Resolve a data element id to a revision manifest: the id may be the
/// revision itself, a cell manifest pointing at one, or a revision mapping.
fn resolve_revision(
    package: &Package,
    storage_index: &fsshttpb::StorageIndex,
    id: ExGuid,
) -> Option<ExGuid> {
    if package.revision_manifests.iter().any(|m| m.id == id) {
        return Some(id);
    }
    if let Some((_, revision)) = package.cell_manifests.iter().find(|(cell, _)| *cell == id) {
        if package.revision_manifests.iter().any(|m| m.id == *revision) {
            return Some(*revision);
        }
    }
    for (mapping, revision, _) in &storage_index.revision_mappings {
        if *mapping == id && package.revision_manifests.iter().any(|m| m.id == *revision) {
            return Some(*revision);
        }
    }
    None
}

/// Rewrite the object-id stream of a property set so its entries are the small
/// object ids the renderer uses. Stream entries appear in the same order as the
/// group data's reference array, so the mapping is positional.
fn map_streams<'a>(
    declaration: &ObjectData<'a>,
    ids: &mut HashMap<ExGuid, u32>,
    next_id: &mut u32,
) -> ParseResult<Vec<u8>> {
    let mut blob = declaration.data.to_vec();
    let mut object_refs: Vec<u32> = Vec::with_capacity(declaration.objects.len());
    for id in declaration.objects {
        let value = *ids.entry(*id).or_insert_with(|| {
            let value = *next_id;
            *next_id += 1;
            value
        });
        object_refs.push(value);
    }

    if blob.len() >= 4 {
        let header = u32_at(&blob, 0)?;
        let oid_count = (header & 0xFF_FFFF) as usize;
        // The group data may describe fewer references than the stream holds;
        // entries without a description keep their original value.
        for (index, value) in object_refs.iter().enumerate().take(oid_count) {
            let at = 4 + index * 4;
            if at + 4 <= blob.len() {
                blob[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
    }
    Ok(blob)
}

/// Guess a file extension from the first bytes of a stored payload.
fn sniff_extension(bytes: &[u8]) -> String {
    let extension = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "gif"
    } else if bytes.starts_with(b"BM") {
        "bmp"
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        "tif"
    } else if bytes.starts_with(b"%PDF") {
        "pdf"
    } else if bytes.starts_with(b"PK\x03\x04") {
        // XPS and OOXML documents are both zip containers.
        "zip"
    } else {
        return String::new();
    };
    format!(".{extension}")
}
