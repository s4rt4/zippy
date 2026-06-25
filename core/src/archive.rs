//! Orkestrasi operasi: pilih backend yang tepat untuk compress/extract/list.
//!
//! Titik masuk utama core yang dipakai frontend & verb CLI. Berdasarkan
//! [`ArchiveKind`] hasil deteksi (magic bytes, berlapis), operasi diarahkan ke
//! backend native (zip/tar/...) atau subprocess (7z/unrar) (Planning Doc §2.2).

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
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

/// Tingkat kompresi yang dipilih user di dialog "Add". Dipetakan ke parameter
/// numerik tiap backend lewat method di bawah (Planning Doc §6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    /// Tanpa kompresi (simpan apa adanya) — tercepat, ukuran terbesar.
    Store,
    /// Cepat (rasio rendah).
    Fastest,
    /// Seimbang (default) — pilihan WinRAR "Normal".
    #[default]
    Normal,
    /// Maksimal (rasio tertinggi, paling lambat).
    Best,
}

impl Level {
    /// Level deflate untuk zip: `None` = default crate (≈6). `Store` ditangani
    /// terpisah via `CompressionMethod::Stored`, jadi di sini diperlakukan ≈fast.
    fn zip_deflate(self) -> Option<i64> {
        match self {
            Level::Store => Some(0),
            Level::Fastest => Some(1),
            Level::Normal => None,
            Level::Best => Some(9),
        }
    }

    /// Level flate2/gzip (0–9; 0 = tanpa kompresi).
    fn flate2(self) -> flate2::Compression {
        flate2::Compression::new(match self {
            Level::Store => 0,
            Level::Fastest => 1,
            Level::Normal => 6,
            Level::Best => 9,
        })
    }

    /// Level bzip2 (1–9; tidak punya mode "store", `Store` dipetakan ke 1).
    fn bzip2(self) -> bzip2::Compression {
        bzip2::Compression::new(match self {
            Level::Store | Level::Fastest => 1,
            Level::Normal => 6,
            Level::Best => 9,
        })
    }

    /// Preset xz/LZMA (0–9).
    fn xz(self) -> u32 {
        match self {
            Level::Store => 0,
            Level::Fastest => 1,
            Level::Normal => 6,
            Level::Best => 9,
        }
    }

    /// Level zstd (1–22; tidak ada "store", `Store` dipetakan ke 1).
    fn zstd(self) -> i32 {
        match self {
            Level::Store | Level::Fastest => 1,
            Level::Normal => 3,
            Level::Best => 19,
        }
    }

    /// Argumen `-mx=N` untuk 7-Zip (0 = store, 9 = ultra).
    pub(crate) fn sevenzip_mx(self) -> u32 {
        match self {
            Level::Store => 0,
            Level::Fastest => 1,
            Level::Normal => 5,
            Level::Best => 9,
        }
    }
}

/// Cara menangani berkas tujuan yang sudah ada saat extract (WinRAR §Overwrite).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverwriteMode {
    /// Tulis ulang berkas yang sudah ada (perilaku lama / default).
    #[default]
    Overwrite,
    /// Lewati berkas yang sudah ada (jangan tulis).
    Skip,
    /// Tulis dengan nama unik baru (`foo.txt` → `foo (1).txt`).
    Rename,
}

impl OverwriteMode {
    /// Flag overwrite untuk 7-Zip: `-aoa` (semua), `-aos` (skip), `-aou` (rename).
    pub(crate) fn sevenzip_flag(self) -> &'static str {
        match self {
            OverwriteMode::Overwrite => "-aoa",
            OverwriteMode::Skip => "-aos",
            OverwriteMode::Rename => "-aou",
        }
    }

    /// Flag overwrite untuk unrar: `-o+` (overwrite), `-o-` (skip), `-or` (rename).
    pub(crate) fn unrar_flag(self) -> &'static str {
        match self {
            OverwriteMode::Overwrite => "-o+",
            OverwriteMode::Skip => "-o-",
            OverwriteMode::Rename => "-or",
        }
    }
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
    extract_all_with(
        archive,
        dest,
        password,
        OverwriteMode::Overwrite,
        &[],
        cancel,
        progress,
    )
}

/// Seperti [`extract_all`], tetapi dengan kebijakan [`OverwriteMode`] eksplisit
/// untuk berkas yang sudah ada, plus daftar `prohibited` (ekstensi lowercase
/// tanpa titik) yang **dilewati** saat extract — padanan "exclude from
/// extracting" WinRAR. `prohibited` kosong = tanpa filter.
pub fn extract_all_with(
    archive: &Path,
    dest: &Path,
    password: Option<&str>,
    mode: OverwriteMode,
    prohibited: &[String],
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    fs::create_dir_all(dest)?;
    let kind = detect_kind(archive)?;
    match kind {
        ArchiveKind::Zip => {
            extract_zip(archive, dest, password, mode, prohibited, cancel, progress)
        }
        k if k.is_tar_family() => {
            let input_size = archive.metadata()?.len();
            let reader = open_tar_reader(archive, k)?;
            extract::extract_tar(reader, dest, input_size, mode, prohibited, cancel, progress)
        }
        ArchiveKind::Gz | ArchiveKind::Bz2 | ArchiveKind::Xz | ArchiveKind::Zst => {
            extract_single(archive, dest, kind, mode, prohibited, cancel, progress)
        }
        ArchiveKind::SevenZip => {
            subprocess::sevenzip_extract(archive, dest, password, mode, prohibited, cancel, progress)
        }
        ArchiveKind::Rar => {
            subprocess::unrar_extract(archive, dest, password, mode, prohibited, cancel, progress)
        }
        _ => Err(Error::UnsupportedFormat),
    }
}

// ---------------------------------------------------------------------------
// Test (verifikasi integritas)
// ---------------------------------------------------------------------------

/// Uji integritas seluruh isi `archive` tanpa menulis ke disk. Setiap entry
/// di-dekompresi penuh (untuk zip, ini sekaligus memverifikasi CRC32). Memancar
/// progress per-entry dan menghormati Cancel.
pub fn test(
    archive: &Path,
    password: Option<&str>,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let kind = detect_kind(archive)?;
    match kind {
        ArchiveKind::Zip => test_zip(archive, password, cancel, progress),
        k if k.is_tar_family() => {
            let input_size = archive.metadata()?.len();
            let reader = open_tar_reader(archive, k)?;
            test_tar(reader, input_size, cancel, progress)
        }
        ArchiveKind::Gz | ArchiveKind::Bz2 | ArchiveKind::Xz | ArchiveKind::Zst => {
            test_single(archive, kind, cancel, progress)
        }
        ArchiveKind::SevenZip => subprocess::sevenzip_test(archive, password, cancel, progress),
        ArchiveKind::Rar => subprocess::unrar_test(archive, password, cancel, progress),
        _ => Err(Error::UnsupportedFormat),
    }
}

fn test_zip(
    archive: &Path,
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
        if !e.is_dir() {
            // Membaca penuh memvalidasi CRC32 (zip crate error bila tak cocok).
            extract::copy_guarded(&mut e, &mut std::io::sink(), &mut guard, cancel)?;
        }
        progress.emit(ProgressEvent::FileProcessed { name, index: i });
    }

    progress.emit(ProgressEvent::Finished {
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
    Ok(())
}

fn test_tar<R: Read>(
    reader: R,
    input_size: u64,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let start = Instant::now();
    let mut guard = DecompressionGuard::new(input_size);
    let mut ar = tar::Archive::new(reader);
    progress.emit(ProgressEvent::Started { total_files: 0 });

    let mut index = 0;
    for entry in ar.entries()? {
        cancel.check()?;
        let mut entry = entry?;
        let name = entry.path()?.to_string_lossy().into_owned();
        extract::copy_guarded(&mut entry, &mut std::io::sink(), &mut guard, cancel)?;
        progress.emit(ProgressEvent::FileProcessed { name, index });
        index += 1;
    }

    progress.emit(ProgressEvent::Finished {
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
    Ok(())
}

fn test_single(
    archive: &Path,
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
    let mut guard = DecompressionGuard::new(input_size);
    progress.emit(ProgressEvent::Started { total_files: 1 });
    extract::copy_guarded(&mut r, &mut std::io::sink(), &mut guard, cancel)?;
    progress.emit(ProgressEvent::Finished {
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// Extract satu entry (untuk View)
// ---------------------------------------------------------------------------

/// Extract satu entry `name` ke bawah `dest_dir` (mempertahankan path relatif),
/// kembalikan path file hasil. Dipakai fitur View (buka satu berkas).
pub fn extract_entry(
    archive: &Path,
    name: &str,
    dest_dir: &Path,
    password: Option<&str>,
    cancel: &CancelToken,
) -> Result<PathBuf> {
    fs::create_dir_all(dest_dir)?;
    let kind = detect_kind(archive)?;
    let mut guard = DecompressionGuard::new(archive.metadata()?.len());

    match kind {
        ArchiveKind::Zip => {
            let f = File::open(archive)?;
            let mut ar = zip::ZipArchive::new(BufReader::new(f)).map_err(zip_err)?;
            let mut e = match password {
                Some(pw) => ar.by_name_decrypt(name, pw.as_bytes()).map_err(zip_err)?,
                None => ar.by_name(name).map_err(zip_err)?,
            };
            let out = extract::prepare_dest(dest_dir, name)?;
            extract::copy_guarded_to_file(&mut e, &out, &mut guard, cancel)?;
            Ok(out)
        }
        k if k.is_tar_family() => {
            let reader = open_tar_reader(archive, k)?;
            let mut ar = tar::Archive::new(reader);
            for entry in ar.entries()? {
                let mut entry = entry?;
                let ename = entry.path()?.to_string_lossy().into_owned();
                if ename == name {
                    let out = extract::prepare_dest(dest_dir, name)?;
                    extract::copy_guarded_to_file(&mut entry, &out, &mut guard, cancel)?;
                    return Ok(out);
                }
            }
            Err(Error::Other(format!("entry tidak ditemukan: {name}")))
        }
        ArchiveKind::Gz | ArchiveKind::Bz2 | ArchiveKind::Xz | ArchiveKind::Zst => {
            // Stream tunggal: extract_single menulis ke dest_dir/<nama-output>.
            extract_single(
                archive,
                dest_dir,
                kind,
                OverwriteMode::Overwrite,
                &[],
                cancel,
                &crate::progress::NullSink,
            )?;
            Ok(dest_dir.join(single_output_name(archive)))
        }
        ArchiveKind::SevenZip => {
            subprocess::sevenzip_extract_entry(archive, name, dest_dir, password, cancel)?;
            Ok(dest_dir.join(name))
        }
        ArchiveKind::Rar => {
            subprocess::unrar_extract_entry(archive, name, dest_dir, password, cancel)?;
            Ok(dest_dir.join(name))
        }
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
    compress_with_level(inputs, dest, password, Level::default(), cancel, progress)
}

/// Seperti [`compress`] tetapi dengan tingkat kompresi eksplisit. Untuk format
/// tanpa kompresi (`.tar` polos) `level` diabaikan.
pub fn compress_with_level(
    inputs: &[&Path],
    dest: &Path,
    password: Option<&str>,
    level: Level,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let kind = kind_from_ext(dest).ok_or(Error::UnsupportedFormat)?;
    let res = match kind {
        ArchiveKind::Zip => compress_zip(inputs, dest, password, level, cancel, progress),
        k if k.is_tar_family() => compress_tar(inputs, dest, k, level, cancel, progress),
        ArchiveKind::Gz | ArchiveKind::Bz2 | ArchiveKind::Xz | ArchiveKind::Zst => {
            compress_single(inputs, dest, kind, level, cancel, progress)
        }
        ArchiveKind::SevenZip => {
            subprocess::sevenzip_compress(inputs, dest, password, level, cancel, progress)
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
// Convert (ubah format) — extract ke folder sementara lalu kompres ulang
// ---------------------------------------------------------------------------

/// Konversi `src` ke format yang ditentukan ekstensi `dest`: ekstrak isi ke
/// direktori sementara lalu kompres ulang. `src_password` membuka sumber
/// terenkripsi; `dest_password`/`level` untuk arsip hasil.
///
/// Catatan: format stream tunggal tujuan (gz/bz2/xz/zst) hanya menerima satu
/// berkas — konversi arsip multi-berkas ke sana akan gagal (pakai `.tar.*`).
pub fn convert(
    src: &Path,
    dest: &Path,
    src_password: Option<&str>,
    dest_password: Option<&str>,
    level: Level,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let tmp = scratch_dir("convert");
    let res = (|| -> Result<()> {
        fs::create_dir_all(&tmp)?;
        // Fase ekstrak (tanpa progress per-file; progress utama = fase kompres).
        extract_all(src, &tmp, src_password, cancel, &crate::progress::NullSink)?;

        let mut inputs: Vec<PathBuf> = fs::read_dir(&tmp)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        inputs.sort();
        if inputs.is_empty() {
            return Err(Error::Other("tidak ada berkas untuk dikonversi".into()));
        }
        let refs: Vec<&Path> = inputs.iter().map(|p| p.as_path()).collect();
        compress_with_level(&refs, dest, dest_password, level, cancel, progress)
    })();
    let _ = fs::remove_dir_all(&tmp);
    res
}

/// Direktori sementara unik untuk operasi (`zippy-<tag>-<pid>-<nanos>`).
pub(crate) fn scratch_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("zippy-{tag}-{}-{nanos}", std::process::id()))
}

// ---------------------------------------------------------------------------
// Komentar arsip (ZIP)
// ---------------------------------------------------------------------------

/// Baca komentar arsip. Hanya ZIP yang membawa komentar; format lain → kosong.
pub fn read_comment(archive: &Path) -> Result<String> {
    if detect_kind(archive)? != ArchiveKind::Zip {
        return Ok(String::new());
    }
    let f = File::open(archive)?;
    let ar = zip::ZipArchive::new(BufReader::new(f)).map_err(zip_err)?;
    Ok(String::from_utf8_lossy(ar.comment()).into_owned())
}

/// Set komentar arsip (ZIP). Tulis-ulang via salin-mentah lossless lalu rename.
/// Tidak didukung untuk ZIP terenkripsi (salin-mentah merusak field AES).
pub fn set_comment(
    archive: &Path,
    comment: &str,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    if detect_kind(archive)? != ArchiveKind::Zip {
        return Err(Error::Other(
            "komentar arsip hanya didukung untuk ZIP".into(),
        ));
    }
    let start = Instant::now();
    let tmp = temp_sibling(archive);
    let res = (|| -> Result<()> {
        let f = File::open(archive)?;
        let mut src = zip::ZipArchive::new(BufReader::new(f)).map_err(zip_err)?;
        let total = src.len();
        for i in 0..total {
            if src.by_index_raw(i).map_err(zip_err)?.encrypted() {
                return Err(Error::Other(
                    "set komentar tidak didukung untuk ZIP terenkripsi".into(),
                ));
            }
        }

        let out = File::create(&tmp)?;
        let mut zw = zip::ZipWriter::new(BufWriter::new(out));
        progress.emit(ProgressEvent::Started { total_files: total });
        for i in 0..total {
            cancel.check()?;
            let entry = src.by_index_raw(i).map_err(zip_err)?;
            let name = entry.name().to_string();
            zw.raw_copy_file(entry).map_err(zip_err)?;
            progress.emit(ProgressEvent::FileProcessed { name, index: i });
        }
        zw.set_comment(comment);
        zw.finish().map_err(zip_err)?;
        Ok(())
    })();
    finalize_replace(res, &tmp, archive, start, progress)
}

// ---------------------------------------------------------------------------
// Delete (hapus entri — edit in-place)
// ---------------------------------------------------------------------------

/// Hapus entri `names` dari `archive` (edit in-place). Setiap nama yang
/// merupakan direktori ikut menghapus seluruh isinya (pencocokan prefiks).
///
/// Implementasi tulis-ulang ke file sementara di direktori yang sama lalu
/// `rename` menimpa archive asal — atomik-ish dan tidak meninggalkan archive
/// rusak bila gagal/cancel di tengah jalan. ZIP & TAR ditangani native (entri
/// disalin mentah tanpa rekompresi untuk ZIP); 7z lewat `7z d`. RAR dan stream
/// tunggal (gz/bz2/xz/zst) tidak mendukung penghapusan entri.
pub fn delete(
    archive: &Path,
    names: &[&str],
    password: Option<&str>,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let kind = detect_kind(archive)?;
    match kind {
        ArchiveKind::Zip => delete_zip(archive, names, password, cancel, progress),
        k if k.is_tar_family() => delete_tar(archive, k, names, cancel, progress),
        ArchiveKind::SevenZip => {
            subprocess::sevenzip_delete(archive, names, password, cancel, progress)
        }
        ArchiveKind::Rar => Err(Error::Other("RAR tidak mendukung hapus (extract only)".into())),
        ArchiveKind::Gz | ArchiveKind::Bz2 | ArchiveKind::Xz | ArchiveKind::Zst => Err(
            Error::Other("format stream tunggal hanya berisi satu berkas — hapus file-nya saja".into()),
        ),
        _ => Err(Error::UnsupportedFormat),
    }
}

/// Bangun predikat "apakah entri ini harus dihapus" dari daftar target.
/// Cocok bila nama sama persis, atau berada di bawah target yang merupakan
/// direktori (`target/...`). Trailing slash diabaikan.
fn make_remover(names: &[&str]) -> impl Fn(&str) -> bool {
    let targets: Vec<String> = names
        .iter()
        .map(|t| t.trim_end_matches('/').to_string())
        .filter(|t| !t.is_empty())
        .collect();
    move |name: &str| {
        let n = name.trim_end_matches('/');
        targets
            .iter()
            .any(|t| n == t || n.starts_with(&format!("{t}/")))
    }
}

/// Path file sementara di direktori yang sama dengan `archive` (agar `rename`
/// tetap dalam satu filesystem). Disisipi PID agar tidak bentrok antar-proses.
fn temp_sibling(archive: &Path) -> PathBuf {
    let parent = archive.parent().unwrap_or_else(|| Path::new("."));
    let name = archive
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".to_string());
    parent.join(format!(".{name}.zippy-{}.tmp", std::process::id()))
}

/// Selesaikan operasi tulis-ulang: sukses → `rename` menimpa asal & pancarkan
/// `Finished`; gagal → buang file sementara dan teruskan error.
fn finalize_replace(
    res: Result<()>,
    tmp: &Path,
    archive: &Path,
    start: Instant,
    progress: &dyn ProgressSink,
) -> Result<()> {
    match res {
        Ok(()) => {
            fs::rename(tmp, archive)?;
            progress.emit(ProgressEvent::Finished {
                elapsed_ms: start.elapsed().as_millis() as u64,
            });
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_file(tmp);
            Err(e)
        }
    }
}

fn delete_zip(
    archive: &Path,
    names: &[&str],
    password: Option<&str>,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let start = Instant::now();
    let remove = make_remover(names);
    let tmp = temp_sibling(archive);

    let res = (|| -> Result<()> {
        let f = File::open(archive)?;
        let mut src = zip::ZipArchive::new(BufReader::new(f)).map_err(zip_err)?;
        let total = src.len();

        // Kumpulkan bit "encrypted" tiap entri lebih dulu.
        let mut enc_flags = Vec::with_capacity(total);
        for i in 0..total {
            enc_flags.push(src.by_index_raw(i).map_err(zip_err)?.encrypted());
        }
        let any_encrypted = enc_flags.iter().any(|&e| e);

        let out = File::create(&tmp)?;
        let mut zw = zip::ZipWriter::new(BufWriter::new(out));
        progress.emit(ProgressEvent::Started { total_files: total });

        if !any_encrypted {
            // Jalur cepat: salin byte terkompresi apa adanya (lossless: metode
            // kompresi & CRC dipertahankan), tanpa dekompresi/password.
            for i in 0..total {
                cancel.check()?;
                let entry = src.by_index_raw(i).map_err(zip_err)?;
                let name = entry.name().to_string();
                if remove(&name) {
                    continue;
                }
                zw.raw_copy_file(entry).map_err(zip_err)?;
                progress.emit(ProgressEvent::FileProcessed { name, index: i });
            }
        } else {
            // Ada entri terenkripsi: `raw_copy_file` membangun ulang header dari
            // metadata dan TIDAK mempertahankan field-ekstra AES, jadi salin
            // mentah akan merusak entri. Maka dekripsi tiap entri lalu enkripsi
            // ulang dengan AES-256 (butuh password).
            let pw = password.ok_or(Error::Password)?;
            for i in 0..total {
                cancel.check()?;
                let mut e = if enc_flags[i] {
                    src.by_index_decrypt(i, pw.as_bytes()).map_err(zip_err)?
                } else {
                    src.by_index(i).map_err(zip_err)?
                };
                let name = e.name().to_string();
                if remove(&name) {
                    continue;
                }
                if e.is_dir() {
                    zw.add_directory(name, zip::write::SimpleFileOptions::default())
                        .map_err(zip_err)?;
                } else {
                    let opts: zip::write::FileOptions<'_, ()> =
                        zip::write::FileOptions::default()
                            .compression_method(zip::CompressionMethod::Deflated)
                            .with_aes_encryption(zip::AesMode::Aes256, pw);
                    zw.start_file(name.clone(), opts).map_err(zip_err)?;
                    extract::copy_guarded(
                        &mut e,
                        &mut zw,
                        &mut DecompressionGuard::new(u64::MAX),
                        cancel,
                    )?;
                    progress.emit(ProgressEvent::FileProcessed { name, index: i });
                }
            }
        }
        zw.finish().map_err(zip_err)?;
        Ok(())
    })();

    finalize_replace(res, &tmp, archive, start, progress)
}

fn delete_tar(
    archive: &Path,
    kind: ArchiveKind,
    names: &[&str],
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let start = Instant::now();
    let remove = make_remover(names);
    let tmp = temp_sibling(archive);

    let res = (|| -> Result<()> {
        let reader = open_tar_reader(archive, kind)?;
        let mut src = tar::Archive::new(reader);
        let w = BufWriter::new(File::create(&tmp)?);
        progress.emit(ProgressEvent::Started { total_files: 0 });

        // Rekompresi pakai level default; struktur tar di-stream ulang minus
        // entri yang dihapus.
        match kind {
            ArchiveKind::Tar => {
                let mut b = tar::Builder::new(w);
                copy_tar_except(&mut src, &mut b, &remove, cancel, progress)?;
                b.finish()?;
            }
            ArchiveKind::TarGz => {
                let enc = GzEncoder::new(w, flate2::Compression::default());
                let mut b = tar::Builder::new(enc);
                copy_tar_except(&mut src, &mut b, &remove, cancel, progress)?;
                b.into_inner()?.finish()?;
            }
            ArchiveKind::TarBz2 => {
                let enc = bzip2::write::BzEncoder::new(w, bzip2::Compression::default());
                let mut b = tar::Builder::new(enc);
                copy_tar_except(&mut src, &mut b, &remove, cancel, progress)?;
                b.into_inner()?.finish()?;
            }
            ArchiveKind::TarXz => {
                let enc = xz2::write::XzEncoder::new(w, 6);
                let mut b = tar::Builder::new(enc);
                copy_tar_except(&mut src, &mut b, &remove, cancel, progress)?;
                b.into_inner()?.finish()?;
            }
            ArchiveKind::TarZst => {
                let enc = zstd::stream::write::Encoder::new(w, 3)?;
                let mut b = tar::Builder::new(enc);
                copy_tar_except(&mut src, &mut b, &remove, cancel, progress)?;
                b.into_inner()?.finish()?;
            }
            _ => return Err(Error::UnsupportedFormat),
        }
        Ok(())
    })();

    finalize_replace(res, &tmp, archive, start, progress)
}

/// Salin semua entri tar dari `src` ke `builder` kecuali yang lolos `remove`.
fn copy_tar_except<R: Read, W: Write>(
    src: &mut tar::Archive<R>,
    builder: &mut tar::Builder<W>,
    remove: &dyn Fn(&str) -> bool,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let mut index = 0;
    for entry in src.entries()? {
        cancel.check()?;
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().into_owned();
        if remove(&path) {
            continue;
        }
        // `append_data` menangani nama panjang (ekstensi GNU/pax) & menghitung
        // ulang checksum header.
        let mut header = entry.header().clone();
        builder.append_data(&mut header, &path, &mut entry)?;
        progress.emit(ProgressEvent::FileProcessed { name: path, index });
        index += 1;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Info enkripsi
// ---------------------------------------------------------------------------

/// Apakah `archive` memakai enkripsi ZIP legasi (ZipCrypto) yang lemah pada
/// setidaknya satu entri. AES-256 dianggap kuat → `false`. Untuk format selain
/// ZIP selalu `false`. Dipakai UI untuk memperingatkan saat membuka archive.
pub fn has_weak_encryption(archive: &Path) -> Result<bool> {
    if detect_kind(archive)? != ArchiveKind::Zip {
        return Ok(false);
    }
    let f = File::open(archive)?;
    let mut ar = zip::ZipArchive::new(BufReader::new(f)).map_err(zip_err)?;
    let n = ar.len();

    // Kumpulkan dulu bit "encrypted" tiap entri (pinjam mutabel berurutan),
    // baru periksa apakah enkripsinya AES — `get_aes_verification_key_and_salt`
    // mengembalikan `None` untuk entri non-AES (yakni ZipCrypto legasi).
    let mut encrypted = Vec::with_capacity(n);
    for i in 0..n {
        encrypted.push(ar.by_index_raw(i).map_err(zip_err)?.encrypted());
    }
    for (i, &enc) in encrypted.iter().enumerate() {
        if enc && ar.get_aes_verification_key_and_salt(i).map_err(zip_err)?.is_none() {
            return Ok(true);
        }
    }
    Ok(false)
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
    mode: OverwriteMode,
    prohibited: &[String],
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

        if is_dir {
            fs::create_dir_all(extract::prepare_dest(dest, &name)?)?;
        } else if let Some(out) = extract::resolve_dest(dest, &name, mode, prohibited)? {
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
    level: Level,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let start = Instant::now();
    let f = File::create(dest)?;
    let mut zw = zip::ZipWriter::new(BufWriter::new(f));

    // Tipe borrowed (bukan SimpleFileOptions yang 'static) agar password AES
    // boleh meminjam dari argumen fungsi. `Store` → metode Stored (tanpa
    // kompresi); selainnya Deflated dengan level eksplisit.
    let method = if level == Level::Store {
        zip::CompressionMethod::Stored
    } else {
        zip::CompressionMethod::Deflated
    };
    let mut opts: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(method);
    if method != zip::CompressionMethod::Stored {
        opts = opts.compression_level(level.zip_deflate());
    }
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
    level: Level,
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
            let enc = GzEncoder::new(w, level.flate2());
            let mut b = tar::Builder::new(enc);
            write_tar_entries(&mut b, inputs, cancel, progress)?;
            b.into_inner()?.finish()?;
        }
        ArchiveKind::TarBz2 => {
            let enc = bzip2::write::BzEncoder::new(w, level.bzip2());
            let mut b = tar::Builder::new(enc);
            write_tar_entries(&mut b, inputs, cancel, progress)?;
            b.into_inner()?.finish()?;
        }
        ArchiveKind::TarXz => {
            let enc = xz2::write::XzEncoder::new(w, level.xz());
            let mut b = tar::Builder::new(enc);
            write_tar_entries(&mut b, inputs, cancel, progress)?;
            b.into_inner()?.finish()?;
        }
        ArchiveKind::TarZst => {
            let enc = zstd::stream::write::Encoder::new(w, level.zstd())?;
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
    mode: OverwriteMode,
    prohibited: &[String],
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
    let mut guard = DecompressionGuard::new(input_size);

    progress.emit(ProgressEvent::Started { total_files: 1 });
    if let Some(out) = extract::resolve_dest(dest, &name, mode, prohibited)? {
        extract::copy_guarded_to_file(&mut r, &out, &mut guard, cancel)?;
    }
    progress.emit(ProgressEvent::FileProcessed { name, index: 0 });
    progress.emit(ProgressEvent::Finished {
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
    Ok(())
}

fn compress_single(
    inputs: &[&Path],
    dest: &Path,
    kind: ArchiveKind,
    level: Level,
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
        ArchiveKind::Gz => Box::new(GzEncoder::new(w, level.flate2())),
        ArchiveKind::Bz2 => Box::new(bzip2::write::BzEncoder::new(w, level.bzip2())),
        ArchiveKind::Xz => Box::new(xz2::write::XzEncoder::new(w, level.xz())),
        ArchiveKind::Zst => Box::new(zstd::stream::write::Encoder::new(w, level.zstd())?.auto_finish()),
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
