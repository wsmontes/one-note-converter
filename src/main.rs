//! Command line entry point: `header`, `inspect` and `convert`.

use one_note_converter::doc;
use one_note_converter::header::{ContainerKind, Header};
use one_note_converter::output::{package_path, root_level};
use one_note_converter::{prop, store};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
onenote2md — convert Microsoft OneNote section files to Markdown

USAGE:
    onenote2md header  <file.one>
    onenote2md inspect <file.one> [--decls] [--object <space>:<oid>]
                                  [--blob <hex>] [--locate <hex>] [--find-text <text>]
    onenote2md convert <file.one> [-o <out.md>] [--assets <dir>] [--front-matter]
    onenote2md notebook <notebook.onetoc2> [-o <dir>] [--report <json>] [--strict]
    onenote2md onepkg   <notebook.onepkg> [-o <dir>] [--report <json>] [--strict]

COMMANDS:
    header   dump the 1024-byte revision store header
    inspect  structure summary: object spaces, roots, object types, invariants,
             pages; the options drill into one declaration, property set, file
             offset or text occurrence
    convert  render Markdown to stdout or -o <out.md>; pictures are written to
             --assets (default: <section>_assets next to the output)
    onepkg   extract a OneNote package (Microsoft Cabinet) and convert the
             sections or notebooks inside it into -o <dir>
    notebook convert every section listed in an .onetoc2 into a folder tree with
             an index.md (default output: <notebook>_md next to the file)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Default)]
struct Options<'a> {
    input: Option<&'a str>,
    output: Option<&'a str>,
    assets: Option<&'a str>,
    /// `inspect --object <space-index>:<oid>` dumps one object's property set.
    object: Option<&'a str>,
    /// `inspect --find-text <needle>` locates text anywhere in the store.
    find_text: Option<&'a str>,
    /// `inspect --locate <hex-offset>` finds the object owning a file offset.
    locate: Option<&'a str>,
    /// `inspect --decls` lists every parsed declaration with its blob range.
    decls: bool,
    /// `inspect --blob <hex-offset>` dumps a property set at a raw offset.
    blob: Option<&'a str>,
    /// `inspect --find-prop <hex>` lists objects carrying a property id.
    find_prop: Option<&'a str>,
    /// `inspect --runs <space>:<oid>` explains how one text node formats.
    runs: Option<&'a str>,
    /// `inspect --find-jcid <hex>` lists objects of one object type.
    find_jcid: Option<&'a str>,
    /// `convert --front-matter` prefixes each page with YAML metadata.
    front_matter: bool,
    /// `--report <path>` writes a JSON summary of what was converted.
    report: Option<&'a str>,
    /// `--strict` turns any warning into a non-zero exit status.
    strict: bool,
}

/// One converted file, as recorded in the `--report` summary.
#[derive(Default, Clone)]
struct ReportEntry {
    path: String,
    kind: &'static str,
    pages: usize,
    assets: usize,
    failures: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Default)]
struct Report {
    entries: Vec<ReportEntry>,
}

impl Report {
    fn add(&mut self, entry: ReportEntry) {
        self.entries.push(entry);
    }

    /// Number of failures plus warnings across every entry.
    fn problems(&self) -> usize {
        self.entries
            .iter()
            .map(|entry| entry.failures.len() + entry.warnings.len())
            .sum()
    }

    fn to_json(&self) -> String {
        let pages: usize = self.entries.iter().map(|entry| entry.pages).sum();
        let assets: usize = self.entries.iter().map(|entry| entry.assets).sum();
        let mut out = String::from("{\n  \"files\": [");
        for (index, entry) in self.entries.iter().enumerate() {
            out.push_str(if index == 0 { "\n" } else { ",\n" });
            out.push_str("    {");
            out.push_str(&format!("\"path\": \"{}\", ", json_escape(&entry.path)));
            out.push_str(&format!("\"kind\": \"{}\", ", entry.kind));
            out.push_str(&format!("\"pages\": {}, ", entry.pages));
            out.push_str(&format!("\"assets\": {}, ", entry.assets));
            out.push_str(&format!("\"failures\": {}, ", json_array(&entry.failures)));
            out.push_str(&format!("\"warnings\": {}", json_array(&entry.warnings)));
            out.push('}');
        }
        out.push_str(&format!(
            "\n  ],\n  \"totals\": {{\"files\": {}, \"pages\": {pages}, \"assets\": {assets}, \"problems\": {}}}\n}}\n",
            self.entries.len(),
            self.problems()
        ));
        out
    }

    fn write(&self, path: &str) -> Result<(), String> {
        std::fs::write(path, self.to_json())
            .map_err(|error| format!("cannot write {path}: {error}"))
    }
}

fn json_array(items: &[String]) -> String {
    if items.is_empty() {
        return "[]".to_string();
    }
    let body: Vec<String> = items
        .iter()
        .map(|item| format!("\"{}\"", json_escape(item)))
        .collect();
    format!("[{}]", body.join(", "))
}

fn json_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn run(args: &[String]) -> Result<(), String> {
    let Some(command) = args.first() else {
        print!("{USAGE}");
        return Ok(());
    };
    if command == "-h" || command == "--help" {
        print!("{USAGE}");
        return Ok(());
    }
    let mut options = Options::default();
    let mut rest = &args[1..];
    while let Some(arg) = rest.first() {
        match arg.as_str() {
            "-o" | "--output" => {
                options.output = Some(
                    rest.get(1)
                        .ok_or_else(|| "-o requires a path".to_string())?
                        .as_str(),
                );
                rest = &rest[2..];
            }
            "--blob" => {
                options.blob = Some(
                    rest.get(1)
                        .ok_or_else(|| "--blob requires a hex offset".to_string())?
                        .as_str(),
                );
                rest = &rest[2..];
            }
            "--front-matter" => {
                options.front_matter = true;
                rest = &rest[1..];
            }
            "--report" => {
                options.report = Some(
                    rest.get(1)
                        .ok_or_else(|| "--report requires a path".to_string())?
                        .as_str(),
                );
                rest = &rest[2..];
            }
            "--strict" => {
                options.strict = true;
                rest = &rest[1..];
            }
            "--find-jcid" => {
                options.find_jcid = Some(
                    rest.get(1)
                        .ok_or_else(|| "--find-jcid requires a hex JCID".to_string())?
                        .as_str(),
                );
                rest = &rest[2..];
            }
            "--runs" => {
                options.runs = Some(
                    rest.get(1)
                        .ok_or_else(|| "--runs requires <space>:<oid>".to_string())?
                        .as_str(),
                );
                rest = &rest[2..];
            }
            "--find-prop" => {
                options.find_prop = Some(
                    rest.get(1)
                        .ok_or_else(|| "--find-prop requires a hex property id".to_string())?
                        .as_str(),
                );
                rest = &rest[2..];
            }
            "--decls" => {
                options.decls = true;
                rest = &rest[1..];
            }
            "--locate" => {
                options.locate = Some(
                    rest.get(1)
                        .ok_or_else(|| "--locate requires a hex file offset".to_string())?
                        .as_str(),
                );
                rest = &rest[2..];
            }
            "--find-text" => {
                options.find_text = Some(
                    rest.get(1)
                        .ok_or_else(|| "--find-text requires a string".to_string())?
                        .as_str(),
                );
                rest = &rest[2..];
            }
            "--object" => {
                options.object = Some(
                    rest.get(1)
                        .ok_or_else(|| "--object requires <space-index>:<oid>".to_string())?
                        .as_str(),
                );
                rest = &rest[2..];
            }
            "--assets" => {
                options.assets = Some(
                    rest.get(1)
                        .ok_or_else(|| "--assets requires a directory".to_string())?
                        .as_str(),
                );
                rest = &rest[2..];
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown option {other}\n\n{USAGE}"));
            }
            other => {
                if options.input.is_some() {
                    return Err(format!("unexpected argument {other}"));
                }
                options.input = Some(other);
                rest = &rest[1..];
            }
        }
    }
    let path = options
        .input
        .ok_or_else(|| format!("missing input file\n\n{USAGE}"))?;
    let data = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;

    let mut report = Report::default();
    match command.as_str() {
        "header" => print_header(&data)?,
        "inspect" => inspect(
            &data,
            options.object,
            options.find_text,
            options.locate,
            options.decls,
            options.blob,
            options.find_prop,
            options.runs,
            options.find_jcid,
        )?,
        command => {
            let result = match command {
                "convert" => convert(&data, Path::new(path), &options, &mut report),
                "onepkg" => onepkg(&data, Path::new(path), &options, &mut report),
                "notebook" => notebook(&data, Path::new(path), &options, &mut report),
                other => return Err(format!("unknown command {other}\n\n{USAGE}")),
            };
            let kind = match command {
                "convert" => "section",
                "onepkg" => "package",
                _ => "notebook",
            };
            if let Err(message) = &result {
                // A failed run is exactly when the report matters, so record
                // the failure and write the file before returning the error.
                report.add(ReportEntry {
                    path: path.to_string(),
                    kind,
                    failures: vec![message.clone()],
                    ..ReportEntry::default()
                });
            }
            if let Some(report_path) = options.report {
                // A report that cannot be written must not hide the real error.
                if let Err(error) = report.write(report_path) {
                    if result.is_ok() {
                        return Err(error);
                    }
                    eprintln!("warning: {error}");
                }
            }
            result?;
            if options.strict && report.problems() > 0 {
                return Err(format!(
                    "--strict: {} problem(s) reported; see {}",
                    report.problems(),
                    options.report.unwrap_or("the log")
                ));
            }
            return Ok(());
        }
    };
    Ok(())
}

fn print_header(data: &[u8]) -> Result<(), String> {
    let header = Header::parse(data)?;
    println!("file size            {}", data.len());
    println!("container            {:?}", header.kind());
    println!("guidFileType         {}", header.guid_file_type);
    println!("guidFile             {}", header.guid_file);
    println!("guidFileFormat       {}", header.guid_file_format);
    println!(
        "ffvLastCode          {:#04x} ({})",
        header.ffv_last_code,
        header.writer_generation()
    );
    println!("cTransactionsInLog   {}", header.c_transactions_in_log);
    if let Ok(store) = one_note_converter::store::Store::parse(data) {
        println!(
            "activeListCounts     {} lists declared by the log; {} transactions active",
            store.list_counts.len(),
            store.transactions
        );
        println!("hashedChunks         {}", store.hashed_chunks.len());
        for chunk in store.hashed_chunks.iter().take(3) {
            println!(
                "  {:#x}..{:#x} md5 {}",
                chunk.stp,
                chunk.stp + chunk.cb as u64,
                chunk.hash_hex()
            );
        }
    }
    println!("guidAncestor         {}", header.guid_ancestor);
    println!("guidFileVersion      {}", header.guid_file_version);
    println!("crcName              {:#010x}", header.crc_name);
    println!(
        "fcrFileNodeListRoot  stp={:#x} cb={:#x}",
        header.fcr_file_node_list_root.stp, header.fcr_file_node_list_root.cb
    );
    println!(
        "fcrTransactionLog    stp={:#x} cb={:#x}",
        header.fcr_transaction_log.stp, header.fcr_transaction_log.cb
    );
    for (name, reference) in [
        ("fcrHashedChunkList", header.fcr_hashed_chunk_list),
        ("fcrFreeChunkList", header.fcr_free_chunk_list),
    ] {
        if reference.is_valid() {
            println!("{name:<21}stp={:#x} cb={:#x}", reference.stp, reference.cb);
        } else {
            println!("{name:<21}absent");
        }
    }
    println!(
        "cbExpectedFileLength {} ({})",
        header.cb_expected_file_length,
        if header.file_length_matches(data.len()) {
            "matches file size"
        } else {
            "MISMATCH — file may be truncated"
        }
    );
    println!("nFileVersionGen      {}", header.n_file_version_generation);
    println!(
        "cbFreeSpaceInFreeChunkList {}",
        header.cb_free_space_in_free_chunk_list
    );
    println!("bnCreated            {}", header.bn_created);
    println!("bnLastWroteToFile    {}", header.bn_last_wrote);
    Ok(())
}

fn inspect_package(data: &[u8]) -> Result<(), String> {
    let package = one_note_converter::fsshttpb::parse(data)?;
    println!("data elements        {}", package.data_element_count);
    if let Some(index) = &package.storage_index {
        println!(
            "storage index        {} manifest, {} cell, {} revision mappings",
            index.manifest_mappings.len(),
            index.cell_mappings.len(),
            index.revision_mappings.len()
        );
    } else {
        println!("storage index        absent");
    }
    println!("storage manifests    {}", package.storage_manifests.len());
    println!("cell manifests       {}", package.cell_manifests.len());
    println!("revision manifests   {}", package.revision_manifests.len());
    println!("object groups        {}", package.object_groups.len());
    println!("declared objects     {}", package.declaration_count());
    println!("object data bytes    {}", package.object_bytes());
    println!("object data blobs    {}", package.blobs.len());
    println!("data element fragments {}", package.fragments.len());
    let store = one_note_converter::packaging::load_store(data, Header::parse(data)?, &package)?;
    println!("\ntranslated object spaces: {}", store.spaces.len());
    for (index, space) in store.spaces.iter().enumerate() {
        let kinds = prop::count_jcids(space.objects.values().map(|object| object.jcid));
        println!(
            "  [{index}] {} ({}) — {} objects, {} roots, gid {}",
            space.gosid,
            space.kind(),
            space.objects.len(),
            space.roots.len(),
            space.gid_table.len()
        );
        for root in &space.roots {
            let jcid = space.objects.get(&root.oid).map(|o| o.jcid).unwrap_or(0);
            println!(
                "        root role {} -> oid {} ({})",
                root.role,
                root.oid,
                prop::jcid_name(jcid)
            );
        }
        for (name, count) in kinds.iter().take(6) {
            println!("        {count:5} {name}");
        }
    }
    for warning in store.warnings.iter().take(5) {
        println!("  warning: {warning}");
    }
    Ok(())
}

fn inspect(
    data: &[u8],
    object: Option<&str>,
    find_text: Option<&str>,
    locate: Option<&str>,
    decls: bool,
    blob: Option<&str>,
    find_prop: Option<&str>,
    runs: Option<&str>,
    find_jcid: Option<&str>,
) -> Result<(), String> {
    let container = Header::parse(data)?.kind();
    if container == ContainerKind::PackageStore {
        inspect_package(data)?;
        println!();
    }
    if container == ContainerKind::PackagedRevisionStore {
        return Err(
            "this revision store carries its payload in a package store located after the \
             header; that offset is not implemented yet"
                .to_string(),
        );
    }
    let (_header, store) = one_note_converter::load(data)?;

    println!("file size            {}", store.file_size);
    println!(
        "transactions         {} active log entries, {} list counts",
        store.transactions,
        store.list_counts.len()
    );
    println!("hashed chunks        {}", store.hashed_chunks.len());
    println!(
        "expected length      {} ({})",
        store.header.cb_expected_file_length,
        if store.header.file_length_matches(store.file_size) {
            "ok"
        } else {
            "MISMATCH"
        }
    );
    println!("object spaces        {}", store.spaces.len());
    println!("objects              {}", store.all_objects().count());
    println!("file data objects    {}", store.file_data.len());

    let histogram = prop::count_jcids(store.all_objects().map(|(_, obj)| obj.jcid));
    println!("\nobject types:");
    for (name, count) in &histogram {
        println!("  {count:5}  {name}");
    }

    let padding = store::count_blob_padding(&store);
    println!("\nproperty set padding (blob - consumed, must be 0..7):");
    for (pad, count) in &padding {
        let flag = if *pad >= 8 { "  <-- malformed" } else { "" };
        println!("  {count:5}  {pad} bytes{flag}");
    }

    let dangling = store::dangling_references(&store);
    println!("\ndangling object references: {}", dangling.len());
    for (guid, oid) in dangling.iter().take(10) {
        println!("  {guid} oid {oid}");
    }

    println!("\nobject spaces and roots:");
    for (index, space) in store.spaces.iter().enumerate() {
        println!(
            "  [{index}] {} ({}) — {} objects, {} file data refs, {} fragments, {} gid entries",
            space.gosid,
            space.kind(),
            space.objects.len(),
            space.file_data_refs.len(),
            space.manifest_list_fragments,
            space.gid_table.len()
        );
        if let Some(root) = space.content_root() {
            println!(
                "        content graph: {} of {} objects reachable from the role-1 root",
                store::reachable_from(space, root),
                space.objects.len()
            );
        }
        if space.encrypted_revisions > 0 {
            println!(
                "        {} encrypted revision(s): content omitted",
                space.encrypted_revisions
            );
        }
        if space.superseded_roots > 0 {
            println!(
                "        {} root declarations from superseded revisions",
                space.superseded_roots
            );
        }
        for root in &space.roots {
            let object = space.objects.get(&root.oid);
            match object {
                Some(object) => println!(
                    "        root role {} -> oid {} ({})",
                    root.role,
                    root.oid,
                    prop::jcid_name(object.jcid)
                ),
                None => println!(
                    "        root role {} -> oid {} (unresolved)",
                    root.role, root.oid
                ),
            }
        }
    }

    if let Some(offset) = blob {
        let offset = usize::from_str_radix(offset.trim_start_matches("0x"), 16)
            .map_err(|_| format!("bad offset {offset}"))?;
        let parsed = prop::parse_object_prop_set(data, offset)
            .map_err(|e| format!("cannot parse property set at {offset:#x}: {e}"))?;
        println!(
            "\nproperty set at {offset:#x}: consumed {} bytes",
            parsed.consumed
        );
        for (pid, value) in &parsed.properties.properties {
            let rendered = match value {
                prop::Value::Bytes(bytes) => {
                    let text = prop::decode_utf16(bytes);
                    format!(
                        "<{} bytes> {:?}",
                        bytes.len(),
                        text.chars().take(60).collect::<String>()
                    )
                }
                prop::Value::Array(sets) => format!("Array of {} sets", sets.len()),
                prop::Value::Set(set) => format!("Set of {} properties", set.properties.len()),
                other => format!("{other:?}"),
            };
            println!(
                "  {:#08x} {:<32} {rendered}",
                pid.0,
                prop::pid_name(pid.id())
            );
        }
    }

    if decls {
        for (index, space) in store.spaces.iter().enumerate() {
            for object in space.objects.values() {
                println!(
                    "space {index} oid {:5} {:22} blob {:#010x}..{:#010x} variants {:?}",
                    object.oid,
                    prop::jcid_name(object.jcid),
                    object.blob_offset,
                    object.blob_offset + object.blob_size,
                    object
                        .text_variants
                        .iter()
                        .map(|text| text.chars().take(30).collect::<String>())
                        .collect::<Vec<_>>()
                );
            }
        }
    }

    if let Some(offset) = locate {
        let offset = usize::from_str_radix(offset.trim_start_matches("0x"), 16)
            .map_err(|_| format!("bad offset {offset}"))?;
        let mut hit = false;
        for (index, space) in store.spaces.iter().enumerate() {
            for object in space.objects.values() {
                if offset >= object.blob_offset && offset < object.blob_offset + object.blob_size {
                    println!(
                        "offset {offset:#x} -> space {index} oid {} ({}) blob {:#x}..{:#x}",
                        object.oid,
                        prop::jcid_name(object.jcid),
                        object.blob_offset,
                        object.blob_offset + object.blob_size
                    );
                    hit = true;
                }
            }
        }
        if !hit {
            println!("offset {offset:#x} is not inside any object property set");
        }
    }

    if let Some(jcid_text) = find_jcid {
        let jcid = u32::from_str_radix(jcid_text.trim_start_matches("0x"), 16)
            .map_err(|_| format!("bad JCID {jcid_text}"))?;
        let mut total = 0;
        for (index, space) in store.spaces.iter().enumerate() {
            for object in space.objects.values() {
                if object.jcid != jcid {
                    continue;
                }
                total += 1;
                if total <= 8 {
                    println!(
                        "jcid {jcid:#x} in space {index} oid {} ({} properties)",
                        object.oid,
                        object.props.properties.len()
                    );
                }
            }
        }
        println!("objects with jcid {jcid:#x}: {total}");
    }

    if let Some(spec) = runs {
        let (space, oid) = spec
            .split_once(':')
            .ok_or_else(|| "--runs expects <space>:<oid>".to_string())?;
        let space_index: usize = space.parse().map_err(|_| format!("bad space {space}"))?;
        let oid: u32 = oid.parse().map_err(|_| format!("bad oid {oid}"))?;
        let space = store
            .spaces
            .get(space_index)
            .ok_or_else(|| format!("no space {space_index}"))?;
        let object = space
            .objects
            .get(&oid)
            .ok_or_else(|| format!("no object {oid}"))?;
        println!("\nruns of object {oid} ({})", prop::jcid_name(object.jcid));
        println!("  plain text: {:?}", doc::object_text_snippet(object));
        match object.props.bytes(prop::pid::TEXT_RUN_INDEX) {
            Some(bytes) => println!(
                "  TextRunIndex: {} bytes {:02x?}",
                bytes.len(),
                &bytes[..bytes.len().min(24)]
            ),
            None => println!("  TextRunIndex: absent"),
        }
        match object.props.oids(prop::pid::TEXT_RUN_FORMATTING) {
            Some(ids) => println!(
                "  TextRunFormatting: {} entries {:?}",
                ids.len(),
                &ids[..ids.len().min(8)]
            ),
            None => println!("  TextRunFormatting: absent"),
        }
        println!(
            "  formatted: {:?}",
            doc::formatted_text_snippet(space, object)
        );
    }

    if let Some(pid_text) = find_prop {
        let pid = u32::from_str_radix(pid_text.trim_start_matches("0x"), 16)
            .map_err(|_| format!("bad property id {pid_text}"))?;
        let mut total = 0;
        for (index, space) in store.spaces.iter().enumerate() {
            for object in space.objects.values() {
                if object.props.get(pid).is_none() {
                    continue;
                }
                total += 1;
                if total <= 8 {
                    println!(
                        "prop {pid:#x} in space {index} oid {} ({})",
                        object.oid,
                        prop::jcid_name(object.jcid)
                    );
                }
            }
        }
        println!("objects with property {pid:#x}: {total}");
    }

    if let Some(needle) = find_text {
        for (index, space) in store.spaces.iter().enumerate() {
            for object in space.objects.values() {
                let Some(text) = doc::object_text_snippet(object) else {
                    continue;
                };
                if !text.contains(needle) {
                    continue;
                }
                println!(
                    "text {needle:?} -> space {index} oid {} ({}) {text:?}",
                    object.oid,
                    prop::jcid_name(object.jcid)
                );
            }
        }
    }

    let pages = doc::extract(&store);
    let notebook_entries = one_note_converter::notebook::read(&store);
    if !notebook_entries.is_empty() {
        let mut lines = Vec::new();
        one_note_converter::notebook::describe(&notebook_entries, 0, &mut lines);
        println!("\nnotebook entries: {}", lines.len());
        for line in lines.iter().take(40) {
            println!("  {line}");
        }
    }

    println!("\npages: {}", pages.len());
    for page in &pages {
        let images = page
            .blocks
            .iter()
            .filter(|block| matches!(block, doc::Block::Image(_)))
            .count();
        let tables = page
            .blocks
            .iter()
            .filter(|block| matches!(block, doc::Block::Table { .. }))
            .count();
        println!(
            "  #{} [space {}] oid {} — {} blocks ({} images, {} tables)",
            page.title,
            page.space_index,
            page.oid,
            page.blocks.len(),
            images,
            tables
        );
    }

    if let Some(spec) = object {
        let (space, oid) = spec
            .split_once(':')
            .ok_or_else(|| "--object expects <space-index>:<oid>".to_string())?;
        let space_index: usize = space
            .parse()
            .map_err(|_| format!("bad space index {space}"))?;
        let oid: u32 = oid.parse().map_err(|_| format!("bad oid {oid}"))?;
        dump_object(&store, space_index, oid)?;
    }

    if !store.warnings.is_empty() {
        println!("\nwarnings:");
        for warning in &store.warnings {
            println!("  {warning}");
        }
    }
    Ok(())
}

fn dump_object(store: &store::Store, space_index: usize, oid: u32) -> Result<(), String> {
    let space = store
        .spaces
        .get(space_index)
        .ok_or_else(|| format!("no object space {space_index}"))?;
    let object = space
        .objects
        .get(&oid)
        .ok_or_else(|| format!("no object {oid} in space {space_index}"))?;
    println!(
        "\nobject {oid} in space {space_index}: jcid {:#010x} ({})",
        object.jcid,
        prop::jcid_name(object.jcid)
    );
    for (pid, value) in &object.props.properties {
        let rendered = match value {
            prop::Value::Bytes(bytes) => {
                let text = prop::decode_utf16(bytes);
                let printable =
                    !text.is_empty() && text.chars().all(|c| !c.is_control() || c == '\n');
                let hex: String = bytes
                    .iter()
                    .take(12)
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                if printable {
                    format!("{text:?} [{hex}] {} bytes", bytes.len())
                } else {
                    format!("<{} bytes> [{hex}]", bytes.len())
                }
            }
            other => format!("{other:?}"),
        };
        println!(
            "  {:#08x} {:<32} {rendered}",
            pid.0,
            prop::pid_name(pid.id())
        );
    }
    if let Some(reference) = space.file_data_refs.get(&oid) {
        println!(
            "  file data: {:?} ext {:?}",
            reference.reference, reference.extension
        );
    }
    Ok(())
}

/// Convert every section of a notebook listed in its `.onetoc2`.
fn notebook(
    data: &[u8],
    toc_path: &Path,
    options: &Options<'_>,
    report: &mut Report,
) -> Result<(), String> {
    let _ = data;
    let base = toc_path
        .parent()
        .map(Path::to_path_buf)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("."));
    let root = match options.output {
        Some(dir) => PathBuf::from(dir),
        None => base.join(format!(
            "{}_md",
            toc_path
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
                .unwrap_or_else(|| "notebook".to_string())
        )),
    };
    notebook_into(&base, root, report, Some(toc_path), options.front_matter)
}

/// Convert every section reachable from a table of contents into `root`.
/// `toc` names the table of contents of the top level; deeper levels find their
/// own.
fn notebook_into(
    base: &Path,
    root: PathBuf,
    report: &mut Report,
    toc: Option<&Path>,
    front_matter: bool,
) -> Result<(), String> {
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("cannot create {}: {error}", root.display()))?;

    let stats = one_note_converter::notebook::write_tree(base, &root, front_matter, toc)?;
    for section in &stats.sections {
        report.add(ReportEntry {
            path: section.input.display().to_string(),
            kind: "section",
            pages: section.pages,
            assets: section.assets,
            failures: section.failures.clone(),
            ..ReportEntry::default()
        });
    }
    if stats.converted == 0 && stats.failures.is_empty() {
        return Err("no sections found for this notebook".to_string());
    }
    if !stats.failures.is_empty() {
        // Sections listed by the notebook could not be read: fail, so a batch
        // run cannot mistake a partial notebook for a complete one.
        return Err(format!(
            "wrote {} section(s) into {}, but {} problem(s):\n  {}",
            stats.converted,
            root.display(),
            stats.failures.len(),
            stats.failures.join("\n  ")
        ));
    }
    eprintln!(
        "wrote {} section(s) into {}",
        stats.converted,
        root.display()
    );
    Ok(())
}

/// Convert a OneNote package (`.onepkg`, a Microsoft Cabinet) by extracting it
/// to a scratch directory and converting what the package holds.
fn onepkg(
    data: &[u8],
    input: &Path,
    options: &Options<'_>,
    report: &mut Report,
) -> Result<(), String> {
    let cabinet = one_note_converter::cab::Cabinet::parse(data)?;
    if cabinet.entries.is_empty() {
        return Err("package contains no files".to_string());
    }
    let root = match options.output {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(format!(
            "{}_md",
            input
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
                .unwrap_or_else(|| "package".to_string())
        )),
    };
    let staging = std::env::temp_dir().join(format!(
        "onenote2md-pkg-{}-{}",
        std::process::id(),
        input
            .file_stem()
            .map(|stem| stem.to_string_lossy().replace(['/', ' '], "_"))
            .unwrap_or_default()
    ));
    let _ = std::fs::remove_dir_all(&staging);

    let extracted: Result<(), String> = (|| {
        for (index, entry) in cabinet.entries.iter().enumerate() {
            let name = package_path(&entry.name)
                .ok_or_else(|| format!("{}: unsafe path inside the package", entry.name))?;
            let path = staging.join(&name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
            }
            let bytes = cabinet
                .read(index)
                .ok_or_else(|| format!("{}: payload is missing", entry.name))?;
            std::fs::write(&path, bytes)
                .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        }
        Ok(())
    })();

    let result = extracted.and_then(|()| {
        let tocs = root_level(&staging, "onetoc2");
        if tocs.is_empty() {
            let mut failures = Vec::new();
            let sections = root_level(&staging, "one");
            if sections.is_empty() {
                return Err("package contains no sections or table of contents".to_string());
            }
            for section in sections {
                let stem = section
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().to_string())
                    .unwrap_or_else(|| "section".to_string());
                let bytes = std::fs::read(&section)
                    .map_err(|error| format!("{}: {error}", section.display()))?;
                let out_dir = root.join(&stem);
                let assets = out_dir.join("assets");
                std::fs::create_dir_all(&out_dir)
                    .map_err(|error| format!("cannot create {}: {error}", out_dir.display()))?;
                let output = out_dir.join(format!("{stem}.md"));
                match one_note_converter::output::Section::read(bytes)
                    .and_then(|section| section.write(&output, &assets, options.front_matter))
                {
                    Ok(stats) => {
                        failures.extend(
                            stats
                                .failures
                                .iter()
                                .map(|error| format!("{stem}: {error}")),
                        );
                        report.add(ReportEntry {
                            path: format!("{stem}.one"),
                            kind: "package-section",
                            pages: stats.pages,
                            assets: stats.assets,
                            ..ReportEntry::default()
                        });
                    }
                    Err(error) => failures.push(format!("{stem}: {error}")),
                }
            }
            if !failures.is_empty() {
                return Err(format!(
                    "{} section(s) could not be converted:\n  {}",
                    failures.len(),
                    failures.join("\n  ")
                ));
            }
            eprintln!(
                "wrote {} section(s) into {}",
                report.entries.len(),
                root.display()
            );
            return Ok(());
        }
        for toc in tocs {
            let stem = toc
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
                .unwrap_or_else(|| "notebook".to_string());
            let base = toc
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| staging.clone());
            notebook_into(
                &base,
                root.join(&stem),
                report,
                Some(&toc),
                options.front_matter,
            )?;
        }
        Ok(())
    });
    let _ = std::fs::remove_dir_all(&staging);
    result
}

fn convert(
    data: &[u8],
    input: &Path,
    options: &Options<'_>,
    report: &mut Report,
) -> Result<(), String> {
    let section = one_note_converter::output::Section::read(data.to_vec())?;
    let kind = if section.store().header.kind() == ContainerKind::PackageStore {
        "package"
    } else {
        "section"
    };
    if kind == "package" {
        eprintln!(
            "note: [MS-FSSHTTPB] package store support is experimental — page bodies may be \
             incomplete"
        );
    }

    let output = options.output.map(PathBuf::from);
    let base_dir = output
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("."));
    let assets_dir = match options.assets {
        Some(dir) => PathBuf::from(dir),
        None => base_dir.join(format!(
            "{}_assets",
            input
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
                .unwrap_or_else(|| "onenote".to_string())
        )),
    };
    let prefix = one_note_converter::output::relative_link_prefix(&base_dir, &assets_dir);

    let (markdown, stats) = section.materialize(&assets_dir, &prefix, options.front_matter)?;
    let mut warnings: Vec<String> = section.store().warnings.clone();
    if kind == "package" {
        warnings.push(
            "[MS-FSSHTTPB] package store support is experimental — page bodies may be incomplete"
                .to_string(),
        );
    }
    report.add(ReportEntry {
        path: input.display().to_string(),
        kind,
        pages: stats.pages,
        assets: stats.assets,
        failures: stats.failures.clone(),
        warnings,
    });
    if !stats.failures.is_empty() {
        return Err(format!(
            "could not write pictures: {}",
            stats.failures.join("; ")
        ));
    }

    match &output {
        Some(path) => {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
                }
            }
            std::fs::write(path, markdown)
                .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
            let mut note = String::new();
            let dangling = one_note_converter::store::dangling_references(section.store()).len();
            if dangling > 0 {
                note.push_str(&format!(
                    " — {dangling} object references could not be resolved"
                ));
            }
            let store_warnings = &section.store().warnings;
            if !store_warnings.is_empty() {
                note = format!(" — {} warnings:", store_warnings.len());
                for warning in store_warnings.iter().take(3) {
                    note.push_str("\n  ");
                    note.push_str(warning);
                }
                if store_warnings.len() > 3 {
                    note.push_str(&format!("\n  … {} more", store_warnings.len() - 3));
                }
            }
            eprintln!(
                "wrote {} ({} pages, {} assets in {}){note}",
                path.display(),
                stats.pages,
                stats.assets,
                assets_dir.display()
            );
        }
        None => print!("{markdown}"),
    }
    Ok(())
}
