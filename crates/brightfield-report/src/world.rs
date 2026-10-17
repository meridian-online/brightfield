//! The Typst world a report compiles in: the bundled fonts, the one source
//! file and the chart pictures, and nothing else.
//!
//! The world reads no file from disk, fetches nothing and serves no package:
//! an import of a package, or of any file it does not hold, is an error. It
//! has no clock either, so the same report compiles to the same pages.

use std::collections::HashMap;
use std::sync::LazyLock;

use typst::diag::{FileError, FileResult, PackageError};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};

/// The fonts the report is set in, and the book Typst finds them by.
static FONTS: LazyLock<(Vec<Font>, LazyHash<FontBook>)> = LazyLock::new(|| {
    use meridian_design::fonts::{
        INTER_BOLD, INTER_MEDIUM, INTER_REGULAR, INTER_SEMIBOLD, JBM_REGULAR,
    };
    let fonts: Vec<Font> = [
        INTER_REGULAR,
        INTER_MEDIUM,
        INTER_SEMIBOLD,
        INTER_BOLD,
        JBM_REGULAR,
    ]
    .into_iter()
    .flat_map(|data| Font::iter(Bytes::new(data)))
    .collect();
    let book = LazyHash::new(FontBook::from_fonts(&fonts));
    (fonts, book)
});

/// Typst's standard library, built once.
static LIBRARY: LazyLock<LazyHash<Library>> = LazyLock::new(|| LazyHash::new(Library::default()));

/// The id of the file at `path` in the project, the only root this world has.
pub(crate) fn project_file(path: &str) -> FileId {
    let vpath = VirtualPath::new(path).expect("a path this crate writes is valid");
    FileId::new(RootedPath::new(VirtualRoot::Project, vpath))
}

/// One report's world: its source and its chart pictures.
pub(crate) struct ReportWorld {
    main: Source,
    files: HashMap<FileId, Bytes>,
}

impl ReportWorld {
    /// A world whose main file is `source` and which holds `files`, each at
    /// its path in the project.
    pub(crate) fn new(source: String, files: Vec<(String, Vec<u8>)>) -> Self {
        let main = Source::new(project_file("/main.typ"), source);
        let files = files
            .into_iter()
            .map(|(path, bytes)| (project_file(&path), Bytes::new(bytes)))
            .collect();
        Self { main, files }
    }

    /// The refusal for a file this world does not hold.
    fn refuse(id: FileId) -> FileError {
        match id.root() {
            VirtualRoot::Package(spec) => FileError::Package(PackageError::Other(Some(
                format!("{spec} is a package, and a report compiles offline with none").into(),
            ))),
            VirtualRoot::Project => FileError::NotFound(id.vpath().get_with_slash().into()),
        }
    }
}

impl World for ReportWorld {
    fn library(&self) -> &LazyHash<Library> {
        &LIBRARY
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &FONTS.1
    }

    fn main(&self) -> FileId {
        self.main.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() {
            Ok(self.main.clone())
        } else {
            Err(Self::refuse(id))
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if id == self.main.id() {
            return Ok(Bytes::from_string(self.main.clone()));
        }
        self.files.get(&id).cloned().ok_or_else(|| Self::refuse(id))
    }

    fn font(&self, index: usize) -> Option<Font> {
        FONTS.0.get(index).cloned()
    }

    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use typst_layout::PagedDocument;

    fn compile(source: &str) -> Result<PagedDocument, String> {
        let world = ReportWorld::new(source.to_string(), Vec::new());
        let compiled = typst::compile::<PagedDocument>(&world);
        let warnings: Vec<String> = compiled
            .warnings
            .iter()
            .map(|w| w.message.to_string())
            .collect();
        assert!(warnings.is_empty(), "{warnings:?}");
        compiled.output.map_err(|errors| {
            errors
                .iter()
                .map(|e| e.message.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        })
    }

    #[test]
    fn a_package_import_is_an_error() {
        let err = compile("#import \"@preview/x:0.1.0\": *\nHello").expect_err("refused");
        assert!(err.contains("package"), "{err}");
    }

    #[test]
    fn a_file_on_disk_is_not_read() {
        let err = compile("#read(\"/etc/hosts\")").expect_err("refused");
        assert!(
            err.contains("not found") || err.contains("/etc/hosts"),
            "{err}"
        );
    }

    #[test]
    fn the_bundled_fonts_are_found() {
        let doc = compile(
            "#set text(font: \"Inter\")\nRegular #text(weight: \"medium\")[medium] #text(weight: \"semibold\")[semibold] #text(weight: \"bold\")[bold] #text(font: \"JetBrains Mono\")[mono]",
        )
        .expect("compiles");
        assert_eq!(doc.pages().len(), 1);
    }
}
