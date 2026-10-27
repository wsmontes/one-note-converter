//! Integration tests against a real OneNote revision store fixture.
//!
//! The fixture comes from the Apache Tika test corpus
//! (`test-documents/testOneNote2016.one`, Apache License 2.0); see
//! `tests/fixtures/README.md` for provenance.
//!
//! The expected title and body below are not this parser's own output: they are
//! the assertions of Apache Tika's `OneNoteParserTest.testOneNote2016`
//! (`assertContains("So good")`, `assertContains("This is one note 2016")`),
//! an independent implementation. That keeps the test from freezing a bug.

use one_note_converter::doc::{extract, Block};
use one_note_converter::header::{ContainerKind, Header, GUID_FILE_TYPE_ONETOC2};
use one_note_converter::store::{count_blob_padding, dangling_references, Store};

const FIXTURE: &[u8] = include_bytes!("fixtures/testOneNote2016.one");

#[test]
fn header_identifies_a_revision_store() {
    let header = Header::parse(FIXTURE).expect("header parses");
    assert_eq!(header.kind(), ContainerKind::RevisionStore);
    assert_eq!(header.ffv_last_code, 0x2A);
    assert!(header.file_length_matches(FIXTURE.len()));
    assert!(header.fcr_file_node_list_root.is_valid());
}

#[test]
fn every_property_set_consumes_its_blob_modulo_padding() {
    let store = Store::parse(FIXTURE).expect("store parses");
    assert!(!store.all_objects().next().is_none(), "no objects parsed");
    for (padding, count) in count_blob_padding(&store) {
        assert!(
            padding < 8,
            "{count} objects left {padding} bytes of a property set unread"
        );
    }
}

#[test]
fn object_references_resolve_inside_their_object_space() {
    let store = Store::parse(FIXTURE).expect("store parses");
    let dangling = dangling_references(&store);
    assert!(dangling.is_empty(), "dangling references: {dangling:?}");
}

#[test]
fn page_title_and_body_are_recovered() {
    let store = Store::parse(FIXTURE).expect("store parses");
    let pages = extract(&store);
    assert_eq!(pages.len(), 1, "expected exactly one page");
    let page = &pages[0];
    assert_eq!(page.title, "So good");
    let text: Vec<&str> = page
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Paragraph { text, .. } | Block::Bullet { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, ["This is one note 2016"]);
}

#[test]
fn markdown_starts_with_the_page_heading() {
    let store = Store::parse(FIXTURE).expect("store parses");
    let pages = extract(&store);
    let mut resolver = |image: &one_note_converter::doc::AssetRef| image.filename.clone();
    let markdown = one_note_converter::markdown::render(&pages, &mut resolver);
    assert_eq!(markdown, "# So good\n\nThis is one note 2016\n");
}

#[test]
fn guid_bytes_decode_with_the_specified_endianness() {
    let header = Header::parse(FIXTURE).expect("header parses");
    assert_eq!(
        header.guid_file_type.to_string(),
        "{7B5C52E4-D88C-4DA7-AEB1-5378D02996D3}"
    );
    assert_eq!(
        header.guid_file_format.to_string(),
        "{109ADD3F-911B-49F5-A5D0-1791EDC8AED8}"
    );
}

#[test]
fn table_of_contents_containers_are_detected_before_parsing() {
    let mut data = FIXTURE.to_vec();
    data[..16].copy_from_slice(&GUID_FILE_TYPE_ONETOC2.0);
    let header = Header::parse(&data).expect("header parses");
    assert_eq!(header.kind(), ContainerKind::TableOfContents);
}

#[test]
fn duplicate_objects_do_not_duplicate_text() {
    let Some(path) = fixture("test-tika-3970-dupetext.one") else {
        return;
    };
    let data = std::fs::read(&path).unwrap();
    let (_header, store) = one_note_converter::load(&data).unwrap();
    let pages = one_note_converter::doc::extract(&store);
    let mut resolver = |image: &one_note_converter::doc::AssetRef| image.filename.clone();
    let markdown = one_note_converter::markdown::render(&pages, &mut resolver);
    // The file carries heaps of redundant objects; each paragraph must still be
    // emitted once.
    assert_eq!(
        markdown.matches("Sunday morning").count(),
        1,
        "{:.400}",
        markdown
    );
    assert_eq!(markdown.matches("This little elephant").count(), 1);
}

fn fixture(name: &str) -> Option<std::path::PathBuf> {
    let path = std::path::Path::new("tests/fixtures").join(name);
    path.exists().then_some(path)
}

/// Wrap files in an uncompressed Microsoft Cabinet, the container of `.onepkg`.
fn cabinet(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let payload: Vec<u8> = entries
        .iter()
        .flat_map(|(_, bytes)| bytes.iter().copied())
        .collect();
    let files_at = 36 + 8;
    let mut table = Vec::new();
    let mut offset = 0u32;
    for (name, bytes) in entries {
        table.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        table.extend_from_slice(&offset.to_le_bytes());
        table.extend_from_slice(&0u16.to_le_bytes());
        table.extend_from_slice(&[0u8; 6]);
        table.extend_from_slice(name.as_bytes());
        table.push(0);
        offset += bytes.len() as u32;
    }
    let data_at = files_at + table.len();
    let mut blocks = Vec::new();
    let mut count = 0u16;
    for chunk in payload.chunks(32 * 1024) {
        blocks.extend_from_slice(&0u32.to_le_bytes());
        blocks.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
        blocks.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
        blocks.extend_from_slice(chunk);
        count += 1;
    }
    let total = data_at + blocks.len();
    let mut out = Vec::new();
    out.extend_from_slice(b"MSCF");
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(files_at as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&[3, 1]);
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(data_at as u32).to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&table);
    out.extend_from_slice(&blocks);
    out
}

#[test]
fn onepkg_extracts_and_converts_the_sections_inside() {
    let Some(fixture) = fixture("testOneNote2016.one") else {
        return;
    };
    let section = std::fs::read(&fixture).unwrap();
    let package = cabinet(&[("Section 1.one", &section)]);

    let dir = std::env::temp_dir().join(format!("onenote2md-onepkg-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let package_path = dir.join("notebook.onepkg");
    std::fs::write(&package_path, &package).unwrap();

    let out = dir.join("out");
    let report = dir.join("report.json");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_onenote2md"))
        .arg("onepkg")
        .arg(&package_path)
        .arg("-o")
        .arg(&out)
        .arg("--report")
        .arg(&report)
        .output()
        .expect("binary runs");
    assert!(
        status.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&status.stderr)
    );

    let written = out.join("Section 1").join("Section 1.md");
    let markdown = std::fs::read_to_string(&written)
        .unwrap_or_else(|error| panic!("{}: {error}", written.display()));
    assert!(markdown.starts_with("# "), "{:.200}", markdown);

    let json = std::fs::read_to_string(&report).unwrap();
    assert!(json.contains("\"kind\": \"package-section\""), "{json}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Links in the Markdown must name the files `images()` hands out, otherwise the
/// in-memory use (`markdown` + write the images) points at nothing.
#[test]
fn markdown_links_name_the_images_the_caller_gets() {
    let Some(path) = fixture("test-tika-3970-dupetext.one") else {
        return;
    };
    let data = std::fs::read(&path).unwrap();
    let section = one_note_converter::output::Section::read(data).unwrap();
    let names: std::collections::BTreeSet<String> = section
        .images()
        .into_iter()
        .map(|image| image.filename)
        .collect();
    let missing: std::collections::BTreeSet<String> =
        section.missing_images().into_iter().collect();
    assert!(!names.is_empty(), "fixture should carry pictures");

    let markdown = section.markdown(false);
    let mut links = Vec::new();
    let mut rest = markdown.as_str();
    while let Some(at) = rest.find("![") {
        let tail = &rest[at + 2..];
        let Some(close) = tail.find("](") else { break };
        let after = &tail[close + 2..];
        let Some(end) = after.find(')') else { break };
        links.push(after[..end].to_string());
        rest = &after[end..];
    }
    assert!(!links.is_empty(), "markdown should reference pictures");
    for link in links {
        // Links are percent-encoded; images and missing names are raw.
        let decoded = link
            .replace("%20", " ")
            .replace("%28", "(")
            .replace("%29", ")");
        assert!(
            names.contains(&decoded) || missing.contains(&decoded),
            "link {decoded:?} is neither delivered nor reported as missing"
        );
    }
}
