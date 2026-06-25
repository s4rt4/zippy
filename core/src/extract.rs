//! Ekstraksi + guard path traversal.
//!
//! Helper bersama backend ZIP & TAR. Setiap entry melewati
//! [`safety::safe_join`](crate::safety::safe_join) sebelum ditulis ke disk
//! (Zip Slip), dan setiap byte output dihitung
//! [`DecompressionGuard`](crate::safety::DecompressionGuard) (zip bomb).

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use tar::Archive as TarArchive;

use crate::archive::{Entry, OverwriteMode};
use crate::cancel::CancelToken;
use crate::error::Result;
use crate::progress::{ProgressEvent, ProgressSink};
use crate::safety::{self, DecompressionGuard};

/// Buat path tujuan yang aman + pastikan direktori induk ada.
pub(crate) fn prepare_dest(dest: &Path, name: &str) -> Result<PathBuf> {
    let out = safety::safe_join(dest, name)?;
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(out)
}

/// Tentukan path tujuan untuk satu **berkas** sesuai [`OverwriteMode`], setelah
/// guard Zip Slip + membuat direktori induk. Mengembalikan:
/// - `Some(path)` → tulis ke `path` (mungkin sudah di-rename agar unik),
/// - `None` → lewati (berkas sudah ada & mode `Skip`).
pub(crate) fn resolve_dest(
    dest: &Path,
    name: &str,
    mode: OverwriteMode,
    prohibited: &[String],
) -> Result<Option<PathBuf>> {
    // Lewati tipe berkas terlarang (security) sebelum menyentuh disk.
    if ext_prohibited(name, prohibited) {
        return Ok(None);
    }
    let out = prepare_dest(dest, name)?;
    if !out.exists() {
        return Ok(Some(out));
    }
    match mode {
        OverwriteMode::Overwrite => Ok(Some(out)),
        OverwriteMode::Skip => Ok(None),
        OverwriteMode::Rename => Ok(Some(unique_path(&out))),
    }
}

/// Apakah ekstensi `name` (lowercase) ada di daftar terlarang.
pub(crate) fn ext_prohibited(name: &str, prohibited: &[String]) -> bool {
    if prohibited.is_empty() {
        return false;
    }
    match Path::new(name).extension().and_then(|e| e.to_str()) {
        Some(e) => {
            let e = e.to_ascii_lowercase();
            prohibited.iter().any(|p| *p == e)
        }
        None => false,
    }
}

/// Cari nama unik untuk `path` yang sudah ada: `foo.txt` → `foo (1).txt`,
/// `foo (2).txt`, … (gaya WinRAR/Nautilus).
fn unique_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path.extension().and_then(|s| s.to_str());
    let mut n = 1u32;
    loop {
        let candidate = match ext {
            Some(e) => parent.join(format!("{stem} ({n}).{e}")),
            None => parent.join(format!("{stem} ({n})")),
        };
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

/// Salin `reader` → `writer` sambil menegakkan batas dekompresi dan memeriksa
/// pembatalan tiap blok (Cancel bisa menghentikan file besar di tengah jalan).
pub(crate) fn copy_guarded<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    guard: &mut DecompressionGuard,
    cancel: &CancelToken,
) -> Result<()> {
    let mut buf = [0u8; 64 * 1024];
    loop {
        cancel.check()?;
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        guard.add_output(n as u64)?;
        writer.write_all(&buf[..n])?;
    }
    Ok(())
}

/// Buat file `out`, salin `reader` ke dalamnya dengan guard + cancel, dan
/// **hapus file parsial** bila copy gagal (Cancel, zip bomb, atau I/O).
pub(crate) fn copy_guarded_to_file<R: Read>(
    reader: &mut R,
    out: &Path,
    guard: &mut DecompressionGuard,
    cancel: &CancelToken,
) -> Result<()> {
    let mut w = fs::File::create(out)?;
    match copy_guarded(reader, &mut w, guard, cancel) {
        Ok(()) => Ok(()),
        Err(e) => {
            drop(w);
            let _ = fs::remove_file(out);
            Err(e)
        }
    }
}

/// List isi sebuah stream tar (sudah ter-dekompresi bila perlu).
pub(crate) fn list_tar<R: Read>(reader: R) -> Result<Vec<Entry>> {
    let mut ar = TarArchive::new(reader);
    let mut out = Vec::new();
    for entry in ar.entries()? {
        let entry = entry?;
        let header = entry.header();
        let size = header.size().unwrap_or(0);
        let is_dir = header.entry_type().is_dir();
        let name = entry.path()?.to_string_lossy().into_owned();
        let modified = header.mtime().ok().map(crate::archive::fmt_epoch);
        out.push(Entry {
            name,
            size,
            compressed_size: size,
            is_dir,
            modified,
            crc32: None,
        });
    }
    Ok(out)
}

/// Extract seluruh isi stream tar ke `dest`, dengan guard Zip Slip + zip bomb.
pub(crate) fn extract_tar<R: Read>(
    reader: R,
    dest: &Path,
    input_size: u64,
    mode: OverwriteMode,
    prohibited: &[String],
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let start = Instant::now();
    let mut guard = DecompressionGuard::new(input_size);
    let mut ar = TarArchive::new(reader);

    // Total entry tidak diketahui di muka untuk stream tar.
    progress.emit(ProgressEvent::Started { total_files: 0 });

    let mut index = 0;
    for entry in ar.entries()? {
        cancel.check()?;
        let mut entry = entry?;
        let is_dir = entry.header().entry_type().is_dir();
        let name = entry.path()?.to_string_lossy().into_owned();

        if is_dir {
            fs::create_dir_all(prepare_dest(dest, &name)?)?;
        } else if let Some(out) = resolve_dest(dest, &name, mode, prohibited)? {
            copy_guarded_to_file(&mut entry, &out, &mut guard, cancel)?;
        }

        progress.emit(ProgressEvent::FileProcessed { name, index });
        index += 1;
    }

    progress.emit(ProgressEvent::Finished {
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    /// Reader yang membatalkan token setelah blok pertama, untuk memicu Cancel
    /// di tengah salinan (iterasi kedua `copy_guarded`).
    struct CancelOnRead<'a> {
        token: &'a CancelToken,
        done: bool,
    }
    impl Read for CancelOnRead<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.done {
                return Ok(0);
            }
            self.done = true;
            self.token.cancel();
            let n = buf.len().min(1024);
            buf[..n].fill(b'x');
            Ok(n)
        }
    }

    #[test]
    fn resolve_dest_new_file_returns_path() {
        let tmp = tempfile::tempdir().unwrap();
        let got = resolve_dest(tmp.path(), "a.txt", OverwriteMode::Skip, &[])
            .unwrap()
            .unwrap();
        assert_eq!(got, tmp.path().join("a.txt"));
    }

    #[test]
    fn resolve_dest_skip_returns_none_when_exists() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), b"x").unwrap();
        let got = resolve_dest(tmp.path(), "a.txt", OverwriteMode::Skip, &[]).unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn resolve_dest_overwrite_returns_same_path() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), b"x").unwrap();
        let got = resolve_dest(tmp.path(), "a.txt", OverwriteMode::Overwrite, &[])
            .unwrap()
            .unwrap();
        assert_eq!(got, tmp.path().join("a.txt"));
    }

    #[test]
    fn resolve_dest_rename_picks_unique_name() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), b"x").unwrap();
        let got = resolve_dest(tmp.path(), "a.txt", OverwriteMode::Rename, &[])
            .unwrap()
            .unwrap();
        assert_eq!(got, tmp.path().join("a (1).txt"));

        // Bila "(1)" juga ada → lanjut ke "(2)".
        std::fs::write(tmp.path().join("a (1).txt"), b"x").unwrap();
        let got2 = resolve_dest(tmp.path(), "a.txt", OverwriteMode::Rename, &[])
            .unwrap()
            .unwrap();
        assert_eq!(got2, tmp.path().join("a (2).txt"));
    }

    #[test]
    fn resolve_dest_prohibited_ext_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let banned = vec!["desktop".to_string(), "sh".to_string()];
        assert!(resolve_dest(tmp.path(), "evil.desktop", OverwriteMode::Overwrite, &banned)
            .unwrap()
            .is_none());
        assert!(resolve_dest(tmp.path(), "ok.txt", OverwriteMode::Overwrite, &banned)
            .unwrap()
            .is_some());
    }

    #[test]
    fn copy_to_file_removes_partial_on_cancel() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("partial.bin");
        let token = CancelToken::new();
        let mut guard = DecompressionGuard::new(1024);
        let mut reader = CancelOnRead { token: &token, done: false };

        let err = copy_guarded_to_file(&mut reader, &out, &mut guard, &token).unwrap_err();
        assert!(matches!(err, Error::Cancelled), "dapat {err:?}");
        assert!(!out.exists(), "file parsial harus dihapus saat cancel");
    }
}
