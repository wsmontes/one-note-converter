//! Semantic walk from the revision store graph to a page/block document model.
//!
//! A page space has exactly one content root, the role-1 root object, which is
//! a `jcidPageManifestNode`. Children are reached through any of the child
//! collections the format uses (`ElementChildNodes`, `ContentChildNodes`,
//! `StructureElementChildNodes`); the walker does not try to guess which one a
//! given object type uses.

use crate::guid::Guid;
use crate::prop::{self, jcid, pid};
use crate::store::{object_text, Object, ObjectSpace, SpaceKind, Store};
use std::collections::HashSet;

/// A picture reference. `guid` is set when the bytes live in this file's file
/// data store, `sibling` when they live in a `onefiles` folder next to it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AssetRef {
    pub alt: String,
    pub filename: String,
    pub extension: String,
    pub guid: Option<Guid>,
    pub sibling: Option<String>,
    /// When the picture itself carries a hyperlink.
    pub link: Option<String>,
}

/// Cell content: text and pictures, in document order.
#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text(String),
    Image(AssetRef),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Paragraph {
        text: String,
        level: u8,
    },
    Bullet {
        text: String,
        level: u8,
        /// Marker written before the item, e.g. `-` or `3.`.
        marker: String,
    },
    Image(AssetRef),
    Attachment(AssetRef),
    /// Content that exists but is not rendered (ink strokes).
    Note(String),
    Table {
        rows: Vec<Vec<Vec<Inline>>>,
    },
}

impl Block {
    fn shift_level(&mut self, base: u8) {
        match self {
            Block::Paragraph { level, .. } | Block::Bullet { level, .. } => {
                *level = level.saturating_sub(base);
            }
            _ => {}
        }
    }

    fn level(&self) -> Option<u8> {
        match self {
            Block::Paragraph { level, .. } | Block::Bullet { level, .. } => Some(*level),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Page {
    pub title: String,
    pub blocks: Vec<Block>,
    pub space_index: usize,
    /// Oid of the page manifest that produced this page.
    pub oid: u32,
    /// Creation time in seconds since the Unix epoch, when the file has one.
    pub created: Option<u64>,
    /// Last modification time in seconds since the Unix epoch.
    pub modified: Option<u64>,
    /// Distinct author names attached to the page.
    pub authors: Vec<String>,
    /// OneNote page level (top-level pages are 1).
    pub level: Option<u32>,
}

const MAX_DEPTH: usize = 64;

pub fn extract(store: &Store) -> Vec<Page> {
    page_order(store)
        .into_iter()
        .map(|(space_index, oid)| render_page(&store.spaces[space_index], space_index, oid))
        .collect()
}

/// Pages in section order: the section's page series names the page object
/// spaces, so its order wins. Any page space the series does not mention is
/// appended in file order.
fn page_order(store: &Store) -> Vec<(usize, u32)> {
    let mut ordered = Vec::new();
    let mut seen = HashSet::new();

    for space in &store.spaces {
        if space.kind() != SpaceKind::Section {
            continue;
        }
        let Some(section_root) = space.content_root() else {
            continue;
        };
        for series_oid in subtree_matching(space, section_root, jcid::PAGE_SERIES_NODE) {
            let Some(series) = space.objects.get(&series_oid) else {
                continue;
            };
            let Some(osids) = series.props.oids(pid::CHILD_GRAPH_SPACE_ELEMENT_NODES) else {
                continue;
            };
            for compact in osids {
                let Some(target) = store.resolve_space(space, *compact) else {
                    continue;
                };
                let page_space = &store.spaces[target];
                if page_space.kind() != SpaceKind::Page {
                    continue;
                }
                if let Some(root) = page_space.content_root() {
                    if seen.insert((target, root)) {
                        ordered.push((target, root));
                    }
                }
            }
        }
    }

    for (index, space) in store.spaces.iter().enumerate() {
        if space.kind() != SpaceKind::Page {
            continue;
        }
        if let Some(root) = space.content_root() {
            if seen.insert((index, root)) {
                ordered.push((index, root));
            }
        }
    }

    // Last resort for files whose roots do not resolve: any page node found in
    // an otherwise unclassified space.
    if ordered.is_empty() {
        for (index, space) in store.spaces.iter().enumerate() {
            if space.kind() == SpaceKind::Section {
                continue;
            }
            for object in space.objects.values() {
                let target = if object.jcid == jcid::PAGE_NODE {
                    Some(object.oid)
                } else {
                    None
                };
                if let Some(oid) = target {
                    if seen.insert((index, oid)) {
                        ordered.push((index, oid));
                    }
                }
            }
        }
    }
    ordered
}

/// Object ids listed by one child collection property.
fn child_oids(obj: &Object, id: u32) -> Vec<u32> {
    obj.props.oids(id).map(|v| v.to_vec()).unwrap_or_default()
}

/// Every child collection used by the format, deduplicated, in a stable order.
fn all_children(obj: &Object) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    for id in [
        pid::ELEMENT_CHILD_NODES,
        pid::CONTENT_CHILD_NODES,
        pid::STRUCTURE_ELEMENT_CHILD_NODES,
    ] {
        if let Some(ids) = obj.props.oids(id) {
            for oid in ids {
                if !out.contains(oid) {
                    out.push(*oid);
                }
            }
        }
    }
    out
}

/// Object ids reachable from `root` (inclusive) whose jcid matches.
fn subtree_matching(space: &ObjectSpace, root: u32, wanted: u32) -> Vec<u32> {
    let mut found = Vec::new();
    let mut visited = HashSet::new();
    let mut stack = vec![(root, 0usize)];
    while let Some((oid, depth)) = stack.pop() {
        if depth > MAX_DEPTH || !visited.insert(oid) {
            continue;
        }
        let Some(obj) = space.objects.get(&oid) else {
            continue;
        };
        if obj.jcid == wanted {
            found.push(oid);
            continue;
        }
        for child in all_children(obj) {
            stack.push((child, depth + 1));
        }
    }
    found
}

/// The title node of a page, plus the text of its first paragraph.
fn find_title(space: &ObjectSpace, roots: &[u32]) -> Option<(u32, String)> {
    let mut stack: Vec<(u32, usize)> = roots.iter().rev().map(|oid| (*oid, 0usize)).collect();
    let mut visited = HashSet::new();
    while let Some((oid, depth)) = stack.pop() {
        if depth > MAX_DEPTH || !visited.insert(oid) {
            continue;
        }
        let Some(obj) = space.objects.get(&oid) else {
            continue;
        };
        if obj.jcid == jcid::TITLE_NODE {
            let mut seen = HashSet::new();
            let title = first_text(space, oid, &mut seen, depth + 1).unwrap_or_default();
            return Some((oid, title));
        }
        for child in all_children(obj).into_iter().rev() {
            stack.push((child, depth + 1));
        }
    }
    None
}

fn render_page(space: &ObjectSpace, space_index: usize, manifest_oid: u32) -> Page {
    let mut page = Page {
        space_index,
        oid: manifest_oid,
        ..Page::default()
    };
    let Some(manifest) = space.objects.get(&manifest_oid) else {
        page.title = "Untitled page".to_string();
        return page;
    };
    let roots = all_children(manifest);

    let mut visited = HashSet::new();
    let title = find_title(space, &roots);
    if let Some((_title_oid, text)) = &title {
        page.title = text.clone();
    }
    if page.title.trim().is_empty() {
        page.title = cached_title(space);
    }

    for root in &roots {
        walk(space, *root, 0, &mut page.blocks, &mut visited, 0);
    }
    if page.title.trim().is_empty() {
        page.title = "Untitled page".to_string();
    } else {
        // The title element also lives in the tree; drop its paragraph once the
        // title itself carries it.
        let wanted = page.title.trim().to_string();
        if let Some(index) = page.blocks.iter().position(|block| match block {
            Block::Paragraph { text, .. } | Block::Bullet { text, .. } => text.trim() == wanted,
            _ => false,
        }) {
            page.blocks.remove(index);
        }
    }

    // OutlineElementChildLevel is 1-based inside its outline; normalize so the
    // shallowest paragraph of the page sits at level 0.
    if let Some(base) = page.blocks.iter().filter_map(Block::level).min() {
        for block in &mut page.blocks {
            block.shift_level(base);
        }
    }
    let metadata = page_metadata(space, &roots);
    page.created = metadata.created;
    page.modified = metadata.modified;
    page.authors = metadata.authors;
    page.level = metadata.level;
    page
}

#[derive(Default)]
struct PageMeta {
    created: Option<u64>,
    modified: Option<u64>,
    authors: Vec<String>,
    level: Option<u32>,
}

/// Collect dates, authors and the page level from a page's subtree.
fn page_metadata(space: &ObjectSpace, roots: &[u32]) -> PageMeta {
    let mut meta = PageMeta::default();
    let mut visited = HashSet::new();
    let mut stack: Vec<(u32, usize)> = roots.iter().map(|oid| (*oid, 0usize)).collect();
    let mut created_seconds: Option<u64> = None;
    let mut topology: Option<u64> = None;
    let mut page_node_created: Option<u64> = None;
    let mut page_node_modified: Option<u64> = None;

    while let Some((oid, depth)) = stack.pop() {
        if depth > MAX_DEPTH || !visited.insert(oid) {
            continue;
        }
        let Some(obj) = space.objects.get(&oid) else {
            continue;
        };
        let is_page_node = obj.jcid == jcid::PAGE_NODE;
        if let Some(prop::Value::U32(seconds)) = obj.props.get(pid::CREATION_TIME_STAMP) {
            let seconds = *seconds as u64;
            if is_page_node {
                page_node_created = Some(page_node_created.map_or(seconds, |c| c.min(seconds)));
            }
            created_seconds = Some(created_seconds.map_or(seconds, |current| current.min(seconds)));
        }
        // The page node's own `LastModifiedTime` describes the page better than
        // the newest stamp anywhere in its subtree.
        if is_page_node {
            if let Some(prop::Value::U32(seconds)) = obj.props.get(pid::LAST_MODIFIED_TIME) {
                page_node_modified = Some(*seconds as u64);
            }
        }
        if let Some(prop::Value::U64(stamp)) = obj.props.get(pid::LAST_MODIFIED_TIME_STAMP) {
            if let Some(seconds) = filetime_to_unix(*stamp) {
                meta.modified = Some(
                    meta.modified
                        .map_or(seconds, |current| current.max(seconds)),
                );
            }
        }
        if let Some(prop::Value::U64(stamp)) = obj.props.get(pid::TOPOLOGY_CREATION_TIME_STAMP) {
            if let Some(seconds) = filetime_to_unix(*stamp) {
                topology = Some(topology.map_or(seconds, |current| current.min(seconds)));
            }
        }
        if let Some(prop::Value::U32(level)) = obj.props.get(pid::PAGE_LEVEL) {
            meta.level = Some(*level);
        }
        for id in [pid::AUTHOR_ORIGINAL, pid::AUTHOR_MOST_RECENT] {
            if let Some(author_oid) = obj.props.oid(id) {
                if let Some(author) = space.objects.get(&author_oid) {
                    if let Some(name) = author_text(author) {
                        if !meta.authors.contains(&name) {
                            meta.authors.push(name);
                        }
                    }
                }
            }
        }
        for child in all_children(obj) {
            stack.push((child, depth + 1));
        }
    }
    meta.created = page_node_created.or(topology).or(created_seconds);
    meta.modified = page_node_modified.or(meta.modified);
    meta
}

/// `Author` is stored on author containers; older files repeat it as text.
fn author_text(obj: &Object) -> Option<String> {
    obj.props
        .bytes(pid::AUTHOR)
        .map(prop::decode_utf16)
        .map(|value| value.trim_matches('\0').trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Windows FILETIME (100 ns ticks since 1601) to Unix seconds.
fn filetime_to_unix(stamp: u64) -> Option<u64> {
    const EPOCH_DIFFERENCE: u64 = 11_644_473_600;
    let seconds = stamp / 10_000_000;
    seconds.checked_sub(EPOCH_DIFFERENCE)
}

fn cached_title(space: &ObjectSpace) -> String {
    for obj in space.objects.values() {
        for id in [pid::CACHED_TITLE_STRING, pid::CACHED_TITLE_STRING_FROM_PAGE] {
            if let Some(bytes) = obj.props.bytes(id) {
                let text = prop::decode_utf16(bytes);
                let text = text.trim_matches(|c: char| c == '\0' || c.is_whitespace());
                if !text.is_empty() {
                    return text.to_string();
                }
            }
        }
    }
    String::new()
}

fn walk(
    space: &ObjectSpace,
    oid: u32,
    level: u8,
    out: &mut Vec<Block>,
    visited: &mut HashSet<u32>,
    depth: usize,
) {
    let mut counters = std::collections::BTreeMap::new();
    walk_with(space, oid, level, out, visited, depth, &mut counters)
}

/// `counters` tracks numbered-list positions per indentation level; deeper
/// levels restart when a shallower one advances.
#[allow(clippy::too_many_arguments)]
fn walk_with(
    space: &ObjectSpace,
    oid: u32,
    level: u8,
    out: &mut Vec<Block>,
    visited: &mut HashSet<u32>,
    depth: usize,
    counters: &mut std::collections::BTreeMap<u8, u32>,
) {
    if depth > MAX_DEPTH || !visited.insert(oid) {
        return;
    }
    let Some(obj) = space.objects.get(&oid) else {
        return;
    };
    if std::env::var_os("ONENOTE_TRACE").is_some() {
        let snippet = object_text(obj).unwrap_or_default();
        eprintln!(
            "{:indent$}{} {} {:?}",
            "",
            oid,
            crate::prop::jcid_name(obj.jcid),
            snippet.chars().take(30).collect::<String>(),
            indent = depth * 2
        );
    }
    match obj.jcid {
        // The title node holds the page title plus its date/time stamps; page
        // bodies hang off outline nodes that are siblings of the title node, in
        // both the desktop and the packaged layout. So the title contributes
        // only its text, extracted by `find_title`.
        jcid::TITLE_NODE => {}
        jcid::RICH_TEXT_NODE => {
            if let Some(text) = object_text(obj) {
                let text = text.trim_matches(|c: char| c.is_whitespace() || c == '\0');
                if !text.is_empty() {
                    out.push(Block::Paragraph {
                        text: emphasize(space, obj, text),
                        level,
                    });
                }
            }
        }
        jcid::IMAGE_NODE => {
            if let Some(asset) = image_block(space, oid) {
                out.push(Block::Image(asset));
            }
        }
        jcid::EMBEDDED_FILE_NODE => {
            if let Some(asset) = attachment_block(space, oid) {
                out.push(Block::Attachment(asset));
            }
        }
        jcid::INK_CONTAINER => out.push(Block::Note("ink drawing not rendered".to_string())),
        jcid::TABLE_NODE => {
            if let Some(table) = table_block(space, oid) {
                out.push(table);
            }
        }
        jcid::LIST_NODE => {
            let mut text = object_text(obj).unwrap_or_default();
            if text.trim().is_empty() {
                for child in all_children(obj) {
                    collect_text(space, child, &mut text, 0);
                }
            }
            let text = text.trim().to_string();
            if !text.is_empty() {
                let marker = list_marker(obj, level, counters);
                out.push(Block::Bullet {
                    text,
                    level,
                    marker,
                });
            }
        }
        jcid::OUTLINE_NODE => {
            // A new outline starts its own numbering scope.
            counters.clear();
            for child in all_children(obj) {
                walk_with(space, child, level, out, visited, depth + 1, counters);
            }
        }
        jcid::OUTLINE_ELEMENT_NODE => {
            let element_level = obj
                .props
                .u8(pid::OUTLINE_ELEMENT_CHILD_LEVEL)
                .unwrap_or(level);
            let children = all_children(obj);
            // Consecutive rich text children form one paragraph, so gather the
            // text first and then descend into everything that is not text.
            let mut text = String::new();
            for child in &children {
                match space.objects.get(child) {
                    Some(object) if object.jcid == jcid::RICH_TEXT_NODE => {
                        let runs = if std::env::var_os("ONENOTE_NO_RUNS").is_some() {
                            None
                        } else {
                            formatted_text(space, object)
                        };
                        match runs {
                            Some(run) => {
                                // A hyperlink applies to the whole rich text node.
                                let url = object
                                    .props
                                    .bytes(pid::HYPERLINK_URL)
                                    .map(prop::decode_utf16)
                                    .map(|value| value.trim_matches('\0').to_string())
                                    .unwrap_or_default();
                                if url.is_empty() || run.trim().is_empty() {
                                    text.push_str(&run);
                                } else {
                                    text.push_str(&format!("[{}]({url})", run.trim()));
                                }
                            }
                            None => collect_text(space, *child, &mut text, 0),
                        }
                    }
                    None => collect_text(space, *child, &mut text, 0),
                    Some(_) => {}
                }
            }
            if text.trim().is_empty() {
                if let Some(own) = object_text(obj) {
                    text = own;
                }
            }
            let text = text.trim_matches(|c: char| c.is_whitespace() || c == '\0');
            if !text.is_empty() {
                // An outline element carries both the item text and, for list
                // items, the marker describing how it is numbered.
                let lists = child_oids(obj, pid::LIST_NODES);
                let text_with_tag = match note_tag(space, obj) {
                    Some(tag) => format!("{tag} {text}"),
                    None => text.to_string(),
                };
                match lists.first().and_then(|oid| space.objects.get(oid)) {
                    Some(list) => out.push(Block::Bullet {
                        text: emphasize(space, obj, &text_with_tag),
                        level: element_level,
                        marker: list_marker(list, element_level, counters),
                    }),
                    None => out.push(Block::Paragraph {
                        text: emphasize(space, obj, &text_with_tag),
                        level: element_level,
                    }),
                }
            }
            for list in child_oids(obj, pid::LIST_NODES) {
                walk_with(
                    space,
                    list,
                    element_level,
                    out,
                    visited,
                    depth + 1,
                    counters,
                );
            }
            // Content that is not text (pictures, tables) is rendered on its own.
            for child in child_oids(obj, pid::CONTENT_CHILD_NODES) {
                match space.objects.get(&child).map(|object| object.jcid) {
                    Some(jcid::RICH_TEXT_NODE) => {}
                    _ => walk_with(
                        space,
                        child,
                        element_level,
                        out,
                        visited,
                        depth + 1,
                        counters,
                    ),
                }
            }
            for child in child_oids(obj, pid::ELEMENT_CHILD_NODES) {
                walk_with(
                    space,
                    child,
                    element_level.saturating_add(1),
                    out,
                    visited,
                    depth + 1,
                    counters,
                );
            }
        }
        _ => {
            // Containers: page manifest, page node, title node, outline node,
            // table cell, and anything else that only groups children.
            for child in all_children(obj) {
                walk_with(space, child, level, out, visited, depth + 1, counters);
            }
        }
    }
}

/// Character formatting of one text run, from a `jcidParagraphStyleObject`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RunStyle {
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    color: Option<u32>,
    highlight: Option<u32>,
}

impl RunStyle {
    fn of(space: &ObjectSpace, oid: u32) -> RunStyle {
        let Some(style) = space.objects.get(&oid) else {
            return RunStyle::default();
        };
        let props = &style.props;
        let color = match props.get(pid::FONT_COLOR) {
            Some(prop::Value::U32(value)) => Some(*value),
            _ => None,
        };
        let highlight = match props.get(pid::HIGHLIGHT) {
            Some(prop::Value::U32(value)) => Some(*value),
            _ => None,
        };
        RunStyle {
            bold: props.bool(pid::BOLD).unwrap_or(false),
            italic: props.bool(pid::ITALIC).unwrap_or(false),
            underline: props.bool(pid::UNDERLINE).unwrap_or(false),
            strikethrough: props.bool(pid::STRIKETHROUGH).unwrap_or(false),
            color,
            highlight,
        }
    }

    /// Emphasis flags that Markdown wraps around text.
    fn emphasis(&self) -> (bool, bool, bool, bool) {
        (self.bold, self.italic, self.underline, self.strikethrough)
    }

    /// Apply `emphasis` markers to the trimmed core of `text`, keeping the
    /// surrounding whitespace outside the markers (Markdown requires emphasis
    /// to hug its content).
    fn wrap_with(&self, text: &str) -> String {
        let emphasis = self.emphasis();
        let leading_len = text.len() - text.trim_start().len();
        let core_end = text.trim_end().len();
        if core_end <= leading_len {
            return text.to_string();
        }
        let leading = &text[..leading_len];
        let core = &text[leading_len..core_end];
        let trailing = &text[core_end..];
        format!("{leading}{}{trailing}", decorate(core, emphasis))
    }

    /// Wrap the run in colour/highlight spans, if it has non-default values.
    fn with_color(&self, text: &str) -> String {
        let mut out = text.to_string();
        if let Some(highlight) = self.highlight {
            if highlight >> 24 != 0 && highlight & 0x00FF_FFFF != 0 {
                out = format!(
                    "<mark style=\"background:#{:06x}\">{out}</mark>",
                    highlight & 0x00FF_FFFF
                );
            }
        }
        if let Some(color) = self.color {
            if color >> 24 != 0 && color & 0x00FF_FFFF != 0 {
                out = format!(
                    "<span style=\"color:#{:06x}\">{out}</span>",
                    color & 0x00FF_FFFF
                );
            }
        }
        out
    }
}

/// `[label]` for an element that carries a note tag, if the definition is in
/// the same object space.
fn note_tag(space: &ObjectSpace, obj: &Object) -> Option<String> {
    let definition = obj.props.oid(pid::NOTE_TAG_DEFINITION_OID)?;
    let definition = space.objects.get(&definition)?;
    let label = definition
        .props
        .bytes(pid::NOTE_TAG_LABEL)
        .map(prop::decode_utf16)
        .map(|value| value.trim_matches('\0').trim().to_string())
        .filter(|value| !value.is_empty())?;
    Some(format!("[{label}]"))
}

/// Marker for a list item: `-` for bulleted lists, `N<suffix>` for numbered
/// ones. `NumberListFormat` carries U+FFFD where the number goes and the text
/// after U+0004 is its suffix.
fn list_marker(
    obj: &Object,
    level: u8,
    counters: &mut std::collections::BTreeMap<u8, u32>,
) -> String {
    let Some(bytes) = obj.props.bytes(pid::NUMBER_LIST_FORMAT) else {
        return "-".to_string();
    };
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    if !units.contains(&0xFFFD) {
        return "-".to_string();
    }
    let suffix: String = match units.iter().position(|unit| *unit == 0x0004) {
        Some(end) => units[end + 1..]
            .iter()
            .filter_map(|unit| char::from_u32(*unit as u32))
            .collect(),
        None => ".".to_string(),
    };
    let counter = counters.entry(level).or_insert(0);
    *counter += 1;
    let number = *counter;
    // A shallower level advancing restarts the deeper ones.
    counters.retain(|deeper, _| *deeper <= level);
    format!("{number}{suffix}")
}

/// Wrap `text` with the emphasis markers described by `emphasis`
/// (bold, italic, underline, strikethrough).
fn decorate(text: &str, emphasis: (bool, bool, bool, bool)) -> String {
    let (bold, italic, underline, strikethrough) = emphasis;
    if text.is_empty() {
        return String::new();
    }
    let mut out = text.to_string();
    if strikethrough {
        out = format!("~~{out}~~");
    }
    if underline {
        out = format!("<u>{out}</u>");
    }
    if italic {
        out = format!("*{out}*");
    }
    if bold {
        out = format!("**{out}**");
    }
    out
}

/// Text with per-run formatting applied.
///
/// `TextRunIndex` holds the character positions (in UTF-16 units of the raw,
/// uncleaned text) where each run ends; the number of runs is one more than the
/// number of positions, matching `TextRunFormatting` entries.
fn formatted_text(space: &ObjectSpace, obj: &Object) -> Option<String> {
    // Only UTF-16 text carries run formatting in this corpus; single-byte text
    // keeps the plain path.
    obj.props.bytes(pid::RICH_TEXT_UNICODE)?;
    let raw = raw_units(obj)?;
    let styles: Vec<u32> = obj
        .props
        .oids(pid::TEXT_RUN_FORMATTING)
        .map(|ids| ids.to_vec())
        .unwrap_or_default();
    let boundaries = run_boundaries(obj, raw.len());

    if styles.len() < 2 || boundaries.len() + 1 != styles.len() {
        // A single style is the paragraph style; element-level emphasis already
        // covers it.
        return Some(prop::clean_text(&String::from_utf16_lossy(&raw)));
    }

    let cleaned = prop::clean_units(&raw);
    let chars: Vec<char> = cleaned.0.chars().collect();

    // Group consecutive runs that share formatting: emitting separate markers
    // would render as literal asterisks.
    let mut runs: Vec<(RunStyle, String)> = Vec::new();
    let mut start = 0usize;
    for (index, style_oid) in styles.iter().enumerate() {
        let end = boundaries.get(index).copied().unwrap_or(raw.len());
        let end = end.min(raw.len()).max(start);
        let from = *cleaned.1.get(start).unwrap_or(&0);
        let to = *cleaned.1.get(end).unwrap_or(&chars.len());
        let text: String = chars[from.min(chars.len())..to.min(chars.len())]
            .iter()
            .collect();
        let style = RunStyle::of(space, *style_oid);
        match runs.last_mut() {
            Some((previous, buffer)) if *previous == style => buffer.push_str(&text),
            _ => runs.push((style, text)),
        }
        start = end;
    }

    // Group consecutive runs that share emphasis so adjacent markers never
    // collide ("*a**b*" would render as literal asterisks); colour spans go
    // inside the emphasis.
    let mut out = String::new();
    let mut index = 0usize;
    while index < runs.len() {
        let emphasis = runs[index].0.emphasis();
        let mut body = String::new();
        while index < runs.len() && runs[index].0.emphasis() == emphasis {
            let (style, text) = &runs[index];
            body.push_str(&style.with_color(text));
            index += 1;
        }
        out.push_str(&runs[index - 1].0.wrap_with(&body));
    }
    Some(out)
}

/// UTF-16 units of an object's text property, before field-code cleanup.
fn raw_units(obj: &Object) -> Option<Vec<u16>> {
    if let Some(bytes) = obj.props.bytes(pid::RICH_TEXT_UNICODE) {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|unit| *unit != 0)
            .collect();
        if !units.is_empty() {
            return Some(units);
        }
    }
    let bytes = obj.props.bytes(pid::TEXT_EXTENDED_ASCII)?;
    let units: Vec<u16> = bytes
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| *byte as u16)
        .collect();
    if units.is_empty() {
        None
    } else {
        Some(units)
    }
}

/// Run end positions from `TextRunIndex`, tolerating an optional count prefix.
fn run_boundaries(obj: &Object, text_len: usize) -> Vec<usize> {
    let Some(bytes) = obj.props.bytes(pid::TEXT_RUN_INDEX) else {
        return Vec::new();
    };
    let values: Vec<u32> = bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    let monotonic = |candidate: &[u32]| {
        candidate.windows(2).all(|pair| pair[0] < pair[1])
            && candidate
                .last()
                .map(|last| (*last as usize) <= text_len)
                .unwrap_or(true)
    };
    // Prefer the whole array; if a leading count makes it inconsistent, drop it.
    if monotonic(&values) {
        return values.into_iter().map(|value| value as usize).collect();
    }
    if !values.is_empty() && monotonic(&values[1..]) {
        return values[1..].iter().map(|value| *value as usize).collect();
    }
    Vec::new()
}

/// Whole-paragraph emphasis from the element's `ParagraphStyle` object.
fn emphasize(space: &ObjectSpace, obj: &Object, text: &str) -> String {
    let Some(style_oid) = obj.props.oid(pid::PARAGRAPH_STYLE) else {
        return text.to_string();
    };
    let Some(style) = space.objects.get(&style_oid) else {
        return text.to_string();
    };
    let bold = style.props.bool(pid::BOLD).unwrap_or(false);
    let italic = style.props.bool(pid::ITALIC).unwrap_or(false);
    let underline = style.props.bool(pid::UNDERLINE).unwrap_or(false);
    if !(bold || italic || underline) {
        return text.to_string();
    }
    let mut out = text.to_string();
    if underline {
        out = format!("<u>{out}</u>");
    }
    if italic {
        out = format!("*{out}*");
    }
    if bold {
        out = format!("**{out}**");
    }
    out
}

fn collect_text(space: &ObjectSpace, oid: u32, out: &mut String, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let Some(obj) = space.objects.get(&oid) else {
        return;
    };
    if std::env::var_os("ONENOTE_TRACE").is_some() {
        let snippet = object_text(obj).unwrap_or_default();
        eprintln!(
            "{:indent$}{} {} {:?}",
            "",
            oid,
            crate::prop::jcid_name(obj.jcid),
            snippet.chars().take(30).collect::<String>(),
            indent = depth * 2
        );
    }
    match obj.jcid {
        jcid::TITLE_NODE => {}
        jcid::RICH_TEXT_NODE => {
            let Some(text) = object_text(obj) else {
                return;
            };
            let url = obj
                .props
                .bytes(pid::HYPERLINK_URL)
                .map(prop::decode_utf16)
                .map(|u| u.trim_matches('\0').to_string())
                .unwrap_or_default();
            if url.is_empty() {
                out.push_str(&text);
            } else {
                out.push_str(&format!("[{text}]({url})"));
            }
        }
        jcid::IMAGE_NODE | jcid::TABLE_NODE => {}
        _ => {
            for child in all_children(obj) {
                collect_text(space, child, out, depth + 1);
            }
            if out.is_empty() {
                if let Some(text) = object_text(obj) {
                    out.push_str(&text);
                }
            }
        }
    }
}

/// Formatted text of an object, for diagnostics.
pub fn formatted_text_snippet(space: &ObjectSpace, object: &Object) -> Option<String> {
    formatted_text(space, object)
}

/// Short text preview of an object, for diagnostics.
pub fn object_text_snippet(object: &Object) -> Option<String> {
    let text = object_text(object)?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.chars().take(70).collect())
}

/// First non-empty text in a subtree, ignoring title furniture such as the
/// date and time stamps (they have no text property of their own beyond the run).
fn first_text(
    space: &ObjectSpace,
    oid: u32,
    seen: &mut HashSet<u32>,
    depth: usize,
) -> Option<String> {
    if depth > MAX_DEPTH || !seen.insert(oid) {
        return None;
    }
    let obj = space.objects.get(&oid)?;
    if let Some(text) = object_text(obj) {
        let text = text.trim();
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    for child in all_children(obj) {
        if let Some(text) = first_text(space, child, seen, depth + 1) {
            return Some(text);
        }
    }
    None
}

fn decode_prop_text(obj: &Object, id: u32) -> Option<String> {
    obj.props
        .bytes(id)
        .map(prop::decode_utf16)
        .map(|text| text.trim_matches('\0').trim().to_string())
        .filter(|text| !text.is_empty())
}

fn image_block(space: &ObjectSpace, oid: u32) -> Option<AssetRef> {
    let obj = space.objects.get(&oid)?;
    if obj.jcid != jcid::IMAGE_NODE {
        return None;
    }
    // A jcidImageNode carries RichEditTextUnicode as the text that describes
    // the picture (used by OneNote for OCR/alt text), so it becomes the alt.
    let link = obj
        .props
        .bytes(pid::HYPERLINK_URL)
        .map(prop::decode_utf16)
        .map(|value| value.trim_matches('\0').trim().to_string())
        .filter(|value| !value.is_empty());
    let mut image = AssetRef {
        link,
        alt: decode_prop_text(obj, pid::IMAGE_ALT_TEXT)
            .or_else(|| object_text(obj))
            .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
            .unwrap_or_default(),
        filename: decode_prop_text(obj, pid::IMAGE_FILENAME).unwrap_or_default(),
        ..AssetRef::default()
    };
    if let Some(container) = obj.props.oid(pid::PICTURE_CONTAINER) {
        if let Some(data) = space.file_data_refs.get(&container) {
            image.extension = data.extension.trim().to_string();
            image.guid = data.store_guid();
            image.sibling = data.sibling_file().map(str::to_string);
        }
    }
    if image.filename.is_empty() {
        image.filename = match (&image.guid, image.extension.is_empty()) {
            (Some(guid), false) => format!("image-{}{}", short_guid(guid), image.extension),
            (Some(guid), true) => format!("image-{}", short_guid(guid)),
            (None, false) => format!("image-{oid}{}", image.extension),
            (None, true) => format!("image-{oid}"),
        };
    }
    Some(image)
}

/// An embedded file (not a picture): rendered as a link to the extracted bytes.
fn attachment_block(space: &ObjectSpace, oid: u32) -> Option<AssetRef> {
    let obj = space.objects.get(&oid)?;
    if obj.jcid != jcid::EMBEDDED_FILE_NODE {
        return None;
    }
    let mut asset = AssetRef {
        alt: String::new(),
        filename: decode_prop_text(obj, pid::EMBEDDED_FILE_NAME)
            .or_else(|| object_text(obj))
            .unwrap_or_else(|| format!("attachment-{oid}")),
        ..AssetRef::default()
    };
    if let Some(container) = obj.props.oid(pid::EMBEDDED_FILE_CONTAINER) {
        if let Some(data) = space.file_data_refs.get(&container) {
            asset.extension = data.extension.trim().to_string();
            asset.guid = data.store_guid();
            asset.sibling = data.sibling_file().map(str::to_string);
        }
    }
    Some(asset)
}

fn short_guid(guid: &Guid) -> String {
    let text = guid.to_string();
    text.trim_start_matches('{')
        .trim_end_matches('}')
        .chars()
        .take(8)
        .collect()
}

fn table_block(space: &ObjectSpace, oid: u32) -> Option<Block> {
    let table = space.objects.get(&oid)?;
    let mut rows = Vec::new();
    for row_oid in all_children(table) {
        let Some(row) = space.objects.get(&row_oid) else {
            continue;
        };
        if row.jcid != jcid::TABLE_ROW_NODE {
            continue;
        }
        let mut cells = Vec::new();
        for cell_oid in all_children(row) {
            let Some(cell) = space.objects.get(&cell_oid) else {
                continue;
            };
            if cell.jcid != jcid::TABLE_CELL_NODE {
                continue;
            }
            cells.push(cell_inlines(space, cell_oid));
        }
        if !cells.is_empty() {
            rows.push(cells);
        }
    }
    if rows.is_empty() {
        None
    } else {
        Some(Block::Table { rows })
    }
}

/// A cell holds outline elements (occasionally a nested table), so it is walked
/// with the same rules as page content and the result kept as inlines.
fn cell_inlines(space: &ObjectSpace, cell_oid: u32) -> Vec<Inline> {
    let mut blocks = Vec::new();
    let mut visited = HashSet::new();
    if let Some(cell) = space.objects.get(&cell_oid) {
        for child in all_children(cell) {
            walk(space, child, 0, &mut blocks, &mut visited, 0);
        }
    }
    let mut out = Vec::new();
    for block in blocks {
        match block {
            Block::Paragraph { text, .. } => push_text(&mut out, &text),
            Block::Bullet { text, .. } => push_text(&mut out, &format!("• {text}")),
            Block::Image(asset) => out.push(Inline::Image(asset)),
            Block::Attachment(asset) => push_text(&mut out, &asset.filename),
            Block::Note(text) => push_text(&mut out, &format!("[{text}]")),
            Block::Table { rows } => {
                for row in rows {
                    let cells: Vec<String> = row.iter().map(|c| inlines_to_text(c)).collect();
                    push_text(&mut out, &cells.join(" / "));
                }
            }
        }
    }
    out
}

fn push_text(out: &mut Vec<Inline>, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    if let Some(Inline::Text(previous)) = out.last_mut() {
        if !previous.is_empty() {
            previous.push(' ');
        }
        previous.push_str(text);
        return;
    }
    out.push(Inline::Text(text.to_string()));
}

fn inlines_to_text(inlines: &[Inline]) -> String {
    inlines
        .iter()
        .filter_map(|inline| match inline {
            Inline::Text(text) => Some(text.as_str()),
            Inline::Image(_) => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prop::{PropertyId, PropertySet, Value};

    fn style(space: &mut ObjectSpace, oid: u32, bold: bool) {
        let props = PropertySet {
            properties: vec![(PropertyId(0x0800_1C04), Value::Bool(bold))],
        };
        space.objects.insert(
            oid,
            Object {
                oid,
                jcid: jcid::PARAGRAPH_STYLE_OBJECT,
                props,
                blob_size: 0,
                consumed: 0,
                blob_offset: 0,
                parse_error: None,
                text_variants: Vec::new(),
            },
        );
    }

    #[test]
    fn bold_run_in_the_middle_keeps_the_surrounding_spaces() {
        let mut space = ObjectSpace::default();
        style(&mut space, 1, true);
        style(&mut space, 2, false);

        let text: Vec<u8> = "Hello bold world"
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        let boundaries: Vec<u8> = [6u32, 10]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let props = PropertySet {
            properties: vec![
                (PropertyId(0x1C00_1C22), Value::Bytes(text)),
                (PropertyId(0x1C00_1E12), Value::Bytes(boundaries)),
                (PropertyId(0x2400_1E13), Value::Oids(vec![2, 1, 2])),
            ],
        };
        let object = Object {
            oid: 3,
            jcid: jcid::RICH_TEXT_NODE,
            props,
            blob_size: 0,
            consumed: 0,
            blob_offset: 0,
            parse_error: None,
            text_variants: Vec::new(),
        };
        space.objects.insert(3, object.clone());

        let rendered = formatted_text_snippet(&space, &space.objects[&3]).expect("formatted");
        assert_eq!(rendered, "Hello **bold** world");
    }

    #[test]
    fn adjacent_runs_with_the_same_emphasis_are_merged() {
        let mut space = ObjectSpace::default();
        style(&mut space, 1, true);

        let text: Vec<u8> = "abcd"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        let boundaries: Vec<u8> = [2u32].iter().flat_map(|v| v.to_le_bytes()).collect();
        let props = PropertySet {
            properties: vec![
                (PropertyId(0x1C00_1C22), Value::Bytes(text)),
                (PropertyId(0x1C00_1E12), Value::Bytes(boundaries)),
                (PropertyId(0x2400_1E13), Value::Oids(vec![1, 1])),
            ],
        };
        let object = Object {
            oid: 2,
            jcid: jcid::RICH_TEXT_NODE,
            props,
            blob_size: 0,
            consumed: 0,
            blob_offset: 0,
            parse_error: None,
            text_variants: Vec::new(),
        };
        space.objects.insert(2, object.clone());

        let rendered = formatted_text_snippet(&space, &space.objects[&2]).expect("formatted");
        assert_eq!(rendered, "**abcd**", "one marker pair, not **ab****cd**");
    }
}
