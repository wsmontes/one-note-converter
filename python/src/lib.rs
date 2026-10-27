//! Python bindings for the OneNote reader.
//!
//! The object model mirrors the Rust library: `Section` owns pages and pictures,
//! `Notebook` mirrors a `.onetoc2` tree, `Package` is a `.onepkg` cabinet. All
//! parsing and Markdown rendering is the same code the `onenote2md` CLI runs.

use std::path::PathBuf;

use one_note_converter::doc::Page;
use one_note_converter::markdown;
use one_note_converter::notebook;
use one_note_converter::output::{self, Section as RustSection};
use pyo3::exceptions::{PyOSError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyByteArray, PyBytes, PyString};

fn error(message: String) -> PyErr {
    PyValueError::new_err(message)
}

/// Read a section from a path, or from already loaded bytes.
fn read_source(source: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    if let Ok(bytes) = source.downcast::<PyBytes>() {
        return Ok(bytes.as_bytes().to_vec());
    }
    if let Ok(buffer) = source.downcast::<PyByteArray>() {
        return Ok(buffer.to_vec());
    }
    if source.is_instance_of::<PyString>() {
        let path: PathBuf = source.extract()?;
        return std::fs::read(&path).map_err(|cause| {
            PyOSError::new_err(format!("cannot read {}: {cause}", path.display()))
        });
    }
    // Anything else that yields a path (os.PathLike).
    let path: PathBuf = source.extract()?;
    std::fs::read(&path)
        .map_err(|cause| PyOSError::new_err(format!("cannot read {}: {cause}", path.display())))
}

fn py_image(py: Python<'_>, filename: String, bytes: &[u8]) -> PyImage {
    PyImage {
        filename,
        data: PyBytes::new(py, bytes).unbind(),
    }
}

/// Bytes of every picture referenced by one page, named exactly as
/// `Section::markdown` and `Section::images` name them.
fn page_pictures<'a>(section: &'a RustSection, page: &Page) -> Vec<(String, &'a [u8])> {
    let names = section.image_names();
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for asset in output::page_images(page) {
        let Some(guid) = asset.guid else {
            continue;
        };
        if !seen.insert(guid) {
            continue;
        }
        let Some(name) = names.get(&guid) else {
            continue;
        };
        if let Some(bytes) =
            one_note_converter::store::file_data_bytes(section.store(), section.bytes(), guid)
        {
            out.push((name.clone(), bytes));
        }
    }
    out
}

#[pyclass(module = "onenote", name = "Image")]
struct PyImage {
    #[pyo3(get)]
    filename: String,
    data: Py<PyBytes>,
}

#[pymethods]
impl PyImage {
    /// Picture bytes.
    #[getter]
    fn data(&self, py: Python<'_>) -> Py<PyBytes> {
        self.data.clone_ref(py)
    }

    fn __len__(&self, py: Python<'_>) -> usize {
        self.data.as_bytes(py).len()
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        format!(
            "<onenote.Image {:?} {} bytes>",
            self.filename,
            self.data.as_bytes(py).len()
        )
    }
}

#[pyclass(module = "onenote", name = "Page")]
struct PyPage {
    #[pyo3(get)]
    title: String,
    #[pyo3(get)]
    level: Option<u32>,
    #[pyo3(get)]
    created: Option<String>,
    #[pyo3(get)]
    modified: Option<String>,
    #[pyo3(get)]
    authors: Vec<String>,
    pictures: Vec<(String, Py<PyBytes>)>,
}

#[pymethods]
impl PyPage {
    /// Pictures referenced by this page.
    #[getter]
    fn images(&self, py: Python<'_>) -> Vec<PyImage> {
        self.pictures
            .iter()
            .map(|(filename, data)| PyImage {
                filename: filename.clone(),
                data: data.clone_ref(py),
            })
            .collect()
    }

    fn __repr__(&self) -> String {
        format!(
            "<onenote.Page {:?} level={:?} images={}>",
            self.title,
            self.level,
            self.pictures.len()
        )
    }
}

#[pyclass(module = "onenote", name = "SectionStats")]
struct PySectionStats {
    #[pyo3(get)]
    pages: usize,
    #[pyo3(get)]
    assets: usize,
    #[pyo3(get)]
    failures: Vec<String>,
}

#[pymethods]
impl PySectionStats {
    fn __repr__(&self) -> String {
        format!(
            "<onenote.SectionStats pages={} assets={} failures={}>",
            self.pages,
            self.assets,
            self.failures.len()
        )
    }
}

#[pyclass(module = "onenote", name = "Section")]
struct PySection {
    inner: RustSection,
}

impl PySection {
    fn page_object(py: Python<'_>, section: &RustSection, page: &Page) -> PyPage {
        PyPage {
            title: page.title.clone(),
            level: page.level,
            created: page.created.map(markdown::iso8601),
            modified: page.modified.map(markdown::iso8601),
            authors: page.authors.clone(),
            pictures: page_pictures(section, page)
                .into_iter()
                .map(|(name, bytes)| (name, PyBytes::new(py, bytes).unbind()))
                .collect(),
        }
    }
}

#[pymethods]
impl PySection {
    /// Number of pages in the section.
    #[getter]
    fn page_count(&self) -> usize {
        self.inner.pages().len()
    }

    /// Every page, with its metadata and pictures.
    #[getter]
    fn pages(&self, py: Python<'_>) -> Vec<PyPage> {
        self.inner
            .pages()
            .iter()
            .map(|page| PySection::page_object(py, &self.inner, page))
            .collect()
    }

    /// Markdown for every page; pictures link to their bare file names.
    #[pyo3(signature = (front_matter = false))]
    fn markdown(&self, front_matter: bool) -> String {
        self.inner.markdown(front_matter)
    }

    /// Pictures Markdown links to whose payload the file does not carry (web
    /// clippings, external caches). They cannot be written.
    fn missing_images(&self) -> Vec<String> {
        self.inner.missing_images()
    }

    /// Every embedded picture, with the names `write` would use.
    fn images(&self, py: Python<'_>) -> Vec<PyImage> {
        self.inner
            .images()
            .into_iter()
            .map(|image| py_image(py, image.filename, &image.bytes))
            .collect()
    }

    /// Write `<output>` and its pictures into `assets`.
    #[pyo3(signature = (output, assets, front_matter = false))]
    fn write(
        &self,
        output: PathBuf,
        assets: PathBuf,
        front_matter: bool,
    ) -> PyResult<PySectionStats> {
        let stats = self
            .inner
            .write(&output, &assets, front_matter)
            .map_err(error)?;
        Ok(PySectionStats {
            pages: stats.pages,
            assets: stats.assets,
            failures: stats.failures,
        })
    }

    fn __repr__(&self) -> String {
        format!("<onenote.Section pages={}>", self.inner.pages().len())
    }
}

#[pyclass(module = "onenote", name = "NotebookSection")]
#[derive(Clone)]
struct PyNotebookSection {
    #[pyo3(get)]
    name: String,
    #[pyo3(get)]
    path: PathBuf,
}

#[pymethods]
impl PyNotebookSection {
    fn __repr__(&self) -> String {
        format!("<onenote.NotebookSection {:?}>", self.name)
    }
}

#[pyclass(module = "onenote", name = "SectionReport")]
#[derive(Clone)]
struct PySectionReport {
    #[pyo3(get)]
    input: PathBuf,
    #[pyo3(get)]
    pages: usize,
    #[pyo3(get)]
    assets: usize,
    #[pyo3(get)]
    failures: Vec<String>,
}

#[pymethods]
impl PySectionReport {
    fn __repr__(&self) -> String {
        format!(
            "<onenote.SectionReport {:?} pages={} assets={}>",
            self.input, self.pages, self.assets
        )
    }
}

#[pyclass(module = "onenote", name = "TreeStats")]
struct PyTreeStats {
    #[pyo3(get)]
    converted: usize,
    #[pyo3(get)]
    sections: Vec<PySectionReport>,
    #[pyo3(get)]
    failures: Vec<String>,
}

#[pymethods]
impl PyTreeStats {
    fn __repr__(&self) -> String {
        format!(
            "<onenote.TreeStats converted={} failures={}>",
            self.converted,
            self.failures.len()
        )
    }
}

fn section_report(section: notebook::SectionReport) -> PySectionReport {
    PySectionReport {
        input: section.input,
        pages: section.pages,
        assets: section.assets,
        failures: section.failures,
    }
}

#[pyclass(module = "onenote", name = "Notebook")]
struct PyNotebook {
    inner: notebook::Notebook,
}

#[pymethods]
impl PyNotebook {
    #[getter]
    fn name(&self) -> String {
        self.inner.name()
    }

    #[getter]
    fn folder(&self) -> PathBuf {
        self.inner.folder.clone()
    }

    #[getter]
    fn toc(&self) -> Option<PathBuf> {
        self.inner.toc.clone()
    }

    /// Sections listed by this level's table of contents.
    #[getter]
    fn sections(&self) -> Vec<PyNotebookSection> {
        self.inner
            .sections
            .iter()
            .map(|section| PyNotebookSection {
                name: section.name.clone(),
                path: section.file.clone(),
            })
            .collect()
    }

    /// Section groups below this level.
    #[getter]
    fn groups(&self) -> Vec<PyNotebook> {
        self.inner
            .groups
            .iter()
            .map(|group| PyNotebook {
                inner: group.clone(),
            })
            .collect()
    }

    /// Entries the table of contents lists whose file or folder is missing.
    #[getter]
    fn errors(&self) -> Vec<String> {
        self.inner.errors.clone()
    }

    /// Every section in the tree, depth first.
    fn all_sections(&self) -> Vec<PyNotebookSection> {
        self.inner
            .all_sections()
            .into_iter()
            .map(|section| PyNotebookSection {
                name: section.name,
                path: section.file,
            })
            .collect()
    }

    /// Convert the whole tree: one `.md` per section plus an `index.md` per level.
    #[pyo3(signature = (out, front_matter = false))]
    fn write(&self, out: PathBuf, front_matter: bool) -> PyResult<PyTreeStats> {
        let Some(toc) = self.inner.toc.clone() else {
            return Err(error("notebook has no table of contents".to_string()));
        };
        let stats = notebook::write_tree(&self.inner.folder, &out, front_matter, Some(&toc))
            .map_err(error)?;
        Ok(PyTreeStats {
            converted: stats.converted,
            sections: stats.sections.into_iter().map(section_report).collect(),
            failures: stats.failures,
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "<onenote.Notebook {:?} sections={} groups={}>",
            self.inner.name(),
            self.inner.sections.len(),
            self.inner.groups.len()
        )
    }
}

#[pyclass(module = "onenote", name = "Package")]
struct PyPackage {
    path: PathBuf,
    entries: Vec<String>,
    payloads: Vec<Vec<u8>>,
}

impl PyPackage {
    fn write_sections(&self, out: &std::path::Path, front_matter: bool) -> PyResult<PyTreeStats> {
        let mut sections = Vec::new();
        let mut failures = Vec::new();
        let staging = std::env::temp_dir().join(format!("onenote-py-pkg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&staging);
        let extracted = self.extract_to(staging.clone())?;
        let mut entries: Vec<PathBuf> = extracted
            .into_iter()
            .filter(|path| path.extension().is_some_and(|ext| ext == "one"))
            .collect();
        entries.sort();
        if entries.is_empty() {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error(
                "package contains no sections or table of contents".to_string(),
            ));
        }
        for entry in entries {
            let stem = entry
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
                .unwrap_or_else(|| "section".to_string());
            let section = std::fs::read(&entry)
                .map_err(|cause| PyOSError::new_err(format!("{}: {cause}", entry.display())))
                .and_then(|bytes| RustSection::read(bytes).map_err(error))?;
            let dir = out.join(&stem);
            let stats = section
                .write(
                    &dir.join(format!("{stem}.md")),
                    &dir.join("assets"),
                    front_matter,
                )
                .map_err(error)?;
            failures.extend(stats.failures.iter().cloned());
            sections.push(PySectionReport {
                input: entry,
                pages: stats.pages,
                assets: stats.assets,
                failures: stats.failures,
            });
        }
        let _ = std::fs::remove_dir_all(&staging);
        Ok(PyTreeStats {
            converted: sections.len(),
            sections,
            failures,
        })
    }
}

#[pymethods]
impl PyPackage {
    #[getter]
    fn path(&self) -> PathBuf {
        self.path.clone()
    }

    #[getter]
    fn entries(&self) -> Vec<String> {
        self.entries.clone()
    }

    /// Parse every `.one` entry inside the package.
    fn sections(&self) -> PyResult<Vec<PySection>> {
        let mut sections = Vec::new();
        for (index, name) in self.entries.iter().enumerate() {
            if !name.to_ascii_lowercase().ends_with(".one") {
                continue;
            }
            let section = RustSection::read(self.payloads[index].clone()).map_err(error)?;
            sections.push(PySection { inner: section });
        }
        Ok(sections)
    }

    /// Write every entry under `dir` and return the extracted paths.
    fn extract_to(&self, dir: PathBuf) -> PyResult<Vec<PathBuf>> {
        let mut written = Vec::new();
        for (index, name) in self.entries.iter().enumerate() {
            let relative = output::package_path(name)
                .ok_or_else(|| error(format!("{name}: unsafe path inside the package")))?;
            let path = dir.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|cause| {
                    PyOSError::new_err(format!("cannot create {}: {cause}", parent.display()))
                })?;
            }
            std::fs::write(&path, &self.payloads[index]).map_err(|cause| {
                PyOSError::new_err(format!("cannot write {}: {cause}", path.display()))
            })?;
            written.push(path);
        }
        Ok(written)
    }

    /// Convert the package: its notebooks when it carries a TOC, otherwise each
    /// section on its own.
    #[pyo3(signature = (out, front_matter = false))]
    fn write(&self, out: PathBuf, front_matter: bool) -> PyResult<PyTreeStats> {
        let staging = std::env::temp_dir().join(format!("onenote-py-pkg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&staging);
        self.extract_to(staging.clone())?;
        let tocs = output::root_level(&staging, "onetoc2");
        let result = if tocs.is_empty() {
            self.write_sections(&out, front_matter)
        } else {
            let mut combined = PyTreeStats {
                converted: 0,
                sections: Vec::new(),
                failures: Vec::new(),
            };
            for toc in tocs {
                let base = toc
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or_else(|| staging.clone());
                let stem = toc
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().to_string())
                    .unwrap_or_else(|| "notebook".to_string());
                match notebook::write_tree(&base, &out.join(stem), front_matter, Some(&toc)) {
                    Ok(stats) => {
                        combined.converted += stats.converted;
                        combined.failures.extend(stats.failures);
                        combined
                            .sections
                            .extend(stats.sections.into_iter().map(section_report));
                    }
                    Err(cause) => {
                        let _ = std::fs::remove_dir_all(&staging);
                        return Err(error(cause));
                    }
                }
            }
            Ok(combined)
        };
        let _ = std::fs::remove_dir_all(&staging);
        result
    }

    fn __repr__(&self) -> String {
        format!(
            "<onenote.Package {:?} entries={}>",
            self.path,
            self.entries.len()
        )
    }
}

/// Parse a `.one` section from a path or from `bytes`.
#[pyfunction]
fn read_section(source: &Bound<'_, PyAny>) -> PyResult<PySection> {
    let data = read_source(source)?;
    let section = RustSection::read(data).map_err(error)?;
    Ok(PySection { inner: section })
}

/// Parse a `.onetoc2` notebook tree.
#[pyfunction]
fn read_notebook(toc: PathBuf) -> PyResult<PyNotebook> {
    let inner = notebook::Notebook::read(&toc).map_err(error)?;
    Ok(PyNotebook { inner })
}

/// Parse a `.onepkg` package (a Microsoft Cabinet).
#[pyfunction]
fn read_package(path: PathBuf) -> PyResult<PyPackage> {
    let data = std::fs::read(&path)
        .map_err(|cause| PyOSError::new_err(format!("cannot read {}: {cause}", path.display())))?;
    let cabinet = one_note_converter::cab::Cabinet::parse(&data).map_err(error)?;
    let mut entries = Vec::new();
    let mut payloads = Vec::new();
    for index in 0..cabinet.entries.len() {
        let bytes = cabinet
            .read(index)
            .ok_or_else(|| {
                error(format!(
                    "{}: payload is missing",
                    cabinet.entries[index].name
                ))
            })?
            .to_vec();
        entries.push(cabinet.entries[index].name.clone());
        payloads.push(bytes);
    }
    Ok(PyPackage {
        path,
        entries,
        payloads,
    })
}

#[pymodule]
fn onenote(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_class::<PyImage>()?;
    m.add_class::<PyPage>()?;
    m.add_class::<PySection>()?;
    m.add_class::<PySectionStats>()?;
    m.add_class::<PyNotebook>()?;
    m.add_class::<PyNotebookSection>()?;
    m.add_class::<PySectionReport>()?;
    m.add_class::<PyTreeStats>()?;
    m.add_class::<PyPackage>()?;
    m.add_function(wrap_pyfunction!(read_section, m)?)?;
    m.add_function(wrap_pyfunction!(read_notebook, m)?)?;
    m.add_function(wrap_pyfunction!(read_package, m)?)?;
    Ok(())
}
