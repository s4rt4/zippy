//! Sanitasi path (Zip Slip) & batas dekompresi (zip bomb).
//!
//! Keamanan masuk core sejak awal (Planning Doc §10.4). Modul ini menyediakan
//! guard yang dipanggil sebelum menulis entry hasil extract ke disk.
//!
//! Status: **Sprint 0 — guard path traversal dasar sudah jalan & teruji**;
//! batas rasio dekompresi diintegrasikan dengan extractor di v0.1.

use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};

/// Batas rasio dekompresi default (output/input). Di atas ini → diduga zip bomb.
pub const DEFAULT_MAX_RATIO: u64 = 100;
/// Batas ukuran total output default (bytes). 0 = tanpa batas.
pub const DEFAULT_MAX_TOTAL_BYTES: u64 = 10 * 1024 * 1024 * 1024; // 10 GiB
/// Rasio baru ditegakkan setelah output melewati ambang ini. Mencegah false
/// positive pada file kecil yang sangat kompresibel (mis. 4KB byte nol).
pub const DEFAULT_RATIO_FLOOR_BYTES: u64 = 64 * 1024 * 1024; // 64 MiB

/// Gabungkan `dest` dengan path entry archive secara aman.
///
/// Menolak path absolut dan komponen `..` (Zip Slip). Mengembalikan path final
/// yang dijamin berada di dalam `dest`.
pub fn safe_join(dest: &Path, entry: &str) -> Result<PathBuf> {
    let entry_path = Path::new(entry);

    if entry_path.is_absolute() {
        return Err(Error::UnsafePath(entry.to_string()));
    }

    let mut out = dest.to_path_buf();
    for comp in entry_path.components() {
        match comp {
            Component::Normal(c) => out.push(c),
            // Buang prefix `./` yang tidak berbahaya.
            Component::CurDir => {}
            // Tolak `..`, root, dan prefix (drive Windows) — semua tidak aman.
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::UnsafePath(entry.to_string()));
            }
        }
    }

    // Sabuk pengaman: hasil akhir harus tetap di bawah `dest`.
    if !out.starts_with(dest) {
        return Err(Error::UnsafePath(entry.to_string()));
    }

    Ok(out)
}

/// Pelacak batas dekompresi (zip bomb guard).
#[derive(Debug)]
pub struct DecompressionGuard {
    max_ratio: u64,
    max_total: u64,
    ratio_floor: u64,
    input_bytes: u64,
    output_bytes: u64,
}

impl DecompressionGuard {
    pub fn new(input_bytes: u64) -> Self {
        Self {
            max_ratio: DEFAULT_MAX_RATIO,
            max_total: DEFAULT_MAX_TOTAL_BYTES,
            ratio_floor: DEFAULT_RATIO_FLOOR_BYTES,
            input_bytes,
            output_bytes: 0,
        }
    }

    /// Catat `n` byte output baru; error bila melewati batas.
    pub fn add_output(&mut self, n: u64) -> Result<()> {
        self.output_bytes = self.output_bytes.saturating_add(n);

        if self.max_total != 0 && self.output_bytes > self.max_total {
            return Err(Error::DecompressionLimit);
        }
        // Rasio hanya ditegakkan setelah output cukup besar (di atas floor) —
        // file kecil yang sangat kompresibel bukan ancaman.
        if self.output_bytes >= self.ratio_floor
            && self.input_bytes > 0
            && self.output_bytes / self.input_bytes > self.max_ratio
        {
            return Err(Error::DecompressionLimit);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn accepts_normal_path() {
        let p = safe_join(Path::new("/tmp/out"), "dir/file.txt").unwrap();
        assert_eq!(p, Path::new("/tmp/out/dir/file.txt"));
    }

    #[test]
    fn rejects_parent_traversal() {
        assert!(safe_join(Path::new("/tmp/out"), "../etc/passwd").is_err());
        assert!(safe_join(Path::new("/tmp/out"), "a/../../b").is_err());
    }

    #[test]
    fn rejects_absolute_path() {
        assert!(safe_join(Path::new("/tmp/out"), "/etc/passwd").is_err());
    }

    #[test]
    fn strips_curdir_prefix() {
        let p = safe_join(Path::new("/tmp/out"), "./file.txt").unwrap();
        assert_eq!(p, Path::new("/tmp/out/file.txt"));
    }

    #[test]
    fn guard_trips_on_total_bytes() {
        let mut g = DecompressionGuard::new(1024);
        g.max_total = 4096;
        assert!(g.add_output(2048).is_ok());
        assert!(g.add_output(4096).is_err());
    }

    #[test]
    fn guard_trips_on_ratio() {
        let mut g = DecompressionGuard::new(10);
        g.max_ratio = 100;
        g.ratio_floor = 0; // uji logika rasio murni tanpa floor
        // 10 * 100 = 1000 (rasio tepat 100) masih ok; di atasnya → bomb.
        assert!(g.add_output(1000).is_ok());
        assert!(g.add_output(100).is_err()); // total 1100 → rasio 110 > 100
    }
}
