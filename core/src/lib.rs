//! Zippy core — pure Rust archive engine.
//!
//! Zero UI dependency: bisa dikompilasi dan diuji independen dari frontend
//! (`cargo test -p zippy-core`). Frontend GTK4 dan verb CLI memakai crate ini
//! sebagai satu-satunya sumber logika (lihat Planning Doc §2, §6.1).
//!
//! Status: **Sprint 0 — scaffold**. Modul masih stub; implementasi penuh
//! menyusul di v0.1 (Sprint 1-3).

pub mod archive;
pub mod error;
pub mod extract;
pub mod formats;
pub mod progress;
pub mod safety;
pub mod subprocess;

pub use archive::{ArchiveKind, Entry};
pub use error::{Error, Result};
pub use formats::Format;
pub use progress::{NullSink, ProgressEvent, ProgressSink};

/// Versi crate core, diekspos untuk ditampilkan di UI / `zippy --version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
