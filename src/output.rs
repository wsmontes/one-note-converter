//! Section-level API shared by the CLI and the Python bindings.
//!
//! A [`Section`] owns the file bytes, the parsed store and its pages, so the same
//! object can render Markdown, hand out the embedded pictures, or write both to
//! disk. Asset naming and link escaping live here too, so every front end emits
//! identical output.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::doc::{self, AssetRef, Page};
use crate::guid::Guid;
use crate::{markdown, store};

/// One embedded picture, ready to be written by the caller.
#[derive(Debug, Clone)]
pub struct Image {
    pub filename: String,
    pub bytes: Vec<u8>,
}

/// What writing a section produced.
#[derive(Debug, Clone, Default)]
pub struct SectionStats {
    pub pages: usize,
    pub assets: usize,
    /// Pictures that could not be written (link target, then the OS error).
    pub failures: Vec<String>,
    /// Missing payloads and parser or format limitations.
    pub warnings: Vec<String>,
}

/// A parsed section file (`.one`), with its pages and embedded data.
pub struct Section {
    data: Vec<u8>,
    store: store::Store,
    pages: Vec<Page>,
}

impl Section {
    /// Parse a section from its file bytes.
    pub fn read(data: Vec<u8>) -> Result<Section, String> {
        let (_header, store) = crate::load(&data)?;
        let pages = doc::extract(&store);
        if pages.is_empty() {
            return Err("no pages found in this section file".to_string());
        }
        Ok(Section { data, store, pages })
    }

    pub fn pages(&self) -> &[Page] {
        &self.pages
    }

    pub fn store(&self) -> &store::Store {
        &self.store
    }

    /// The section file bytes, as given to [`Section::read`].
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    /// One pass over every picture: the display name per GUID (the order is the
    /// render order, which is what makes `unique_name` suffixes stable) and the
    /// bytes to write. `markdown`, `images` and `materialize` all read from it,
    /// so a link always names a file the caller has.
    fn named_images(&self) -> (BTreeMap<Guid, String>, Vec<Image>, Vec<String>) {
        let mut written = BTreeSet::new();
        let mut names: BTreeMap<Guid, String> = BTreeMap::new();
        let mut images = Vec::new();
        let mut missing: BTreeSet<String> = BTreeSet::new();
        for page in &self.pages {
            for asset in page_images(page) {
                let payload = asset
                    .guid
                    .and_then(|guid| store::file_data_bytes(&self.store, &self.data, guid));
                match (asset.guid, payload) {
                    (Some(guid), Some(bytes)) => {
                        if names.contains_key(&guid) {
                            continue;
                        }
                        let name = unique_name(&asset_name(&asset, guid), &mut written);
                        names.insert(guid, name.clone());
                        images.push(Image {
                            filename: name,
                            bytes: bytes.to_vec(),
                        });
                    }
                    // A reference without payload (web clipping, external cache):
                    // Markdown keeps the name, but there is nothing to write.
                    _ => {
                        missing.insert(match &asset.sibling {
                            Some(sibling) => sibling.clone(),
                            None => asset.filename.clone(),
                        });
                    }
                }
            }
        }
        (names, images, missing.into_iter().collect())
    }

    /// Pictures referenced by the pages, with the name each one is linked by.
    /// Only references whose bytes are missing: Markdown names them, but they
    /// cannot be written because the file does not carry their payload.
    pub fn missing_images(&self) -> Vec<String> {
        self.named_images().2
    }

    /// Display name of every picture that has a payload, keyed by its GUID.
    pub fn image_names(&self) -> BTreeMap<Guid, String> {
        self.named_images().0
    }

    /// Markdown for every page; picture links use the same names as
    /// [`Section::images`].
    pub fn markdown(&self, front_matter: bool) -> String {
        let (names, _, _) = self.named_images();
        let mut resolver = |image: &AssetRef| -> String {
            if let Some(name) = image.guid.and_then(|guid| names.get(&guid)) {
                return join_link("", name);
            }
            link_name(image)
        };
        markdown::render_with_options(&self.pages, &mut resolver, front_matter)
    }

    /// Embedded pictures, with the names [`Section::markdown`] links to.
    pub fn images(&self) -> Vec<Image> {
        self.named_images().1
    }

    /// Render Markdown, writing every picture into `assets` and linking it with
    /// `prefix`. Returns the Markdown and what was written.
    pub fn materialize(
        &self,
        assets: &Path,
        prefix: &str,
        front_matter: bool,
    ) -> Result<(String, SectionStats), String> {
        let (names, images, missing) = self.named_images();
        let mut failures: Vec<String> = Vec::new();
        let mut warnings = self.store.warnings.clone();
        for name in missing {
            warnings.push(format!("asset {name:?} has no embedded payload"));
        }
        if self.store.header.kind() == crate::header::ContainerKind::PackageStore {
            warnings.push("[MS-FSSHTTPB] support is experimental; page bodies may be incomplete".into());
        }
        let mut assets_written = 0usize;
        for image in &images {
            let path = assets.join(&image.filename);
            let ok = std::fs::create_dir_all(assets)
                .and_then(|()| std::fs::write(&path, &image.bytes))
                .is_ok();
            if ok {
                assets_written += 1;
            } else {
                failures.push(format!("cannot write {}", path.display()));
            }
        }
        let mut resolver = |image: &AssetRef| -> String {
            if let Some(name) = image.guid.and_then(|guid| names.get(&guid)) {
                return join_link(prefix, name);
            }
            let name = link_name(image);
            if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            }
        };
        let text = markdown::render_with_options(&self.pages, &mut resolver, front_matter);
        Ok((
            text,
            SectionStats {
                pages: self.pages.len(),
                assets: assets_written,
                failures,
                warnings,
            },
        ))
    }

    /// Export each page as a standalone Markdown document, with its own valid
    /// YAML front matter. Assets are shared by pages inside this section.
    /// The result includes an index.md in the requested output directory.
    pub fn write_pages(&self, out: &Path, front_matter: bool) -> Result<SectionStats, String> {
        std::fs::create_dir_all(out)
            .map_err(|error| format!("cannot create {}: {error}", out.display()))?;
        let assets = out.join("assets");
        let (_, mut stats) = self.materialize(&assets, "assets", front_matter)?;
        let names = self.image_names();
        let mut used = BTreeSet::new();
        let mut index = String::new();
        for page in &self.pages {
            let label = format!("{}-{}-{}.md", page.title, page.space_index, page.oid);
            let filename = unique_name(&label, &mut used);
            let mut resolver = |image: &AssetRef| {
                image.guid.and_then(|guid| names.get(&guid))
                    .map(|name| join_link("assets", name))
                    .unwrap_or_else(|| link_name(image))
            };
            let text = markdown::render_with_options(
                std::slice::from_ref(page), &mut resolver, front_matter
            );
            let file = out.join(&filename);
            std::fs::write(&file, text)
                .map_err(|error| format!("cannot write {}: {error}", file.display()))?;
            index.push_str(&format!("- [{}]({})\n",
                page.title.replace('[', "\\[").replace(']', "\\]"),
                escape_link(&filename)));
        }
        std::fs::write(out.join("index.md"), index)
            .map_err(|error| format!("cannot write {}/index.md: {error}", out.display()))?;
        stats.pages = self.pages.len();
        Ok(stats)
    }

    /// Write `<output>` plus its pictures into `assets`.
    pub fn write(
        &self,
        output: &Path,
        assets: &Path,
        front_matter: bool,
    ) -> Result<SectionStats, String> {
        let base_dir = output
            .parent()
            .map(Path::to_path_buf)
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| PathBuf::from("."));
        let prefix = relative_link_prefix(&base_dir, assets);
        let (text, stats) = self.materialize(assets, &prefix, front_matter)?;
        if let Some(parent) = output.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
            }
        }
        std::fs::write(output, text)
            .map_err(|error| format!("cannot write {}: {error}", output.display()))?;
        Ok(stats)
    }
}

/// Every picture referenced by one page's blocks.
pub fn page_images(page: &Page) -> Vec<AssetRef> {
    let mut images = Vec::new();
    collect_images(&page.blocks, &mut images);
    images
}

fn collect_images(blocks: &[doc::Block], into: &mut Vec<AssetRef>) {
    for block in blocks {
        match block {
            doc::Block::Image(image) => into.push(image.clone()),
            doc::Block::Table { rows } => {
                for row in rows {
                    for cell in row {
                        for inline in cell {
                            if let doc::Inline::Image(image) = inline {
                                into.push(image.clone());
                            }
                        }
                    }
                }
            }
            // Text-only blocks carry no picture reference.
            _ => {}
        }
    }
}

/// How Markdown links a picture that has no payload: the sibling it points at,
/// or its stored file name. Percent-encoded, like every link.
fn link_name(image: &AssetRef) -> String {
    match &image.sibling {
        Some(sibling) => join_link("", sibling),
        None => join_link("", &image.filename),
    }
}

/// `stem-<short guid><extension>`, the name used for an extracted picture.
pub fn asset_name(image: &AssetRef, guid: crate::guid::Guid) -> String {
    let short: String = guid
        .to_string()
        .trim_start_matches('{')
        .chars()
        .take(8)
        .collect();
    let (stem, extension) = match image.filename.rsplit_once('.') {
        Some((stem, extension)) => (stem.to_string(), format!(".{extension}")),
        None => (image.filename.clone(), image.extension.clone()),
    };
    let stem = stem.trim();
    if stem.is_empty() {
        format!("image-{short}{extension}")
    } else {
        format!("{stem}-{short}{extension}")
    }
}

/// Keep names unique and free of path separators.
pub fn unique_name(filename: &str, written: &mut BTreeSet<String>) -> String {
    let sanitized: String = filename
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("image")
        .chars()
        .map(|c| if c.is_control() || "<>:\"|?*".contains(c) { '_' } else { c })
        .collect();
    let sanitized = sanitized.trim_end_matches(['.', ' ']);
    let sanitized = if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        "image"
    } else {
        sanitized
    };
    let (stem, extension) = match sanitized.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
        _ => (sanitized.to_string(), String::new()),
    };
    let mut candidate = sanitized.to_string();
    let mut counter = 2;
    while !written.insert(candidate.clone()) {
        candidate = format!("{stem}-{counter}{extension}");
        counter += 1;
    }
    candidate
}

fn percent_encode(name: &str) -> String {
    let mut escaped = String::new();
    for byte in name.bytes() {
        match byte {
            b' ' | b'(' | b')' | b'%' | b'#' | b'?' | b'<' | b'>' | b'[' | b']'
            | b'\\' | b'&' | b'\'' | b'"' | b'\n' | b'\r' => {
                escaped.push_str(&format!("%{byte:02X}"));
            }
            // Leave Unicode UTF-8 untouched for readable, portable links.
            _ => escaped.push(byte as char),
        }
    }
    // Reconstitute UTF-8: percent-encoded bytes are ASCII; unescaped bytes
    // should stay as valid UTF-8, rather than mojibake.
    if name.is_ascii() {
        return escaped;
    }
    let mut result = String::new();
    for ch in name.chars() {
        if ch.is_ascii() {
            let b = ch as u8;
            if matches!(b, b' ' | b'(' | b')' | b'%' | b'#' | b'?' | b'<' | b'>'
                | b'[' | b']' | b'\\' | b'&' | b'\'' | b'"' | b'\n' | b'\r') {
                result.push_str(&format!("%{b:02X}"));
            } else {
                result.push(ch);
            }
        } else {
            result.push(ch);
        }
    }
    result
}

pub fn escape_link(name: &str) -> String {
    percent_encode(name)
}

pub fn join_link(prefix: &str, name: &str) -> String {
    let escaped = percent_encode(name);
    if prefix.is_empty() {
        escaped
    } else {
        format!("{prefix}/{escaped}")
    }
}

/// Link prefix that turns `assets` into a path relative to `base`.
pub fn relative_link_prefix(base: &Path, assets: &Path) -> String {
    use std::path::Component;
    let clean = |path: &Path| -> PathBuf {
        let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let mut result = PathBuf::new();
        for component in absolute.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => { result.pop(); }
                other => result.push(other.as_os_str()),
            }
        }
        result
    };
    let base = clean(base);
    let assets = clean(assets);
    let from: Vec<_> = base.components().collect();
    let to: Vec<_> = assets.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    // Different volume roots (e.g. Windows drives) have no portable relative link.
    if common == 0 {
        return assets.to_string_lossy().replace('\\', "/");
    }
    let mut parts = vec!["..".to_string(); from.len() - common];
    parts.extend(to[common..].iter().map(|part| part.as_os_str().to_string_lossy().to_string()));
    parts.join("/")
}

/// The `<name>.onetoc2` that describes a folder's notebook.
///
/// A dotfile `.onetoc2` is the OneNote Recycle Bin sentinel, not a notebook
/// index, so it never counts.
pub fn find_toc(folder: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(folder).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_toc = path
            .extension()
            .map(|ext| ext == "onetoc2")
            .unwrap_or(false);
        let hidden = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().starts_with('.'))
            .unwrap_or(false);
        if is_toc && !hidden {
            return Some(path);
        }
    }
    None
}

/// Subdirectories that carry their own table of contents (section groups).
pub fn subfolders_with_toc(folder: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(folder) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && find_toc(&path).is_some() {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// Files directly inside `dir` with the given extension, sorted.
pub fn root_level(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == extension))
        .collect();
    found.sort();
    found
}

/// Reject absolute paths and `..` components coming from a package.
pub fn package_path(name: &str) -> Option<PathBuf> {
    let normalised = name.replace('\\', "/");
    if normalised.starts_with('/') || normalised.chars().any(|c| c.is_control()) {
        return None;
    }
    let mut path = PathBuf::new();
    for part in normalised.split('/') {
        if part == ".." || part.contains(':') {
            return None;
        }
        if part.is_empty() || part == "." {
            continue;
        }
        path.push(part);
    }
    (!path.as_os_str().is_empty()).then_some(path)
}

/// A TOC element is a file or folder name, never a path.
/// Validate before using it for either reading or writing.
pub fn safe_toc_name(name: &str) -> Option<&str> {
    if name.is_empty() || name == "." || name == ".." || name.ends_with('.')
        || name.ends_with(' ') || name.contains(['/', '\\', ':'])
        || name.chars().any(|c| c.is_control() || "<>\"|?*".contains(c))
    {
        None
    } else {
        Some(name)
    }
}


#[cfg(test)]
mod safety_tests {
    use super::*;

    #[test]
    fn rejects_unsafe_package_and_toc_names() {
        for name in ["/etc/secret.one", "../secret.one", "a/../secret.one",
                     "C:\\temp\\secret.one", "sub/../../secret.one"] {
            assert!(package_path(name).is_none(), "{name}");
        }
        assert_eq!(package_path("notes/Section.one"), Some(PathBuf::from("notes/Section.one")));
        for name in ["../escape.one", "/absolute.one", "C:\\escape.one",
                     "sub/section.one", "..", "bad?.one"] {
            assert!(safe_toc_name(name).is_none(), "{name}");
        }
        assert_eq!(safe_toc_name("Nova Seção 1.one"), Some("Nova Seção 1.one"));
    }

    #[test]
    fn sibling_asset_links_are_relative_and_uri_safe() {
        let relative = relative_link_prefix(Path::new("export/pages"), Path::new("export/assets"));
        assert_eq!(relative, "../assets");
        assert_eq!(join_link(&relative, "A#? 100% (1).png"),
                   "../assets/A%23%3F%20100%25%20%281%29.png");
        assert_eq!(escape_link("Nova Seção 1.md"), "Nova%20Seção%201.md");
    }

    #[test]
    fn missing_payload_is_reported_during_materialization() {
        let mut page = Page { title: "Pictures".into(), ..Page::default() };
        page.blocks.push(doc::Block::Image(AssetRef {
            filename: "not-embedded.png".into(), ..AssetRef::default()
        }));
        let section = Section { data: Vec::new(), store: store::Store::default(), pages: vec![page] };
        let scratch = crate::temp::ScratchDir::new("onenote-missing-asset-test").unwrap();
        let (markdown, stats) = section.materialize(scratch.path(), "", false).unwrap();
        assert!(markdown.contains("not-embedded.png"));
        assert!(stats.warnings.iter().any(|w| w.contains("not-embedded.png")));
        assert_eq!(stats.assets, 0);
    }
}
