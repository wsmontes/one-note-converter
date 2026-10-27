//! Markdown rendering of the page/block model.

use crate::doc::{AssetRef, Block, Inline, Page};

/// Render pages to Markdown. `link` maps a picture reference to the URL that
/// the Markdown should point at; it may write the asset as a side effect.
pub fn render(pages: &[Page], link: &mut dyn FnMut(&AssetRef) -> String) -> String {
    render_with_options(pages, link, false)
}

/// Render pages, optionally prefixing each one with YAML front matter.
pub fn render_with_options(
    pages: &[Page],
    link: &mut dyn FnMut(&AssetRef) -> String,
    front_matter: bool,
) -> String {
    let mut out = String::new();
    for (index, page) in pages.iter().enumerate() {
        if index > 0 {
            if !front_matter {
                out.push_str("---\n\n");
            } else {
                out.push('\n');
            }
        }
        if front_matter {
            out.push_str(&page_front_matter(page));
        }
        out.push_str("# ");
        out.push_str(&oneline(&page.title));
        out.push_str("\n\n");
        for block in &page.blocks {
            match block {
                Block::Paragraph { text, level } => {
                    if *level > 0 {
                        out.push_str(&"    ".repeat(*level as usize));
                    }
                    out.push_str(text);
                    out.push_str("\n\n");
                }
                Block::Bullet {
                    text,
                    level,
                    marker,
                } => {
                    out.push_str(&"  ".repeat(*level as usize));
                    out.push_str(marker);
                    out.push(' ');
                    out.push_str(&oneline(text));
                    out.push('\n');
                }
                Block::Image(image) => {
                    let target = link(image);
                    let picture = format!("![{}]({})", escape_alt(&image.alt), target);
                    match &image.link {
                        Some(url) => out.push_str(&format!("[{picture}]({url})")),
                        None => out.push_str(&picture),
                    }
                    out.push_str("\n\n");
                }
                Block::Attachment(attachment) => {
                    let target = link(attachment);
                    let name = escape_alt(&attachment.filename);
                    out.push_str(&format!("[{name}]({target})\n\n"));
                }
                Block::Table { rows } => out.push_str(&render_table(rows, link)),
                Block::Note(text) => {
                    out.push_str("> ");
                    out.push_str(&oneline(text));
                    out.push_str("\n\n");
                }
            }
        }
    }
    out.trim_end().to_string() + "\n"
}

/// YAML front matter for one page: title, dates, authors and level.
fn page_front_matter(page: &Page) -> String {
    let mut out = String::from("---\n");
    out.push_str(&format!(
        "title: \"{}\"\n",
        yaml_escape(&oneline(&page.title))
    ));
    if let Some(created) = page.created {
        out.push_str(&format!("created: {}\n", iso8601(created)));
    }
    if let Some(modified) = page.modified {
        out.push_str(&format!("modified: {}\n", iso8601(modified)));
    }
    if !page.authors.is_empty() {
        out.push_str("authors:\n");
        for author in &page.authors {
            out.push_str(&format!("  - \"{}\"\n", yaml_escape(author)));
        }
    }
    if let Some(level) = page.level {
        out.push_str(&format!("level: {level}\n"));
    }
    out.push_str("---\n\n");
    out
}

/// Escape a value for a double-quoted YAML scalar.
fn yaml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Unix seconds to `YYYY-MM-DDTHH:MM:SSZ` (proleptic Gregorian, no leap seconds).
pub fn iso8601(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rem = seconds % 86_400;
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn render_table(rows: &[Vec<Vec<Inline>>], link: &mut dyn FnMut(&AssetRef) -> String) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut out = String::new();
    for (index, row) in rows.iter().enumerate() {
        out.push('|');
        for column in 0..width {
            out.push(' ');
            let cell = row
                .get(column)
                .map(|inlines| render_inlines(inlines, link))
                .unwrap_or_default();
            out.push_str(&cell.replace('|', "\\|"));
            out.push_str(" |");
        }
        out.push('\n');
        if index == 0 {
            out.push('|');
            for _ in 0..width {
                out.push_str(" --- |");
            }
            out.push('\n');
        }
    }
    out.push('\n');
    out
}

fn render_inlines(inlines: &[Inline], link: &mut dyn FnMut(&AssetRef) -> String) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(text) => {
                let text = oneline(text);
                if text.is_empty() {
                    continue;
                }
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(&text);
            }
            Inline::Image(image) => {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(&format!("![{}]({})", escape_alt(&image.alt), link(image)));
            }
        }
    }
    out
}

/// Alt text may contain brackets; escape them so the image syntax survives.
fn escape_alt(text: &str) -> String {
    let text = oneline(text);
    text.replace('[', "\\[").replace(']', "\\]")
}

fn oneline(text: &str) -> String {
    text.split(['\n', '\r'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_assets(_: &AssetRef) -> String {
        "unused.png".to_string()
    }

    #[test]
    fn bullets_keep_level_indentation() {
        let pages = vec![Page {
            title: "T".into(),
            blocks: vec![
                Block::Bullet {
                    text: "a".into(),
                    level: 0,
                    marker: "-".into(),
                },
                Block::Bullet {
                    text: "b".into(),
                    level: 2,
                    marker: "-".into(),
                },
            ],
            ..Page::default()
        }];
        let md = render(&pages, &mut no_assets);
        assert!(md.starts_with("# T\n"));
        assert!(md.contains("\n- a\n"));
        assert!(md.contains("\n    - b\n"));
    }

    fn cell(text: &str) -> Vec<Inline> {
        vec![Inline::Text(text.to_string())]
    }

    #[test]
    fn first_table_row_becomes_the_header() {
        let rows = vec![vec![cell("h1"), cell("h2")], vec![cell("c1"), cell("c2")]];
        let md = render_table(&rows, &mut no_assets);
        assert_eq!(md, "| h1 | h2 |\n| --- | --- |\n| c1 | c2 |\n\n");
    }

    #[test]
    fn pipes_inside_cells_are_escaped() {
        let rows = vec![vec![cell("a|b")]];
        assert!(render_table(&rows, &mut no_assets).contains("a\\|b"));
    }

    #[test]
    fn table_cell_images_use_the_resolver() {
        let rows = vec![vec![vec![
            Inline::Text("x".into()),
            Inline::Image(AssetRef {
                alt: "a".into(),
                filename: "p.png".into(),
                ..AssetRef::default()
            }),
        ]]];
        let mut resolver = |image: &AssetRef| format!("assets/{}", image.filename);
        let md = render_table(&rows, &mut resolver);
        assert!(md.contains("x ![a](assets/p.png)"));
    }

    #[test]
    fn iso8601_formats_known_instants() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(1_336_059_427), "2012-05-03T15:37:07Z");
        assert_eq!(iso8601(1_000_000_000), "2001-09-09T01:46:40Z");
    }

    #[test]
    fn front_matter_carries_title_dates_and_authors() {
        let pages = vec![Page {
            title: "A \"quoted\" page".into(),
            created: Some(0),
            modified: Some(86_400),
            authors: vec!["Ada".into()],
            level: Some(1),
            ..Page::default()
        }];
        let mut resolver = |image: &AssetRef| image.filename.clone();
        let md = render_with_options(&pages, &mut resolver, true);
        assert!(
            md.starts_with("---\ntitle: \"A \\\"quoted\\\" page\"\n"),
            "{md}"
        );
        assert!(md.contains("created: 1970-01-01T00:00:00Z\n"));
        assert!(md.contains("modified: 1970-01-02T00:00:00Z\n"));
        assert!(md.contains("authors:\n  - \"Ada\"\n"));
        assert!(md.contains("level: 1\n---\n\n# A \"quoted\" page\n"));
    }

    #[test]
    fn image_links_come_from_the_resolver() {
        let pages = vec![Page {
            title: "T".into(),
            blocks: vec![Block::Image(AssetRef {
                alt: "alt".into(),
                filename: "x.png".into(),
                ..AssetRef::default()
            })],
            ..Page::default()
        }];
        let mut resolver = |image: &AssetRef| format!("assets/{}", image.filename);
        let md = render(&pages, &mut resolver);
        assert!(md.contains("![alt](assets/x.png)"));
    }
}
