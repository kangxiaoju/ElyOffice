//! Embeddable Writer and Impress sessions without an application shell.

use std::{io, path::{Path, PathBuf}, sync::{Arc, Mutex}};

use ely_lok::{Doc, LokView, Worker, file_url, layout};
use gpui::{App, Entity};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Access {
    Editable,
    ReadOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OfficeKind {
    Writer,
    Presentation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentState {
    Ready,
    Dirty,
    Closed,
    WorkerEnded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimePaths {
    pub lok_program: PathBuf,
    pub lok_install: PathBuf,
    pub lok_profile: PathBuf,
    pub pdf_worker: PathBuf,
}

pub struct OfficeRuntime {
    worker: Arc<Mutex<Worker>>,
    #[cfg(feature = "pdf")]
    pdf_worker: PathBuf,
}

impl OfficeRuntime {
    pub fn spawn(paths: RuntimePaths) -> io::Result<Self> {
        let worker = Worker::spawn_from(&paths.lok_program, &paths.lok_install, &paths.lok_profile)?;
        Ok(Self {
            worker: Arc::new(Mutex::new(worker)),
            #[cfg(feature = "pdf")]
            pdf_worker: paths.pdf_worker,
        })
    }

    /// Starts the configured PDF worker by its explicit path.
    #[cfg(feature = "pdf")]
    pub fn spawn_pdf(&self) -> io::Result<ely_pdf_engine::Worker> {
        ely_pdf_engine::Worker::spawn_from(&self.pdf_worker)
    }

    pub fn open(
        &self,
        kind: OfficeKind,
        path: impl Into<PathBuf>,
        access: Access,
        cx: &mut App,
    ) -> io::Result<OfficeSession> {
        let path = path.into();
        let format = format_for(kind, &path)?;
        let mut worker = self.worker.lock().expect("the worker lock");
        let doc = worker.load(&file_url(&path))?;
        match kind {
            OfficeKind::Writer if !doc.is_sheet() && !doc.is_presentation() => {}
            OfficeKind::Presentation if doc.is_presentation() => {}
            _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "the loaded document does not match the requested office kind")),
        }
        let geometry = layout(&mut worker, &doc)?;
        drop(worker);
        let surface = LokView::open(Arc::clone(&self.worker), doc.clone(), geometry, cx);
        surface.update(cx, |view, _| view.set_read_only(access == Access::ReadOnly));
        Ok(OfficeSession {
            worker: Arc::clone(&self.worker),
            doc: Some(doc),
            surface,
            path,
            format,
            access,
            saved_edits: 0,
        })
    }

    pub fn ended(&self) -> Option<String> {
        self.worker.lock().expect("the worker lock").ended()
    }

    pub fn close(&self) -> io::Result<()> {
        self.worker.lock().expect("the worker lock").shutdown()
    }
}

pub struct OfficeSession {
    worker: Arc<Mutex<Worker>>,
    doc: Option<Doc>,
    surface: Entity<LokView>,
    path: PathBuf,
    format: &'static str,
    access: Access,
    saved_edits: u64,
}

impl OfficeSession {
    pub fn surface(&self) -> Entity<LokView> {
        self.surface.clone()
    }

    pub fn state(&self, cx: &App) -> DocumentState {
        if self.doc.is_none() {
            return DocumentState::Closed;
        }
        if self.worker.lock().expect("the worker lock").ended().is_some() {
            return DocumentState::WorkerEnded;
        }
        if self.surface.read(cx).edited() > self.saved_edits {
            DocumentState::Dirty
        } else {
            DocumentState::Ready
        }
    }

    pub fn save(&mut self, cx: &App) -> io::Result<()> {
        self.save_to(self.path.clone(), self.format, cx)
    }

    pub fn save_as(&mut self, path: impl Into<PathBuf>, cx: &App) -> io::Result<()> {
        let path = path.into();
        let format = extension(&path)?;
        self.save_to(path.clone(), format, cx)?;
        self.path = path;
        self.format = format;
        Ok(())
    }

    fn save_to(&mut self, path: PathBuf, format: &'static str, cx: &App) -> io::Result<()> {
        let doc = self.doc.as_ref().ok_or_else(|| io::Error::other("the office session is closed"))?;
        doc.save_owned(&mut self.worker.lock().expect("the worker lock"), &file_url(&path), format)?;
        self.saved_edits = self.surface.read(cx).edited();
        Ok(())
    }

    pub fn set_read_only(&mut self, read_only: bool, cx: &mut App) {
        self.access = if read_only { Access::ReadOnly } else { Access::Editable };
        self.surface.update(cx, |view, _| view.set_read_only(read_only));
    }

    pub fn close(&mut self) -> io::Result<()> {
        let Some(doc) = self.doc.take() else { return Ok(()); };
        self.worker.lock().expect("the worker lock").close_doc(doc)
    }
}

fn format_for(kind: OfficeKind, path: &Path) -> io::Result<&'static str> {
    let format = extension(path)?;
    match (kind, format) {
        (OfficeKind::Writer, "odt" | "doc" | "docx" | "fodt" | "rtf") => Ok(format),
        (OfficeKind::Presentation, "odp" | "ppt" | "pptx" | "fodp") => Ok(format),
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "the file extension does not match the office kind")),
    }
}

fn extension(path: &Path) -> io::Result<&'static str> {
    match path.extension().and_then(|value| value.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("odt") => Ok("odt"), Some("doc") => Ok("doc"), Some("docx") => Ok("docx"),
        Some("fodt") => Ok("fodt"), Some("rtf") => Ok("rtf"), Some("odp") => Ok("odp"),
        Some("ppt") => Ok("ppt"), Some("pptx") => Ok("pptx"), Some("fodp") => Ok("fodp"),
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "unsupported office file extension")),
    }
}
