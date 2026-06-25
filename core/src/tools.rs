//! Perkakas tambahan gaya WinRAR yang relevan di Linux:
//!
//! - **Pindai virus** (`Tools → Scan archive for viruses`) lewat **ClamAV**
//!   (`clamdscan`/`clamscan`). Opsional: bila biner tak terpasang, fitur
//!   melaporkan ketidaktersediaan alih-alih gagal diam-diam.
//! - **Perbaiki arsip** (`Tools → Repair archive`). Padanan Linux untuk
//!   "recovery record" RAR: (a) sidecar **PAR2** bila ada + `par2` terpasang,
//!   (b) `zip -FF` untuk ZIP yang rusak/terpotong.
//!
//! Semua via subprocess dengan `LC_ALL=C` (lihat [`crate::subprocess`]).

use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::archive::{self, ArchiveKind};
use crate::cancel::CancelToken;
use crate::error::{Error, Result};
use crate::subprocess::{hardened_command, run_status};

// ---------------------------------------------------------------------------
// Pindai virus (ClamAV)
// ---------------------------------------------------------------------------

/// Satu temuan virus: path (di dalam arsip / di disk) + nama signature.
#[derive(Debug, Clone)]
pub struct ScanFinding {
    pub path: String,
    pub signature: String,
}

/// Hasil pemindaian satu arsip.
#[derive(Debug, Clone)]
pub struct ScanReport {
    /// Nama scanner yang dipakai (`clamdscan` / `clamscan`).
    pub scanner: String,
    /// Daftar berkas terinfeksi (kosong = bersih).
    pub findings: Vec<ScanFinding>,
    /// Output gabungan untuk ditampilkan ke user.
    pub raw: String,
}

impl ScanReport {
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
}

/// Scanner ClamAV yang tersedia di PATH, prioritas daemon (`clamdscan`, lebih
/// cepat) lalu standalone (`clamscan`). `None` bila tidak ada.
pub fn virus_scanner() -> Option<&'static str> {
    for s in ["clamdscan", "clamscan"] {
        let ok = hardened_command(s)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|st| st.success())
            .unwrap_or(false);
        if ok {
            return Some(s);
        }
    }
    None
}

/// Pindai `archive` memakai ClamAV. ClamAV membuka isi arsip sendiri (zip, 7z,
/// rar, tar, dll) sehingga kita cukup menyodorkan file arsipnya — tidak perlu
/// mengekstrak malware ke disk lebih dulu.
///
/// Kode keluar ClamAV: `0` bersih, `1` ada virus, `2+` error.
pub fn scan(archive: &Path, cancel: &CancelToken) -> Result<ScanReport> {
    let scanner = virus_scanner().ok_or_else(|| {
        Error::Other("ClamAV tidak terpasang (butuh `clamscan` atau `clamdscan`)".into())
    })?;

    let mut cmd = hardened_command(scanner);
    if scanner == "clamdscan" {
        // Agar daemon (berjalan sebagai user clamav) tetap bisa membaca file
        // milik user yang menjalankan Zippy.
        cmd.arg("--fdpass");
    }
    cmd.arg("--no-summary").arg("--").arg(archive);

    let out = run_status(&mut cmd, None, Some(cancel))?;
    let raw = format!("{}{}", out.stdout, out.stderr);
    match out.code {
        Some(0) | Some(1) => Ok(ScanReport {
            scanner: scanner.to_string(),
            findings: parse_clam(&out.stdout),
            raw,
        }),
        other => Err(Error::Other(format!(
            "pemindaian gagal (kode {}): {}",
            other.map(|c| c.to_string()).unwrap_or_else(|| "sinyal".into()),
            out.stderr.trim()
        ))),
    }
}

/// Parse baris ClamAV bergaya `path: Signature FOUND`.
fn parse_clam(stdout: &str) -> Vec<ScanFinding> {
    stdout
        .lines()
        .filter_map(|l| {
            let stripped = l.trim_end().strip_suffix(" FOUND")?;
            let (path, sig) = stripped.rsplit_once(": ")?;
            Some(ScanFinding {
                path: path.to_string(),
                signature: sig.to_string(),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Perbaiki arsip
// ---------------------------------------------------------------------------

/// Hasil upaya perbaikan arsip.
#[derive(Debug, Clone)]
pub struct RepairReport {
    /// Metode yang dipakai (`PAR2` / `zip -FF`).
    pub method: String,
    /// File hasil bila perbaikan menulis arsip baru (mis. `foo-fixed.zip`).
    pub output_path: Option<PathBuf>,
    /// Apakah tool melaporkan sukses.
    pub repaired: bool,
    pub raw: String,
}

/// Apakah biner `par2` tersedia.
pub fn par2_available() -> bool {
    hardened_command("par2")
        .arg("--help")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Apakah biner `zip` tersedia.
fn zip_available() -> bool {
    hardened_command("zip")
        .arg("-h")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Sidecar PAR2 untuk `archive`, mis. `foo.zip` → `foo.zip.par2`.
fn par2_sidecar(archive: &Path) -> Option<PathBuf> {
    let mut name = archive.as_os_str().to_os_string();
    name.push(".par2");
    let p = PathBuf::from(name);
    p.exists().then_some(p)
}

/// Perbaiki `archive`. Strategi:
/// 1. Bila ada sidecar `*.par2` dan `par2` terpasang → `par2 repair`.
/// 2. Bila ZIP → `zip -FF` ke `<nama>-fixed.zip`.
/// 3. Selain itu → tidak didukung.
pub fn repair(archive: &Path, cancel: &CancelToken) -> Result<RepairReport> {
    if let Some(par2) = par2_sidecar(archive) {
        if par2_available() {
            return repair_par2(archive, &par2, cancel);
        }
    }

    // Deteksi format toleran-rusak: pakai ekstensi sebagai fallback bila magic
    // bytes sudah korup.
    let kind = archive::detect_kind(archive)
        .ok()
        .or_else(|| archive::kind_from_ext(archive));

    match kind {
        Some(ArchiveKind::Zip) => {
            if !zip_available() {
                return Err(Error::Other(
                    "perintah `zip` tidak terpasang (paket Info-ZIP)".into(),
                ));
            }
            repair_zip(archive, cancel)
        }
        _ => Err(Error::Other(
            "perbaikan otomatis hanya untuk ZIP (`zip -FF`) atau arsip dengan sidecar `.par2`. \
             Untuk recovery RAR, pakai WinRAR/unrar di platform lain."
                .into(),
        )),
    }
}

fn repair_par2(archive: &Path, par2: &Path, cancel: &CancelToken) -> Result<RepairReport> {
    let mut cmd = hardened_command("par2");
    cmd.arg("repair").arg("--").arg(par2);
    let out = run_status(&mut cmd, None, Some(cancel))?;
    let raw = format!("{}{}", out.stdout, out.stderr);
    Ok(RepairReport {
        method: "PAR2".into(),
        output_path: Some(archive.to_path_buf()),
        repaired: out.code == Some(0),
        raw,
    })
}

fn repair_zip(archive: &Path, cancel: &CancelToken) -> Result<RepairReport> {
    let out_path = fixed_sibling(archive);
    let mut cmd = hardened_command("zip");
    cmd.arg("-FF").arg(archive).arg("--out").arg(&out_path);
    // `zip -FF` kadang bertanya "Is this a single-disk archive? (y/n)" — jawab
    // "y" otomatis agar non-interaktif.
    let out = run_status(&mut cmd, Some("y\n"), Some(cancel))?;
    let raw = format!("{}{}", out.stdout, out.stderr);
    let repaired = out.code == Some(0) && out_path.exists();
    Ok(RepairReport {
        method: "zip -FF".into(),
        output_path: out_path.exists().then_some(out_path),
        repaired,
        raw,
    })
}

/// `foo.zip` → `foo-fixed.zip` (di folder yang sama).
fn fixed_sibling(archive: &Path) -> PathBuf {
    let parent = archive.parent().unwrap_or_else(|| Path::new("."));
    let stem = archive
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("archive");
    let ext = archive.extension().and_then(|s| s.to_str()).unwrap_or("zip");
    parent.join(format!("{stem}-fixed.{ext}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_clam_finds_infected_lines() {
        let out = "\
/tmp/a.zip: OK
/tmp/a.zip: Eicar-Test-Signature FOUND
sub/dir/evil.exe: Win.Test.EICAR_HDB-1 FOUND
";
        let f = parse_clam(out);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].path, "/tmp/a.zip");
        assert_eq!(f[0].signature, "Eicar-Test-Signature");
        assert_eq!(f[1].path, "sub/dir/evil.exe");
        assert_eq!(f[1].signature, "Win.Test.EICAR_HDB-1");
    }

    #[test]
    fn parse_clam_clean_output_is_empty() {
        assert!(parse_clam("/tmp/a.zip: OK\n").is_empty());
        assert!(parse_clam("").is_empty());
    }

    #[test]
    fn fixed_sibling_inserts_suffix() {
        assert_eq!(
            fixed_sibling(Path::new("/home/u/foo.zip")),
            PathBuf::from("/home/u/foo-fixed.zip")
        );
        // tanpa ekstensi → fallback .zip
        assert_eq!(
            fixed_sibling(Path::new("bar")),
            PathBuf::from("bar-fixed.zip")
        );
    }

    #[test]
    fn par2_sidecar_absent_for_missing_file() {
        assert!(par2_sidecar(Path::new("/nonexistent/none.zip")).is_none());
    }
}
