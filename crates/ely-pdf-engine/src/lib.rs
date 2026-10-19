//! MuPDF in its own process: protocol and client (PDF-01).

mod client;
mod content;
mod edit;
mod marks;
mod ocr;
mod output;
mod redact;
mod secure;
mod sign;
mod structure;
pub mod proto;
#[cfg(any(test, feature = "test-support"))]
pub mod sample;

pub use client::{Doc, Opened, Refused, Tile, Worker, is_stale, refused};
pub use content::{Drawn, TextBlock, Writing, Wrote};
pub use ocr::{OcrLine, tessdata};
pub use output::Optimizing;
pub use redact::{Cleaning, Reach, Redacted, Region};
pub use secure::Lock;
pub use sign::{Digest, Signature, Signed, Signing};
pub use structure::{Attachment, BookmarkChange, Link, LinkChange, Properties, Target};
pub use marks::{Annot, Field, FieldKind, Marks, NewAnnot, NewField, Shape};
pub use proto::{Bookmark, Char};

/// The worker binary is behind `worker-bin`, so a client that only needs the
/// IPC client (Yu's PDF surface) builds without MuPDF. This test guards the
/// manifest boundary: the bin must stay gated on the feature.
#[cfg(test)]
mod manifest_contract {
    #[test]
    fn worker_bin_requires_the_feature() {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .expect("the crate manifest");
        // `worker-bin` is an empty feature (not an alias or a dependency set).
        assert!(manifest.contains("worker-bin = []"), "worker-bin must stay an empty feature");
        // The worker binary is gated on it, so `--lib` alone never links MuPDF.
        assert!(manifest.contains("required-features = [\"worker-bin\"]"), "the bin must require worker-bin");
        assert!(manifest.contains("name = \"ely-pdf-worker\""), "the bin must keep its name");
        assert!(manifest.contains("autobins = false"), "the bin must be declared explicitly");
    }
}
