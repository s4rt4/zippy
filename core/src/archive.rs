//! Orkestrasi operasi: pilih backend yang tepat untuk compress/extract/list.
//!
//! Titik masuk utama core yang dipakai frontend & verb CLI. Berdasarkan
//! [`ArchiveKind`] hasil deteksi (magic bytes, berlapis), operasi diarahkan ke
//! backend native (zip/tar/...) atau subprocess (7z/unrar) (Planning Doc §2.2).

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::time::Instant;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;

use crate::cancel::CancelToken;
use crate::error::{Error, Result};
use crate::extract;
use crate::formats::{self, Format};
use crate::progress::{ProgressEvent, ProgressSink};
use crate::safety::DecompressionGuard;
use crate::subprocess;

/// Satu entry di dalam archive (untuk list view di UI).
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub size: u64,
    pub compressed_size: u64,
    pub is_dir: bool,
    /// Waktu modifikasi terformat `"YYYY-MM-DD HH:MM"` bila tersedia.
    pub modified: Option<String>,
    /// Checksum CRC32 (tersedia untuk zip; None untuk format tanpa CRC).
    pub crc32: Option<u32>,
}

impl Entry {
    /// Entry minimal tanpa metadata waktu/CRC (dipakai backend yang tidak
    /// mengeksposnya).
    pub(crate) fn basic(name: String, size: u64, compressed_size: u64, is_dir: bool) -> Self {
        Entry {
            name,
            size,
            compressed_size,
            is_dir,
            modified: None,
            crc32: None,
        }
    }
}

/// Format epoch detik (UTC) → `"YYYY-MM-DD HH:MM"`. Algoritma civil-from-days
/// (Howard Hinnant) agar tidak butuh dependensi tanggal.
pub(crate) fn fmt_epoch(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hour, minute) = ((rem / 3600) as u32, (rem % 3600 / 60) as u32);

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = if month <= 2 { year + 1 } else { year };

    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

/// Jenis archive lengkap, termasuk apakah ia tar-compound atau stream tunggal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
    TarZst,
    Gz,
    Bz2,
    Xz,
    Zst,
    SevenZip,
    Rar,
}

impl ArchiveKind {
    fn is_tar_family(self) -> bool {
        matches!(
            self,
            ArchiveKind::Tar
                | ArchiveKind::TarGz
                | ArchiveKind::TarBz2
                | ArchiveKind::TarXz
                | ArchiveKind::TarZst
        )
    }
}

// ---------------------------------------------------------------------------
// Deteksi
// ---------------------------------------------------------------------------

/// Deteksi [`ArchiveKind`] dari file: magic bytes dulu (berlapis untuk
/// tar-compound), fallback ke ekstensi bila magic tidak konklusif.
pub fn detect_kind(path: &Path) -> Result<ArchiveKind> {
    let mut head = [0u8; 512];
    let n = {
        let mut f = File::open(path)?;
        read_fill(&mut f, &mut head)?
    };
    let outer = formats::detect(&head[..n]);

    let kind = match outer {
        Format::Zip => Some(ArchiveKind::Zip),
        Format::SevenZip => Some(ArchiveKind::SevenZip),
        Format::Rar => Some(ArchiveKind::Rar),
        Format::Tar => Some(ArchiveKind::Tar),
        // Stream terkompresi: intip apakah isinya tar (ustar di offset 257).
        Format::Gzip => Some(tar_or(path, outer, ArchiveKind::TarGz, ArchiveKind::Gz)?),
        Format::Bzip2 => Some(tar_or(path, outer, ArchiveKind::TarBz2, ArchiveKind::Bz2)?),
        Format::Xz => Some(tar_or(path, outer, ArchiveKind::TarXz, ArchiveKind::Xz)?),
        Format::Zstd => Some(tar_or(path, outer, ArchiveKind::TarZst, ArchiveKind::Zst)?),
        Format::Unknown => None,
    };

    kind.or_else(|| kind_from_ext(path))
        .ok_or(Error::UnsupportedFormat)
}

/// Intip apakah stream terkompresi berisi tar; kembalikan `yes`/`no` sesuai.
fn tar_or(path: &Path, outer: Format, yes: ArchiveKind, no: ArchiveKind) -> Result<ArchiveKind> {
    let f = File::open(path)?;
    let mut r: Box<dyn Read> = match outer {
        Format::Gzip => Box::new(GzDecoder::new(f)),
        Format::Bzip2 => Box::new(bzip2::read::BzDecoder::new(f)),
        Format::Xz => Box::new(xz2::read::XzDecoder::new(f)),
        Format::Zstd => Box::new(zstd::stream::read::Decoder::new(f)?),
        _ => return Ok(no),
    };
    let mut head = [0u8; 512];
    let n = read_fill(&mut r, &mut head)?;
    if formats::detect(&head[..n]) == Format::Tar {
        Ok(yes)
    } else {
        Ok(no)
    }
}

/// Tentukan [`ArchiveKind`] dari ekstensi (fallback / untuk output compress).
pub fn kind_from_ext(path: &Path) -> Option<ArchiveKind> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    let k = if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        ArchiveKind::TarGz
    } else if name.ends_with(".tar.bz2") || name.ends_with(".tbz2") {
        ArchiveKind::TarBz2
    } else if name.ends_with(".tar.xz") || name.ends_with(".txz") {
        ArchiveKind::TarXz
    } else if name.ends_with(".tar.zst") {
        ArchiveKind::TarZst
    } else if name.ends_with(".tar") {
        ArchiveKind::Tar
    } else if name.ends_with(".zip") {
        ArchiveKind::Zip
    } else if name.ends_with(".7z") {
        ArchiveKind::SevenZip
    } else if name.ends_with(".rar") {
        ArchiveKind::Rar
    } else if name.ends_with(".gz") {
        ArchiveKind::Gz
    } else if name.ends_with(".bz2") {
        ArchiveKind::Bz2
    } else if name.ends_with(".xz") {
        ArchiveKind::Xz
    } else if name.ends_with(".zst") {
        ArchiveKind::Zst
    } else {
        return None;
    };
    Some(k)
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

/// Daftar isi archive tanpa meng-extract.
pub fn list(archive: &Path, password: Option<&str>) -> Result<Vec<Entry>> {
    let kind = detect_kind(archive)?;
    match kind {
        ArchiveKind::Zip => list_zip(archive),
        k if k.is_tar_family() => {
            let reader = open_tar_reader(archive, k)?;
            extract::list_tar(reader)
        }
        ArchiveKind::Gz | ArchiveKind::Bz2 | ArchiveKind::Xz | ArchiveKind::Zst => {
            Ok(vec![Entry::basic(
                single_output_name(archive),
                0,
                archive.metadata()?.len(),
                false,
            )])
        }
        ArchiveKind::SevenZip => subprocess::sevenzip_list(archive),
        ArchiveKind::Rar => subprocess::unrar_list(archive),
        _ => {
            let _ = password;
            Err(Error::UnsupportedFormat)
        }
    }
}

// ---------------------------------------------------------------------------
// Extract
// ---------------------------------------------------------------------------

/// Extract seluruh isi `archive` ke `dest`.
///
/// `cancel` diperiksa di batas tiap entry dan di dalam loop salin byte; saat
/// dibatalkan, file yang sedang ditulis dihapus dan fungsi mengembalikan
/// [`Error::Cancelled`].
pub fn extract_all(
    archive: &Path,
    dest: &Path,
    password: Option<&str>,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    fs::create_dir_all(dest)?;
    let kind = detect_kind(archive)?;
    match kind {
        ArchiveKind::Zip => extract_zip(archive, dest, password, cancel, progress),
        k if k.is_tar_family() => {
            let input_size = archive.metadata()?.len();
            let reader = open_tar_reader(archive, k)?;
            extract::extract_tar(reader, dest, input_size, cancel, progress)
        }
        ArchiveKind::Gz | ArchiveKind::Bz2 | ArchiveKind::Xz | ArchiveKind::Zst => {
            extract_single(archive, dest, kind, cancel, progress)
        }
        ArchiveKind::SevenZip => {
            subprocess::sevenzip_extract(archive, dest, password, cancel, progress)
        }
        ArchiveKind::Rar => subprocess::unrar_extract(archive, dest, password, cancel, progress),
        _ => Err(Error::UnsupportedFormat),
    }
}

// ---------------------------------------------------------------------------
// Compress
// ---------------------------------------------------------------------------

/// Buat archive baru `dest` dari kumpulan `inputs`. Format ditentukan dari
/// ekstensi `dest`.
///
/// Bila operasi gagal di tengah jalan (termasuk Cancel), archive parsial `dest`
/// dihapus agar tidak meninggalkan file rusak.
pub fn compress(
    inputs: &[&Path],
    dest: &Path,
    password: Option<&str>,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let kind = kind_from_ext(dest).ok_or(Error::UnsupportedFormat)?;
    let res = match kind {
        ArchiveKind::Zip => compress_zip(inputs, dest, password, cancel, progress),
        k if k.is_tar_family() => compress_tar(inputs, dest, k, cancel, progress),
        ArchiveKind::Gz | ArchiveKind::Bz2 | ArchiveKind::Xz | ArchiveKind::Zst => {
            compress_single(inputs, dest, kind, cancel, progress)
        }
        ArchiveKind::SevenZip => {
            subprocess::sevenzip_compress(inputs, dest, password, cancel, progress)
        }
        ArchiveKind::Rar => Err(Error::Other("RAR compress tidak didukung (extract only)".into())),
        _ => Err(Error::UnsupportedFormat),
    };
    if res.is_err() {
        // Best-effort: buang archive parsial. (Untuk 7z, file dibuat oleh child
        // process; tetap kita coba hapus.)
        let _ = fs::remove_file(dest);
    }
    res
}

// ---------------------------------------------------------------------------
// ZIP backend (native)
// ---------------------------------------------------------------------------

fn list_zip(archive: &Path) -> Result<Vec<Entry>> {
    let f = File::open(archive)?;
    let mut ar = zip::ZipArchive::new(BufReader::new(f)).map_err(zip_err)?;
    let mut out = Vec::with_capacity(ar.len());
    for i in 0..ar.len() {
        // `by_index_raw` membaca metadata tanpa butuh password (untuk listing).
        let e = ar.by_index_raw(i).map_err(zip_err)?;
        let modified = e.last_modified().map(|d| {
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}",
                d.year(),
                d.month(),
                d.day(),
                d.hour(),
                d.minute()
            )
        });
        let is_dir = e.is_dir();
        out.push(Entry {
            name: e.name().to_string(),
            size: e.size(),
            compressed_size: e.compressed_size(),
            is_dir,
            modified,
            // CRC tidak bermakna untuk entry direktori.
            crc32: if is_dir { None } else { Some(e.crc32()) },
        });
    }
    Ok(out)
}

fn extract_zip(
    archive: &Path,
    dest: &Path,
    password: Option<&str>,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let start = Instant::now();
    let f = File::open(archive)?;
    let input_size = f.metadata()?.len();
    let mut ar = zip::ZipArchive::new(BufReader::new(f)).map_err(zip_err)?;
    let mut guard = DecompressionGuard::new(input_size);

    let total = ar.len();
    progress.emit(ProgressEvent::Started { total_files: total });

    for i in 0..total {
        cancel.check()?;
        let mut e = match password {
            Some(pw) => ar.by_index_decrypt(i, pw.as_bytes()).map_err(zip_err)?,
            None => ar.by_index(i).map_err(zip_err)?,
        };
        let name = e.name().to_string();
        let is_dir = e.is_dir();
        let out = extract::prepare_dest(dest, &name)?;

        if is_dir {
            fs::create_dir_all(&out)?;
        } else {
            extract::copy_guarded_to_file(&mut e, &out, &mut guard, cancel)?;
        }

        progress.emit(ProgressEvent::FileProcessed { name, index: i });
    }

    progress.emit(ProgressEvent::Finished {
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
    Ok(())
}

fn compress_zip(
    inputs: &[&Path],
    dest: &Path,
    password: Option<&str>,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let start = Instant::now();
    let f = File::create(dest)?;
    let mut zw = zip::ZipWriter::new(BufWriter::new(f));

    // Tipe borrowed (bukan SimpleFileOptions yang 'static) agar password AES
    // boleh meminjam dari argumen fungsi.
    let mut opts: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    if let Some(pw) = password {
        opts = opts.with_aes_encryption(zip::AesMode::Aes256, pw);
    }

    progress.emit(ProgressEvent::Started {
        total_files: count_files(inputs),
    });

    let mut index = 0;
    for input in inputs {
        let base = input.parent().unwrap_or(Path::new(""));
        zip_add(&mut zw, opts, base, input, cancel, progress, &mut index)?;
    }
    zw.finish().map_err(zip_err)?;

    progress.emit(ProgressEvent::Finished {
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
    Ok(())
}

fn zip_add<W: Write + std::io::Seek>(
    zw: &mut zip::ZipWriter<W>,
    opts: zip::write::FileOptions<'_, ()>,
    base: &Path,
    path: &Path,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
    index: &mut usize,
) -> Result<()> {
    cancel.check()?;
    let rel = path.strip_prefix(base).unwrap_or(path);
    let name = rel.to_string_lossy().replace('\\', "/");

    if path.is_dir() {
        if !name.is_empty() {
            zw.add_directory(format!("{name}/"), opts).map_err(zip_err)?;
        }
        let mut entries: Vec<_> = fs::read_dir(path)?.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.path());
        for e in entries {
            zip_add(zw, opts, base, &e.path(), cancel, progress, index)?;
        }
    } else {
        zw.start_file(name.clone(), opts).map_err(zip_err)?;
        let mut f = File::open(path)?;
        std::io::copy(&mut f, zw)?;
        progress.emit(ProgressEvent::FileProcessed {
            name,
            index: *index,
        });
        *index += 1;
    }
    Ok(())
}

fn zip_err(e: zip::result::ZipError) -> Error {
    use zip::result::ZipError;
    match e {
        ZipError::Io(io) => Error::Io(io),
        ZipError::InvalidPassword => Error::Password,
        ZipError::UnsupportedArchive(msg) if msg.to_lowercase().contains("password") => {
            Error::Password
        }
        other => Error::Other(other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// TAR family backend (native)
// ---------------------------------------------------------------------------

fn open_tar_reader(archive: &Path, kind: ArchiveKind) -> Result<Box<dyn Read>> {
    let f = File::open(archive)?;
    let r: Box<dyn Read> = match kind {
        ArchiveKind::Tar => Box::new(f),
        ArchiveKind::TarGz => Box::new(GzDecoder::new(f)),
        ArchiveKind::TarBz2 => Box::new(bzip2::read::BzDecoder::new(f)),
        ArchiveKind::TarXz => Box::new(xz2::read::XzDecoder::new(f)),
        ArchiveKind::TarZst => Box::new(zstd::stream::read::Decoder::new(f)?),
        _ => return Err(Error::UnsupportedFormat),
    };
    Ok(r)
}

fn compress_tar(
    inputs: &[&Path],
    dest: &Path,
    kind: ArchiveKind,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let start = Instant::now();
    progress.emit(ProgressEvent::Started {
        total_files: count_files(inputs),
    });

    let w = BufWriter::new(File::create(dest)?);
    match kind {
        ArchiveKind::Tar => {
            let mut b = tar::Builder::new(w);
            write_tar_entries(&mut b, inputs, cancel, progress)?;
            b.finish()?;
        }
        ArchiveKind::TarGz => {
            let enc = GzEncoder::new(w, flate2::Compression::default());
            let mut b = tar::Builder::new(enc);
            write_tar_entries(&mut b, inputs, cancel, progress)?;
            b.into_inner()?.finish()?;
        }
        ArchiveKind::TarBz2 => {
            let enc = bzip2::write::BzEncoder::new(w, bzip2::Compression::default());
            let mut b = tar::Builder::new(enc);
            write_tar_entries(&mut b, inputs, cancel, progress)?;
            b.into_inner()?.finish()?;
        }
        ArchiveKind::TarXz => {
            let enc = xz2::write::XzEncoder::new(w, 6);
            let mut b = tar::Builder::new(enc);
            write_tar_entries(&mut b, inputs, cancel, progress)?;
            b.into_inner()?.finish()?;
        }
        ArchiveKind::TarZst => {
            let enc = zstd::stream::write::Encoder::new(w, 3)?;
            let mut b = tar::Builder::new(enc);
            write_tar_entries(&mut b, inputs, cancel, progress)?;
            b.into_inner()?.finish()?;
        }
        _ => return Err(Error::UnsupportedFormat),
    }

    progress.emit(ProgressEvent::Finished {
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
    Ok(())
}

/// Tulis tiap input ke builder tar. Direktori ditelusuri manual (bukan
/// `append_dir_all`) supaya tiap berkas memancarkan `FileProcessed` dan Cancel
/// terdeteksi per-entry, bukan hanya antar-input.
fn write_tar_entries<W: Write>(
    builder: &mut tar::Builder<W>,
    inputs: &[&Path],
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let mut index = 0;
    for input in inputs {
        let name = input
            .file_name()
            .ok_or_else(|| Error::Other(format!("input tanpa nama: {}", input.display())))?;
        add_tar_entry(builder, input, Path::new(name), cancel, progress, &mut index)?;
    }
    Ok(())
}

/// Tambah satu path (file atau dir) ke tar di bawah nama `arc`, rekursif untuk
/// direktori. `index` naik tiap berkas (bukan direktori).
fn add_tar_entry<W: Write>(
    builder: &mut tar::Builder<W>,
    disk: &Path,
    arc: &Path,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
    index: &mut usize,
) -> Result<()> {
    cancel.check()?;
    // `append_path_with_name` menambah entry dir (header saja) atau file+isi.
    builder.append_path_with_name(disk, arc)?;

    if disk.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(disk)?.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.path());
        for e in entries {
            add_tar_entry(builder, &e.path(), &arc.join(e.file_name()), cancel, progress, index)?;
        }
    } else {
        progress.emit(ProgressEvent::FileProcessed {
            name: arc.to_string_lossy().into_owned(),
            index: *index,
        });
        *index += 1;
    }
    Ok(())
}

/// Hitung jumlah berkas (bukan direktori) di bawah `inputs`, rekursif. Dipakai
/// untuk `total_files` agar progress bar compress determinate.
fn count_files(inputs: &[&Path]) -> usize {
    fn rec(p: &Path) -> usize {
        if p.is_dir() {
            fs::read_dir(p)
                .map(|rd| rd.filter_map(|e| e.ok()).map(|e| rec(&e.path())).sum())
                .unwrap_or(0)
        } else {
            1
        }
    }
    inputs.iter().map(|p| rec(p)).sum()
}

// ---------------------------------------------------------------------------
// Single-file stream backend (gz / bz2 / xz / zst tanpa tar)
// ---------------------------------------------------------------------------

/// Nama output untuk stream tunggal: buang ekstensi kompresi terluar.
fn single_output_name(archive: &Path) -> String {
    archive
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".to_string())
}

fn extract_single(
    archive: &Path,
    dest: &Path,
    kind: ArchiveKind,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let start = Instant::now();
    let input_size = archive.metadata()?.len();
    let f = File::open(archive)?;
    let mut r: Box<dyn Read> = match kind {
        ArchiveKind::Gz => Box::new(GzDecoder::new(f)),
        ArchiveKind::Bz2 => Box::new(bzip2::read::BzDecoder::new(f)),
        ArchiveKind::Xz => Box::new(xz2::read::XzDecoder::new(f)),
        ArchiveKind::Zst => Box::new(zstd::stream::read::Decoder::new(f)?),
        _ => return Err(Error::UnsupportedFormat),
    };

    let name = single_output_name(archive);
    let out = extract::prepare_dest(dest, &name)?;
    let mut guard = DecompressionGuard::new(input_size);

    progress.emit(ProgressEvent::Started { total_files: 1 });
    extract::copy_guarded_to_file(&mut r, &out, &mut guard, cancel)?;
    progress.emit(ProgressEvent::FileProcessed {
        name,
        index: 0,
    });
    progress.emit(ProgressEvent::Finished {
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
    Ok(())
}

fn compress_single(
    inputs: &[&Path],
    dest: &Path,
    kind: ArchiveKind,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    if inputs.len() != 1 || inputs[0].is_dir() {
        return Err(Error::Other(
            "format stream tunggal (gz/bz2/xz/zst) hanya untuk satu file — pakai tar.* untuk banyak file".into(),
        ));
    }
    let start = Instant::now();
    progress.emit(ProgressEvent::Started { total_files: 1 });

    let mut input = File::open(inputs[0])?;
    let w = BufWriter::new(File::create(dest)?);
    let mut enc: Box<dyn Write> = match kind {
        ArchiveKind::Gz => Box::new(GzEncoder::new(w, flate2::Compression::default())),
        ArchiveKind::Bz2 => Box::new(bzip2::write::BzEncoder::new(w, bzip2::Compression::default())),
        ArchiveKind::Xz => Box::new(xz2::write::XzEncoder::new(w, 6)),
        ArchiveKind::Zst => Box::new(zstd::stream::write::Encoder::new(w, 3)?.auto_finish()),
        _ => return Err(Error::UnsupportedFormat),
    };
    // Loop manual (bukan io::copy) agar Cancel bisa menghentikan file besar di
    // tengah jalan.
    let mut buf = [0u8; 64 * 1024];
    loop {
        cancel.check()?;
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        enc.write_all(&buf[..n])?;
    }
    enc.flush()?;
    drop(enc); // pastikan trailer encoder ditulis

    let name = single_output_name(dest);
    progress.emit(ProgressEvent::FileProcessed { name, index: 0 });
    progress.emit(ProgressEvent::Finished {
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// util
// ---------------------------------------------------------------------------

/// Baca sampai buffer penuh atau EOF; kembalikan jumlah byte terisi.
fn read_fill<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = r.read(&mut buf[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(filled)
}

#[cfg(test)]
mod tests;
