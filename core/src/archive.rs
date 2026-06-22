//! Orkestrasi operasi: pilih backend yang tepat untuk compress/extract/list.
//!
//! Ini titik masuk utama core yang dipakai frontend & verb CLI. Berdasarkan
//! [`Format`](crate::Format) hasil deteksi, operasi diarahkan ke backend native
//! (zip/tar/...) atau subprocess (7z/unrar) (Planning Doc §2.2).
//!
//! Status: **Sprint 0 — API placeholder**. Implementasi di v0.1 (Sprint 1-3).

use std::path::Path;

use crate::error::{Error, Result};
use crate::progress::ProgressSink;

/// Satu entry di dalam archive (untuk list view di UI).
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub size: u64,
    pub compressed_size: u64,
    pub is_dir: bool,
}

/// Daftar isi archive tanpa meng-extract.
pub fn list(_archive: &Path) -> Result<Vec<Entry>> {
    Err(Error::Other("list: belum diimplementasikan (v0.1)".into()))
}

/// Extract seluruh isi `archive` ke `dest`.
pub fn extract_all(
    _archive: &Path,
    _dest: &Path,
    _password: Option<&str>,
    _progress: &dyn ProgressSink,
) -> Result<()> {
    Err(Error::Other("extract_all: belum diimplementasikan (v0.1)".into()))
}

/// Buat archive baru `dest` dari kumpulan `inputs`.
pub fn compress(
    _inputs: &[&Path],
    _dest: &Path,
    _password: Option<&str>,
    _progress: &dyn ProgressSink,
) -> Result<()> {
    Err(Error::Other("compress: belum diimplementasikan (v0.1)".into()))
}
