//! Notebook table of contents (`.onetoc2`) walking.
//!
//! A TOC is a revision store whose root object is a
//! `jcidPersistablePropertyContainerForTOC`. Each entry lists its children
//! through `TOCEntryIndex_OidIndex`; leaves carry the file name of the section
//! they point at. Groups are entries that only have children.

use crate::guid::Guid;
use crate::prop::{self, jcid, pid};
use crate::store::Store;
use std::collections::HashSet;

/// `TOCEntryIndex_OidIndex`: child entries of a notebook node.
const TOC_ENTRY_INDEX: u32 = 0x1CF6;
/// `FileIdentityGuid`: GUID of the section a leaf points at.
const FILE_IDENTITY_GUID: u32 = 0x1D94;
/// Section file name inside the notebook folder.
const SECTION_FILE_NAME: u32 = 0x1D6B;
/// `NotebookElementOrderingID`: position of the entry among its siblings.
const ELEMENT_ORDERING_ID: u32 = 0x1CBE;

#[derive(Debug, Clone, Default)]
pub struct Entry {
    /// Display/file name of the entry, e.g. `Section A.one`.
    pub name: String,
    /// GUID identifying the section, when present.
    pub guid: Option<Guid>,
    /// Ordering hint; entries are sorted by it.
    pub order: u32,
    pub children: Vec<Entry>,
}

impl Entry {
    pub fn is_section(&self) -> bool {
        self.children.is_empty() && self.name.ends_with(".one")
    }

    pub fn is_group(&self) -> bool {
        !self.children.is_empty()
    }
}

/// Read the notebook tree from a table of contents store.
pub fn read(store: &Store) -> Vec<Entry> {
    let mut roots: Vec<u32> = Vec::new();
    for space in &store.spaces {
        for object in space.objects.values() {
            if object.jcid != jcid::TOC_CONTAINER {
                continue;
            }
            // The root node is the one that lists children; leaves elsewhere in
            // the store are reached from it.
            if object.props.oids(TOC_ENTRY_INDEX).is_some() && !roots.contains(&object.oid) {
                roots.push(object.oid);
            }
        }
    }

    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for oid in roots {
        // Prefer root role 1 when the store exposes roots.
        if let Some(entry) = read_entry(store, oid, &mut seen, 0) {
            entries.push(entry);
        }
    }
    sort(&mut entries);
    // The TOC's own root entry is unnamed and only groups the notebook, so its
    // children are the notebook's top level.
    if entries.len() == 1 && entries[0].name.is_empty() && !entries[0].children.is_empty() {
        return entries.remove(0).children;
    }
    entries
}

fn read_entry(store: &Store, oid: u32, seen: &mut HashSet<u32>, depth: usize) -> Option<Entry> {
    if depth > 32 || !seen.insert(oid) {
        return None;
    }
    let (_, object) = store
        .spaces
        .iter()
        .find_map(|space| space.objects.get(&oid).map(|object| (space, object)))?;

    let name = object
        .props
        .bytes(SECTION_FILE_NAME)
        .map(prop::decode_utf16)
        .map(|value| value.trim_matches('\0').trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_default();
    let guid = object
        .props
        .bytes(FILE_IDENTITY_GUID)
        .filter(|bytes| bytes.len() >= 16)
        .and_then(|bytes| Guid::read(bytes, 0).ok());
    let order = match object.props.get(ELEMENT_ORDERING_ID) {
        Some(prop::Value::U32(value)) => *value,
        _ => 0,
    };

    let mut entry = Entry {
        name,
        guid,
        order,
        children: Vec::new(),
    };
    if let Some(children) = object.props.oids(TOC_ENTRY_INDEX) {
        for child in children {
            if let Some(child_entry) = read_entry(store, *child, seen, depth + 1) {
                entry.children.push(child_entry);
            }
        }
    }
    // An entry with neither a name nor children carries nothing.
    if entry.name.is_empty() && entry.children.is_empty() {
        return None;
    }
    sort(&mut entry.children);
    Some(entry)
}

fn sort(entries: &mut [Entry]) {
    entries.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.name.cmp(&b.name)));
}

/// Human-readable tree, used by `inspect` on a table of contents.
pub fn describe(entries: &[Entry], depth: usize, out: &mut Vec<String>) {
    for entry in entries {
        let kind = if entry.is_group() { "group" } else { "section" };
        out.push(format!(
            "{}{} ({kind}){}",
            "  ".repeat(depth),
            if entry.name.is_empty() {
                "<unnamed>"
            } else {
                &entry.name
            },
            match entry.guid {
                Some(guid) => format!(" [{guid}]"),
                None => String::new(),
            }
        ));
        describe(&entry.children, depth + 1, out);
    }
}

/// Keep `pid` imported for readers of this module.
#[allow(dead_code)]
fn _pid_marker() -> u32 {
    pid::NOTE_TAG_LABEL
}

use std::path::{Path, PathBuf};

use crate::output::{self, Section};

/// One section of a notebook, resolved to its file.
#[derive(Debug, Clone)]
pub struct NotebookSection {
    pub name: String,
    pub file: PathBuf,
}

/// A notebook level: its folder, the table of contents that describes it, the
/// sections it lists and the groups below it.
#[derive(Debug, Clone, Default)]
pub struct Notebook {
    pub folder: PathBuf,
    pub toc: Option<PathBuf>,
    pub sections: Vec<NotebookSection>,
    pub groups: Vec<Notebook>,
    /// Entries the table of contents lists whose file or folder is missing.
    pub errors: Vec<String>,
}

impl Notebook {
    /// Read the notebook tree rooted at `toc`.
    pub fn read(toc: &Path) -> Result<Notebook, String> {
        let folder = toc
            .parent()
            .map(Path::to_path_buf)
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| PathBuf::from("."));
        Ok(read_level(&folder, Some(toc), 0))
    }

    /// Name of the notebook folder.
    pub fn name(&self) -> String {
        self.folder
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    /// Every section in the tree, depth first.
    pub fn all_sections(&self) -> Vec<NotebookSection> {
        let mut out = self.sections.clone();
        for group in &self.groups {
            out.extend(group.all_sections());
        }
        out
    }
}

fn read_level(folder: &Path, toc: Option<&Path>, depth: usize) -> Notebook {
    let mut notebook = Notebook {
        folder: folder.to_path_buf(),
        toc: toc
            .map(Path::to_path_buf)
            .or_else(|| output::find_toc(folder)),
        ..Notebook::default()
    };
    if depth > 8 {
        return notebook;
    }
    let Some(toc_path) = notebook.toc.clone() else {
        return notebook;
    };
    let entries = match std::fs::read(&toc_path) {
        Ok(bytes) => match crate::load(&bytes) {
            Ok((_header, store)) => read(&store),
            Err(error) => {
                notebook
                    .errors
                    .push(format!("{}: {error}", toc_path.display()));
                Vec::new()
            }
        },
        Err(error) => {
            notebook
                .errors
                .push(format!("{}: {error}", toc_path.display()));
            Vec::new()
        }
    };
    for entry in entries {
        if entry.is_section() {
            if output::safe_toc_name(&entry.name).is_none() {
                notebook
                    .errors
                    .push(format!("unsafe section name in TOC: {:?}", entry.name));
                continue;
            }
            let file = folder.join(&entry.name);
            if !file.is_file() {
                notebook
                    .errors
                    .push(format!("{}: section file is missing", file.display()));
                continue;
            }
            notebook.sections.push(NotebookSection {
                name: entry.name.clone(),
                file,
            });
            continue;
        }
        let name = if entry.name.is_empty() {
            "Notebook".to_string()
        } else {
            entry.name.trim_end_matches(".one").to_string()
        };
        if output::safe_toc_name(&name).is_none() {
            notebook
                .errors
                .push(format!("unsafe group name in TOC: {name:?}"));
            continue;
        }
        let source = folder.join(&name);
        if !source.is_dir() {
            notebook
                .errors
                .push(format!("{}: group folder is missing", source.display()));
            continue;
        }
        notebook.groups.push(read_level(&source, None, depth + 1));
    }
    // Groups the table of contents does not mention but that exist on disk, so
    // the tree matches what `write_tree` converts.
    for subfolder in output::subfolders_with_toc(folder) {
        if notebook
            .groups
            .iter()
            .any(|group| group.folder == subfolder)
        {
            continue;
        }
        notebook
            .groups
            .push(read_level(&subfolder, None, depth + 1));
    }
    notebook
}

/// What one written section produced.
#[derive(Debug, Clone)]
pub struct SectionReport {
    pub input: PathBuf,
    pub pages: usize,
    pub assets: usize,
    pub failures: Vec<String>,
    pub warnings: Vec<String>,
}

/// Result of writing a notebook tree.
#[derive(Debug, Clone, Default)]
pub struct TreeStats {
    pub converted: usize,
    pub sections: Vec<SectionReport>,
    pub failures: Vec<String>,
    pub warnings: Vec<String>,
}

/// Write a notebook tree: one `.md` per section, `<section>_assets` for its
/// pictures, and an `index.md` per level that lists the sections in notebook
/// order followed by the groups.
pub fn write_tree(
    base: &Path,
    out: &Path,
    front_matter: bool,
    toc: Option<&Path>,
) -> Result<TreeStats, String> {
    write_tree_with_options(base, out, front_matter, toc, false)
}

/// Optional per-page export, preserving one Markdown front matter per file.
pub fn write_tree_with_options(
    base: &Path,
    out: &Path,
    front_matter: bool,
    toc: Option<&Path>,
    split_pages: bool,
) -> Result<TreeStats, String> {
    let mut stats = TreeStats::default();
    stats.converted = write_level(base, out, 0, front_matter, toc, split_pages, &mut stats)?;
    Ok(stats)
}

fn write_level(
    folder: &Path,
    out: &Path,
    depth: usize,
    front_matter: bool,
    toc: Option<&Path>,
    split_pages: bool,
    stats: &mut TreeStats,
) -> Result<usize, String> {
    if depth > 8 {
        return Ok(0);
    }
    std::fs::create_dir_all(out)
        .map_err(|error| format!("cannot create {}: {error}", out.display()))?;
    let mut converted = 0usize;
    let mut index = String::new();
    let mut handled: Vec<PathBuf> = Vec::new();
    let mut written_sections = HashSet::new();

    let toc = match toc {
        Some(path) => Some(path.to_path_buf()),
        None => output::find_toc(folder),
    };
    let entries = match toc {
        Some(path) => match std::fs::read(&path) {
            Ok(bytes) => match crate::load(&bytes) {
                Ok((_header, store)) => read(&store),
                Err(error) => {
                    stats.failures.push(format!("{}: {error}", path.display()));
                    Vec::new()
                }
            },
            Err(error) => {
                stats.failures.push(format!("{}: {error}", path.display()));
                Vec::new()
            }
        },
        None => Vec::new(),
    };

    for entry in &entries {
        if entry.is_section() {
            if output::safe_toc_name(&entry.name).is_none() {
                stats
                    .failures
                    .push(format!("unsafe section name in TOC: {:?}", entry.name));
                continue;
            }
            let stem = entry.name.trim_end_matches(".one");
            if !written_sections.insert(stem.to_lowercase()) {
                stats
                    .failures
                    .push(format!("duplicate output section name: {stem:?}"));
                continue;
            }
            let destination = if split_pages {
                format!("{stem}/index.md")
            } else {
                format!("{stem}.md")
            };
            index.push_str(&format!(
                "- [{stem}]({})\n",
                output::escape_link(&destination)
            ));
            let input = folder.join(&entry.name);
            let assets = out.join(format!("{stem}_assets"));
            let section_output = out.join(format!("{stem}.md"));
            let bytes = match std::fs::read(&input) {
                Ok(bytes) => bytes,
                Err(error) => {
                    stats.failures.push(format!("{}: {error}", input.display()));
                    continue;
                }
            };
            match Section::read(bytes).and_then(|section| {
                if split_pages {
                    section.write_pages(&out.join(stem), front_matter)
                } else {
                    section.write(&section_output, &assets, front_matter)
                }
            }) {
                Ok(written) => {
                    converted += 1;
                    stats.warnings.extend(
                        written
                            .warnings
                            .iter()
                            .map(|warning| format!("{}: {warning}", input.display())),
                    );
                    stats.failures.extend(
                        written
                            .failures
                            .iter()
                            .map(|error| format!("{}: {error}", input.display())),
                    );
                    stats.sections.push(SectionReport {
                        input,
                        pages: written.pages,
                        assets: written.assets,
                        failures: written.failures,
                        warnings: written.warnings,
                    });
                }
                Err(error) => stats.failures.push(format!("{}: {error}", input.display())),
            }
            continue;
        }

        let name = if entry.name.is_empty() {
            "Notebook".to_string()
        } else {
            entry.name.trim_end_matches(".one").to_string()
        };
        if output::safe_toc_name(&name).is_none() {
            stats
                .failures
                .push(format!("unsafe group name in TOC: {name:?}"));
            continue;
        }
        let source = folder.join(&name);
        if !source.is_dir() {
            // A partial notebook must fail, not quietly convert the rest.
            stats
                .failures
                .push(format!("{}: group folder is missing", source.display()));
            continue;
        }
        if output::find_toc(&source).is_some() {
            handled.push(source.clone());
        }
        converted += write_level(
            &source,
            &out.join(&name),
            depth + 1,
            front_matter,
            None,
            split_pages,
            stats,
        )?;
        index.push_str(&format!(
            "- **[{name}]({}/index.md)**\n",
            output::escape_link(&name)
        ));
    }

    // Groups the table of contents does not mention but that exist on disk.
    for subfolder in output::subfolders_with_toc(folder) {
        if handled.iter().any(|path| path == &subfolder) {
            continue;
        }
        let name = subfolder
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_default();
        converted += write_level(
            &subfolder,
            &out.join(&name),
            depth + 1,
            front_matter,
            None,
            split_pages,
            stats,
        )?;
        index.push_str(&format!(
            "- **[{name}]({}/index.md)**\n",
            output::escape_link(&name)
        ));
    }

    std::fs::write(out.join("index.md"), index)
        .map_err(|error| format!("cannot write {}/index.md: {error}", out.display()))?;
    Ok(converted)
}

/// Convert all sections in a CAB without a root table of contents, preserving
/// subfolder structure. Shared by the CLI and Python surfaces.
pub fn write_unindexed(
    source: &Path,
    out: &Path,
    front_matter: bool,
    split_pages: bool,
) -> Result<TreeStats, String> {
    let mut stats = TreeStats::default();
    fn visit(
        source: &Path,
        current: &Path,
        out: &Path,
        front_matter: bool,
        split_pages: bool,
        depth: usize,
        stats: &mut TreeStats,
        seen_outputs: &mut HashSet<String>,
    ) -> Result<(), String> {
        if depth > 32 {
            stats.failures.push(format!(
                "package nesting exceeds 32 levels: {}",
                current.display()
            ));
            return Ok(());
        }
        let mut entries = std::fs::read_dir(current)
            .map_err(|error| format!("{}: {error}", current.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("{}: {error}", current.display()))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let ty = entry
                .file_type()
                .map_err(|error| format!("{}: {error}", path.display()))?;
            if ty.is_symlink() {
                stats
                    .failures
                    .push(format!("symlink in extracted package: {}", path.display()));
                continue;
            }
            if ty.is_dir() {
                visit(
                    source,
                    &path,
                    out,
                    front_matter,
                    split_pages,
                    depth + 1,
                    stats,
                    seen_outputs,
                )?;
                continue;
            }
            if !ty.is_file() || path.extension().is_none_or(|ext| ext != "one") {
                continue;
            }
            let relative = path
                .parent()
                .unwrap_or(source)
                .strip_prefix(source)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            let stem = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if output::safe_toc_name(&stem).is_none() {
                stats
                    .failures
                    .push(format!("unsafe section name: {stem:?}"));
                continue;
            }
            let target = out.join(relative).join(&stem);
            let unique_key = target.to_string_lossy().to_lowercase();
            if !seen_outputs.insert(unique_key) {
                stats
                    .failures
                    .push(format!("colliding section output: {}", target.display()));
                continue;
            }
            let bytes =
                std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
            let result = Section::read(bytes).and_then(|section| {
                if split_pages {
                    section.write_pages(&target, front_matter)
                } else {
                    section.write(
                        &target.join(format!("{stem}.md")),
                        &target.join("assets"),
                        front_matter,
                    )
                }
            });
            match result {
                Ok(written) => {
                    stats.converted += 1;
                    stats.failures.extend(
                        written
                            .failures
                            .iter()
                            .map(|failure| format!("{}: {failure}", path.display())),
                    );
                    stats.warnings.extend(
                        written
                            .warnings
                            .iter()
                            .map(|warning| format!("{}: {warning}", path.display())),
                    );
                    stats.sections.push(SectionReport {
                        input: path,
                        pages: written.pages,
                        assets: written.assets,
                        failures: written.failures,
                        warnings: written.warnings,
                    });
                }
                Err(error) => stats.failures.push(format!("{}: {error}", path.display())),
            }
        }
        Ok(())
    }
    visit(
        source,
        source,
        out,
        front_matter,
        split_pages,
        0,
        &mut stats,
        &mut HashSet::new(),
    )?;
    if stats.converted == 0 && stats.failures.is_empty() {
        return Err("package contains no sections".into());
    }
    Ok(stats)
}
