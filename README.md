# one-note-converter

`onenote2md` — converts Microsoft OneNote section files (`.one`) to Markdown, in Rust, with no
dependencies.

Only the plain *revision store* layout is supported: files whose header carries the format GUID
`{109ADD3F-911B-49F5-A5D0-1791EDC8AED8}` with a nil legacy file version.

The same revision store also travels wrapped in the [MS-FSSHTTPB] package store
(`{638DE92F-A6D4-4BC1-9A36-B3FC2511A5B7}`), which is how OneNote for Windows and OneDrive deliver
sections. The wrapped payload holds the same object model, but reaching it needs FSSHTTPB
data-element parsing (about 2 900 lines in a comparable implementation), so those files are rejected
with that explanation instead of being mis-parsed.

## Usage

```sh
cargo build --release

# structure report: object spaces, roots, object types, invariants, pages
./target/release/onenote2md inspect Section.one

# drill down while reverse engineering a file
./target/release/onenote2md inspect Section.one --decls                  # every object + blob range
./target/release/onenote2md inspect Section.one --object 2:14            # one property set
./target/release/onenote2md inspect Section.one --blob 0x138f8           # raw property set offset
./target/release/onenote2md inspect Section.one --locate 0x14000         # which object owns an offset
./target/release/onenote2md inspect Section.one --find-text "Sunday"     # which object carries text

# header dump (1024 bytes, field by field)
./target/release/onenote2md header Section.one

# Markdown to stdout, or to a file with pictures extracted next to it
./target/release/onenote2md convert Section.one -o Notes.md
./target/release/onenote2md convert Section.one -o out/Notes.md --assets out/images

# a whole notebook: every section listed in the table of contents
./target/release/onenote2md notebook "Open Notebook.onetoc2" -o out/notebook
```

`convert` writes pictures to `<section>_assets` next to the output file by default, naming them after
the OneNote image name plus a short GUID so duplicates such as `Untitled picture.png` stay distinct.

`notebook` follows the table of contents **given on the command line** (a folder with several
`.onetoc2` files does not make it guess); sections are written as `<name>.md` next to their assets,
and each level gets an `index.md` listing them in notebook order. In OneNote a section group is a
subfolder with its own `.onetoc2`, so groups become subfolders with their own index. A section that
the table of contents lists but the folder does not contain is reported and makes the command exit
non-zero, so a batch run cannot mistake a partial notebook for a complete one.

`--front-matter` puts `title`, `created`, `modified`, `authors` and `level` in YAML above each
page, in `convert`, `notebook` and `onepkg` alike. Dates come from the page node's own `LastModifiedTime` and the file's creation stamps, so a
page that never recorded one simply omits the key. `--report` writes a JSON summary
(`files[].{path,kind,pages,assets,failures,warnings}` plus `totals`), and `--strict` exits non-zero
when that summary holds any warning — packaged stores report their experimental status that way.

`onepkg` reads the cabinet directly: uncompressed folders and MSZIP (the deflate history carries
across blocks, so each block is inflated with the previous 32 KiB as dictionary). Quantum and LZX
are rejected with a message instead of guessed. Package paths are sanitised — absolute paths and
`..` are refused — before anything is written.

## Python bindings

`python/` is a PyO3 extension module over this crate, so the parser is shared, not
re-implemented:

```python
import onenote

section = onenote.read_section("Section.one")
section.pages[0].title
section.markdown(front_matter=True)
section.write("out/Section.md", "out/img")

notebook = onenote.read_notebook("Open Notebook.onetoc2")
notebook.all_sections()
notebook.write("out/notebook")

onenote.read_package("Open Notebook.onepkg").write("out/package")
```

Build it with `maturin develop --release` inside `python/` (wheel:
`maturin build --release`); see `python/README.md` for the full API.

## What the parser reads

The layout is documented by Microsoft as [MS-ONESTORE] (revision store) and [MS-ONE] (property ids,
object types, root roles). The chain implemented here:

```
header (1024 B)
└── root file node list                     -> one FileNode per object space
    ├── ObjectSpaceManifestListReferenceFND  -> object space manifest list
    │   └── RevisionManifestListReferenceFND -> revision manifests
    │       └── ObjectGroupListReferenceFND  -> object groups
    │           └── ObjectDeclaration*FND    -> BlobRef -> ObjectSpaceObjectPropSet
    └── FileDataStoreListReferenceFND        -> FileDataStoreObject (embedded pictures)
```

Details that the implementation had to get right, each verified against real files:

- `FileNode.Size` includes the 4-byte node header, and nodes are not aligned.
- File node lists are bounded by the transaction log: only the first `cTransactionsInLog`
  sentinel-terminated transactions are committed, and each `TransactionEntry` declares the active
  number of nodes for one list. Nodes past that length belong to an incomplete transaction and are
  dropped (`ONENOTE_TRACE_NODES=1` reports it; no file in the test corpus needs it).
- The hashed chunk list (`fcrHashedChunkList`) holds `HashedChunkDescriptor2FND` nodes: a `BlobRef`
  plus its MD5. `header` and `inspect` report the entry count and hashes.
- A file node list spans fragments; only the non-final fragments carry `ChunkTerminatorFND`, so the
  final fragment ends in zero padding instead.
- `PropertySet.cProperties` is **2 bytes**. One level down, `prtArrayOfPropertyValues` has a
  4-byte element count followed by a **single** `prid` describing the type shared by every element
  (not one prid per element, which shifts the whole payload).
- The high bits of a `PropertyID` carry its type, which determines the size and location of the
  value; strings are length-prefixed UTF-16, `TextExtendedAscii` is single-byte Windows-1252.
- `RootObjectReference3FND.oidRoot` is an **ExtendedGUID**, not a CompactID: it resolves to
  `(gid_table_index << 8) | n`. Root role 1 is the content root, role 2 metadata, role 4 revision
  metadata.
- Pictures live in `FileDataStoreObject` blobs framed by two fixed GUIDs.
- Rich text embeds RTF-ish field codes starting with a noncharacter in `U+FDD0..=U+FDEF`; the
  `HYPERLINK "…"` instruction is dropped and the field result kept.

## Which content becomes Markdown

Each page is a page object space, identified by its role-1 root (a `jcidPageManifestNode`), and
pages are ordered by the section's page series. Page content is walked through every child
collection the format uses (`ElementChildNodes`, `ContentChildNodes`, `StructureElementChildNodes`),
so a node's type does not have to be guessed to reach its children.

Pages are rendered as `# Title`, paragraphs (indented by `OutlineElementChildLevel`), bullets, tables
whose cells may hold text and pictures, images, and embedded files as links (the attachment bytes are
extracted next to the pictures). Whole-paragraph bold/italic/underline come from
the element's `ParagraphStyle` object. A `jcidImageNode`'s `RichEditTextUnicode` — the text OneNote
keeps for the picture — becomes the image's alt text.

The output reflects the **current** state: superseded revisions and version-history/conflict spaces
are not rendered. When the current declaration of an object carries no text at all, the text of a
superseded declaration of the same object is used instead (this is what recovers the alt text of
embedded XPS/PDF previews).

## Validated on

44 files from two independent corpora (Tika test documents, and the samples of
[onenote.rs](https://github.com/msiemens/onenote.rs) — Windows exports, Joplin, ink, German):

| Corpus | Files | Revision store | Converted | Legacy container |
| --- | --- | --- | --- | --- |
| Apache Tika test documents | 10 | 9 | 7 | 1 |
| onenote.rs samples | 34 | 7 | 7 | 25, plus 2 Git-LFS pointer stubs |

Across the 14 files that converted: 24 pages, 189 text blocks and 71 assets extracted (pictures and
attachments), **0** objects skipped, **0** partially decoded property sets, **0** malformed property
set padding, and no panics. The only dangling object reference in the corpus is inside a deliberately
corrupted Tika fuzz fixture.

The two revision-store files that do not convert are the `.onetoc2` table of contents (wrong file
type for a section) and that corrupted fuzz fixture. The 26 remaining files carry the
`{638DE92F-A6D4-4BC1-9A36-B3FC2511A5B7}` package store: the same object model, wrapped in
[MS-FSSHTTPB] data elements (the layout OneNote for Windows and OneDrive produce, and what Joplin
writes). Supporting them means implementing that wrapper, which is the largest remaining piece of
work.

## Verified against

Apache Tika's `OneNoteParserTest` is used as an independent oracle, since its assertions were
written against the same fixtures by a different implementation:

| Tika assertion (fixture) | Result |
| --- | --- |
| `testOneNote2016`: "So good", "This is one note 2016" | exact match, asserted by `tests/onestore.rs` |
| `testOneNote2`: two page GUIDs | 2 pages |
| `testOneNote2`: "Section1HeaderTitle", "Section1TextArea1", "Section1TextArea2", "wow this is neat" | present |
| `test-tika-3970-dupetext`: "Sunday morning" appears once | exactly 1 |
| `test-tika-3970-dupetext`: one page GUID | 1 page |
| `test-tika-4303-Chinese-notes`: `中文标题` | present |
| `testOneNote1`: two pages | 2 pages |
| `assertNoJunk`: no font names, no mis-decoded UTF-16 | no "Calibri", no C1 controls, no stray CJK in Latin fixtures |

Known difference: Tika merges text from *all* revision contexts, so it also reports text that exists
only in another context's version of the same object (for example "neat info about totally killin it
bro" in `testOneNote2`, where the current page holds "wow this is neat" for that node). This
converter renders the current revision and does not merge divergent edits.

## Known limits

- Empty paragraphs that exist only for vertical spacing are not reproduced.
- Note tags appear as a `[label]` prefix; the tag's own colour/shape is not reproduced.
- Ink strokes are not rendered: the page shows a `> ink drawing not rendered` marker where they were.
- Front matter (dates, authors) is not written yet.

- `Charset` other than the default code page (0, Windows-1252) is not honoured; East Asian
  single-byte text would need the matching code page.
- Sections wrapped in the [MS-FSSHTTPB] package store are rejected (see above); `.onetoc2` files are
  rejected as the wrong file type.
- Files whose live content is only described by the transaction log are not replayed; the parser
  reads object groups.

## Provenance and license

This crate is MIT licensed (see `LICENSE`). All formats were implemented from the published
specifications — [MS-ONESTORE] (revision store, property ids) and [MS-FSSHTTPB] (package store) —
and validated against real files plus the Apache Tika test assertions.

`onenote-parser` (msiemens, MPL-2.0) was consulted as an independent implementation to cross-check
observed behaviour while reverse engineering the package store. The code here is an independent
implementation written from the specifications, not a port: the bit layouts and element orderings are
transcribed from [MS-FSSHTTPB]/[MS-ONESTORE] and documented in `src/fsshttpb.rs`, so the module does
not depend on that project.

## Tests

```sh
cargo test
```

The integration test parses `tests/fixtures/testOneNote2016.one` (from the Apache Tika test corpus,
Apache-2.0 — see `tests/fixtures/README.md`) and checks the header, that every property set consumes
its blob modulo `0..=7` bytes of padding, that no object reference dangles, and the exact title,
body and Markdown produced.
