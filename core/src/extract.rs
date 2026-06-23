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

use crate::archive::Entry;
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
        out.push(Entry {
            name,
            size,
            compressed_size: size,
            is_dir,
        });
    }
    Ok(out)
}

/// Extract seluruh isi stream tar ke `dest`, dengan guard Zip Slip + zip bomb.
pub(crate) fn extract_tar<R: Read>(
    reader: R,
    dest: &Path,
    input_size: u64,
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
        let out = prepare_dest(dest, &name)?;

        if is_dir {
            fs::create_dir_all(&out)?;
        } else {
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
