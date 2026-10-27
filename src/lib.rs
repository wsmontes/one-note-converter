//! Parser and Markdown renderer for Microsoft OneNote `.one` section files.
//!
//! The on-disk layout follows the published revision store specification,
//! [MS-ONESTORE]; the meaning of property ids and object types follows
//! [MS-ONE]. `onenote2md` is the command line front end.
//!
//! ```no_run
//! let data = std::fs::read("section.one").unwrap();
//! let store = one_note_converter::store::Store::parse(&data).unwrap();
//! let pages = one_note_converter::doc::extract(&store);
//! let markdown = one_note_converter::markdown::render(&pages, &mut |image| {
//!     image.filename.clone()
//! });
//! println!("{markdown}");
//! ```

pub mod cab;
pub mod doc;
pub mod filenode;
pub mod fsshttpb;
pub mod guid;
pub mod header;
pub mod markdown;
pub mod notebook;
pub mod output;
pub mod packaging;
pub mod prop;
pub mod reader;
pub mod store;
pub mod temp;

use header::Header;
use reader::ParseResult;

/// Load any supported section file into the object model, whether it is a plain
/// revision store or a [MS-FSSHTTPB] package store.
pub fn load(data: &[u8]) -> ParseResult<(Header, store::Store)> {
    let header = Header::parse(data)?;
    match header.kind() {
        header::ContainerKind::Unknown => Err(format!(
            "unrecognized container: file type {}, file format {}",
            header.guid_file_type, header.guid_file_format
        )),
        header::ContainerKind::PackageStore => {
            let package = fsshttpb::parse(data)?;
            let store = packaging::load_store(data, header.clone(), &package)?;
            Ok((header, store))
        }
        // A `.onetoc2` table of contents uses the same revision store layout, so
        // it loads the same way and is walked as a notebook by the caller.
        _ => {
            let store = store::Store::parse(data)?;
            Ok((header, store))
        }
    }
}
