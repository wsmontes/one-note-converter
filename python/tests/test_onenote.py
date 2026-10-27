"""Behavioural tests for the `onenote` Python bindings.

They run against the Rust parser through the extension module: every assertion
here is about the public Python surface, not the internals.
"""

from __future__ import annotations

import re
import struct
from pathlib import Path

import pytest

import onenote

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
FIXTURES = REPO / "tests" / "fixtures"
NOTEBOOK = HERE / "data" / "notebook"


def section_file() -> Path:
    path = FIXTURES / "test-tika-3970-dupetext.one"
    if not path.is_file():
        pytest.skip(f"fixture missing: {path}")
    return path


def cabinet(entries: list[tuple[str, bytes]]) -> bytes:
    """Write a store-method Microsoft Cabinet, independently of the Rust reader."""
    payload = b"".join(data for _, data in entries)
    files_at = 36 + 8
    table = bytearray()
    offset = 0
    for name, data in entries:
        table += struct.pack("<IIHHHH", len(data), offset, 0, 0, 0, 0x20)
        table += name.encode("utf-8") + b"\0"
        offset += len(data)
    data_at = files_at + len(table)
    blocks = bytearray()
    count = 0
    for start in range(0, len(payload), 32768):
        chunk = payload[start : start + 32768]
        blocks += struct.pack("<IHH", 0, len(chunk), len(chunk)) + chunk
        count += 1
    total = data_at + len(blocks)
    header = b"MSCF" + struct.pack("<III", 0, total, 0)
    header += struct.pack("<II", files_at, 0)
    header += bytes([3, 1]) + struct.pack("<HHHHH", 1, len(entries), 0, 0, 0)
    header += struct.pack("<IHH", data_at, count, 0)
    return bytes(header + table + blocks)


def test_section_exposes_pages_metadata_and_markdown() -> None:
    section = onenote.read_section(section_file())

    assert section.page_count >= 1
    page = section.pages[0]
    assert page.title
    assert isinstance(page.authors, list)
    assert page.created is None or page.created.endswith("Z")

    markdown = section.markdown()
    assert markdown.startswith("# ")
    # The file carries heaps of duplicate objects; text must still appear once.
    assert markdown.count("Sunday morning") == 1


def test_section_reads_bytes_as_well_as_paths() -> None:
    path = section_file()
    from_path = onenote.read_section(path)
    from_bytes = onenote.read_section(path.read_bytes())

    assert from_bytes.page_count == from_path.page_count
    assert from_bytes.markdown() == from_path.markdown()


def test_section_images_and_write(tmp_path: Path) -> None:
    section = onenote.read_section(section_file())
    images = section.images()

    stats = section.write(tmp_path / "out.md", tmp_path / "img", front_matter=True)
    assert stats.pages == section.page_count
    assert stats.assets == len(images)
    assert stats.failures == []

    text = (tmp_path / "out.md").read_text(encoding="utf-8")
    assert text.startswith("---\ntitle: ")
    assert "img/" in text
    written = sorted(p.name for p in (tmp_path / "img").iterdir())
    assert written == sorted(image.filename for image in images)
    assert all(image.data for image in images)


def links_in(markdown: str) -> set[str]:
    """Every `![](target)` target, percent-decoded."""
    from urllib.parse import unquote

    return {unquote(link) for link in re.findall(r"!\[[^\]]*\]\(([^)]+)\)", markdown)}


def test_markdown_links_match_image_names(tmp_path: Path) -> None:
    """Each link is either delivered by `images()` or reported as missing."""
    section = onenote.read_section(section_file())
    names = {image.filename for image in section.images()}
    missing = set(section.missing_images())

    for page in section.pages:
        assert {image.filename for image in page.images} <= names

    links = links_in(section.markdown())
    assert links, "markdown should reference the pictures"
    assert links <= names | missing, f"{links - names - missing} are unaccounted for"

    # Writing delivers exactly the images, and every delivered link exists.
    section.write(tmp_path / "out.md", tmp_path / "img")
    written = {path.name for path in (tmp_path / "img").iterdir()}
    assert written == names
    assert (links & names) <= written
    # Delivered names carry the GUID suffix, not the raw title from the file.
    assert any(re.search(r"-[0-9A-F]{8}\.", name) for name in written), sorted(written)


def test_front_matter_can_be_switched_off() -> None:
    section = onenote.read_section(section_file())
    assert section.markdown(front_matter=False).startswith("# ")
    assert section.markdown(front_matter=True).startswith("---\n")


def test_corrupt_input_raises_value_error() -> None:
    with pytest.raises(ValueError):
        onenote.read_section(b"not a one file at all")


def test_notebook_tree_matches_what_it_writes(tmp_path: Path) -> None:
    notebook = onenote.read_notebook(NOTEBOOK / "Open Notebook.onetoc2")

    assert notebook.errors == []
    assert [section.name for section in notebook.sections] == ["Nova Seção 1.one"]
    assert [group.name for group in notebook.groups] == ["Group Section 1"]
    nested = notebook.groups[0]
    assert [group.name for group in nested.groups] == ["Group Section 1-a"]

    every = notebook.all_sections()
    assert len(every) == 4

    stats = notebook.write(tmp_path)
    assert stats.converted == len(every)
    assert stats.failures == []
    assert len(stats.sections) == len(every)

    root_index = (tmp_path / "index.md").read_text(encoding="utf-8")
    assert "- [Nova Seção 1](Nova%20Seção%201.md)" in root_index
    assert "- **[Group Section 1](Group%20Section%201/index.md)**" in root_index
    assert (
        tmp_path / "Group Section 1" / "Group Section 1-a" / "Subsection 3.md"
    ).is_file()


def test_notebook_front_matter_reaches_every_section(tmp_path: Path) -> None:
    notebook = onenote.read_notebook(NOTEBOOK / "Open Notebook.onetoc2")
    notebook.write(tmp_path, front_matter=True)
    written = list(tmp_path.rglob("*.md"))
    assert written
    for path in written:
        text = path.read_text(encoding="utf-8")
        if path.name == "index.md":
            assert text.startswith("- ") or text == ""
        else:
            assert text.startswith("---\ntitle: ")


def test_package_reads_sections_from_a_cabinet(tmp_path: Path) -> None:
    package_path = tmp_path / "notebook.onepkg"
    package_path.write_bytes(cabinet([("Section 1.one", section_file().read_bytes())]))

    package = onenote.read_package(package_path)
    assert package.entries == ["Section 1.one"]
    sections = package.sections()
    assert len(sections) == 1
    assert sections[0].page_count >= 1

    out = tmp_path / "converted"
    stats = package.write(out, front_matter=True)
    assert stats.converted == 1
    assert stats.failures == []
    text = (out / "Section 1" / "Section 1.md").read_text(encoding="utf-8")
    assert text.startswith("---\ntitle: ")


def test_package_with_table_of_contents_becomes_a_tree(tmp_path: Path) -> None:
    entries = [(path.name, path.read_bytes()) for path in NOTEBOOK.iterdir() if path.is_file()]
    entries.append(
        (
            "Group Section 1/Group Section 1.onetoc2",
            (NOTEBOOK / "Group Section 1" / "Group Section 1.onetoc2").read_bytes(),
        )
    )
    entries.append(
        (
            "Group Section 1/Subsection 1.one",
            (NOTEBOOK / "Group Section 1" / "Subsection 1.one").read_bytes(),
        )
    )
    package_path = tmp_path / "with-toc.onepkg"
    package_path.write_bytes(cabinet(entries))

    package = onenote.read_package(package_path)
    out = tmp_path / "tree"
    stats = package.write(out)

    assert stats.converted >= 2
    assert (out / "Open Notebook" / "index.md").is_file()
    assert (out / "Open Notebook" / "Nova Seção 1.md").is_file()
