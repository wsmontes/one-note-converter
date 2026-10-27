# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-10-26

First release: a CLI, a Rust library and Python bindings over one parser.

### Added

- **Sections** (`.one`): reads `[MS-ONESTORE]` revision stores — header, file node
  lists, property sets, object spaces, revisions, object groups, file data.
- **Packages** (`.onepkg`): reads Microsoft Cabinets with stored and MSZIP
  folders; Quantum and LZX are rejected with a message.
- **Notebooks** (`.onetoc2`): the table of contents is honoured exactly as given,
  section groups become subfolders with their own `index.md`, and entries whose
  file or folder is missing are reported instead of skipped.
- **Markdown output**: page headings, paragraphs, numbered and bulleted lists
  with nesting, tables, pictures (extracted with stable deduplicated names),
  attachments, ink markers, note tags, and inline run formatting (bold, italic,
  strike, underline, colour, highlight, hyperlinks).
- **YAML front matter** per page: title, creation and modification dates,
  authors, page level.
- **Diagnostics**: `header` and `inspect` commands (object spaces, declarations,
  property sets, blob ranges, text search, hashed chunk list, transaction log).
- **Packaged stores** (`[MS-FSSHTTPB]`): data elements, cells, object data and
  the storage index are translated into the same object model as plain stores.
- **Python bindings** (`onenote`): `read_section`, `read_notebook`,
  `read_package` with `Section`/`Page`/`Image`/`Notebook`/`Package`, abi3 wheels
  for CPython 3.8+.
- **Machine-readable runs**: `--report` writes a JSON summary, `--strict` turns
  any warning into a non-zero exit status.

### Notes

- Encrypted sections are detected and reported; their content is not decrypted.
- `cacheRead` style provider quirks and other converter-side details live in
  `PLAN.md`.

[Unreleased]: https://github.com/wsmontes/one-note-converter/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/wsmontes/one-note-converter/releases/tag/v0.1.0
