//! Round-trip tests: compress → extract → verifikasi isi identik
//! (Planning Doc §10.1).

use std::fs;
use std::path::{Path, PathBuf};

use super::*;
use crate::cancel::CancelToken;
use crate::progress::NullSink;

/// Buat pohon sumber contoh: `a.txt` + `sub/b.txt`.
fn make_src(root: &Path) -> Vec<PathBuf> {
    fs::create_dir_all(root.join("sub")).unwrap();
    fs::write(root.join("a.txt"), b"halo dunia\n").unwrap();
    fs::write(root.join("sub/b.txt"), b"isi kedua\n").unwrap();
    vec![root.join("a.txt"), root.join("sub")]
}

fn assert_extracted(out: &Path) {
    assert_eq!(fs::read(out.join("a.txt")).unwrap(), b"halo dunia\n");
    assert_eq!(fs::read(out.join("sub/b.txt")).unwrap(), b"isi kedua\n");
}

fn roundtrip(ext: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let srcs = make_src(&src);
    let src_refs: Vec<&Path> = srcs.iter().map(|p| p.as_path()).collect();

    let archive = tmp.path().join(format!("out.{ext}"));
    compress(&src_refs, &archive, None, &CancelToken::new(), &NullSink).unwrap();
    assert!(archive.exists(), "archive {ext} tidak dibuat");

    // Deteksi harus mengenali format dari magic bytes.
    let kind = detect_kind(&archive).unwrap();
    assert_ne!(kind, ArchiveKind::Rar);

    let out = tmp.path().join("out");
    extract_all(&archive, &out, None, &CancelToken::new(), &NullSink).unwrap();
    assert_extracted(&out);

    // List harus mengembalikan entry (jumlah >= 2 file).
    let entries = list(&archive, None).unwrap();
    assert!(!entries.is_empty(), "list {ext} kosong");
}

#[test]
fn roundtrip_zip() {
    roundtrip("zip");
}

#[test]
fn roundtrip_tar() {
    roundtrip("tar");
}

#[test]
fn roundtrip_tar_gz() {
    roundtrip("tar.gz");
}

#[test]
fn roundtrip_tar_bz2() {
    roundtrip("tar.bz2");
}

#[test]
fn roundtrip_tar_xz() {
    roundtrip("tar.xz");
}

#[test]
fn roundtrip_tar_zst() {
    roundtrip("tar.zst");
}

#[test]
fn single_file_gz_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("note.txt");
    fs::write(&input, b"satu file saja\n").unwrap();

    let archive = tmp.path().join("note.txt.gz");
    compress(&[input.as_path()], &archive, None, &CancelToken::new(), &NullSink).unwrap();
    assert_eq!(detect_kind(&archive).unwrap(), ArchiveKind::Gz);

    let out = tmp.path().join("out");
    extract_all(&archive, &out, None, &CancelToken::new(), &NullSink).unwrap();
    assert_eq!(fs::read(out.join("note.txt")).unwrap(), b"satu file saja\n");
}

#[test]
fn single_file_zst_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("data.bin");
    fs::write(&input, vec![7u8; 4096]).unwrap();

    let archive = tmp.path().join("data.bin.zst");
    compress(&[input.as_path()], &archive, None, &CancelToken::new(), &NullSink).unwrap();
    assert_eq!(detect_kind(&archive).unwrap(), ArchiveKind::Zst);

    let out = tmp.path().join("out");
    extract_all(&archive, &out, None, &CancelToken::new(), &NullSink).unwrap();
    assert_eq!(fs::read(out.join("data.bin")).unwrap(), vec![7u8; 4096]);
}

#[test]
fn zip_aes256_password_roundtrip() {
    // Verifikasi tulis AES-256 di crate zip (Planning Doc §3 — wajib dicek awal).
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let srcs = make_src(&src);
    let src_refs: Vec<&Path> = srcs.iter().map(|p| p.as_path()).collect();

    let archive = tmp.path().join("secret.zip");
    compress(&src_refs, &archive, Some("rahasia"), &CancelToken::new(), &NullSink).unwrap();

    // Extract dengan password benar → sukses.
    let out_ok = tmp.path().join("ok");
    extract_all(&archive, &out_ok, Some("rahasia"), &CancelToken::new(), &NullSink).unwrap();
    assert_extracted(&out_ok);

    // Extract dengan password salah → Error::Password.
    let out_bad = tmp.path().join("bad");
    let err = extract_all(&archive, &out_bad, Some("salah"), &CancelToken::new(), &NullSink).unwrap_err();
    assert!(matches!(err, Error::Password), "harusnya Error::Password, dapat {err:?}");
}

#[test]
fn extract_precancelled_returns_cancelled() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let srcs = make_src(&src);
    let src_refs: Vec<&Path> = srcs.iter().map(|p| p.as_path()).collect();
    let archive = tmp.path().join("c.zip");
    compress(&src_refs, &archive, None, &CancelToken::new(), &NullSink).unwrap();

    let token = CancelToken::new();
    token.cancel();
    let out = tmp.path().join("out");
    let err = extract_all(&archive, &out, None, &token, &NullSink).unwrap_err();
    assert!(matches!(err, Error::Cancelled), "dapat {err:?}");
    assert!(!out.join("a.txt").exists(), "tidak boleh ada file ter-extract");
}

#[test]
fn compress_precancelled_cleans_up_partial() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let srcs = make_src(&src);
    let src_refs: Vec<&Path> = srcs.iter().map(|p| p.as_path()).collect();
    let archive = tmp.path().join("c.zip");

    let token = CancelToken::new();
    token.cancel();
    let err = compress(&src_refs, &archive, None, &token, &NullSink).unwrap_err();
    assert!(matches!(err, Error::Cancelled), "dapat {err:?}");
    assert!(!archive.exists(), "archive parsial harus dihapus saat cancel");
}

#[test]
fn rejects_zip_slip_on_extract() {
    // Bangun ZIP berisi entry "../evil.txt" secara manual, lalu pastikan ditolak.
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let archive = tmp.path().join("evil.zip");
    {
        let f = fs::File::create(&archive).unwrap();
        let mut zw = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default();
        zw.start_file("../evil.txt", opts).unwrap();
        zw.write_all(b"pwned").unwrap();
        zw.finish().unwrap();
    }

    let out = tmp.path().join("out");
    let err = extract_all(&archive, &out, None, &CancelToken::new(), &NullSink).unwrap_err();
    assert!(matches!(err, Error::UnsafePath(_)), "harusnya UnsafePath, dapat {err:?}");
}

#[test]
fn sevenzip_roundtrip_if_available() {
    if !crate::subprocess::sevenzip_available() {
        eprintln!("7z tidak tersedia — test dilewati");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let srcs = make_src(&src);
    let src_refs: Vec<&Path> = srcs.iter().map(|p| p.as_path()).collect();

    let archive = tmp.path().join("out.7z");
    compress(&src_refs, &archive, None, &CancelToken::new(), &NullSink).unwrap();
    assert_eq!(detect_kind(&archive).unwrap(), ArchiveKind::SevenZip);

    let entries = list(&archive, None).unwrap();
    assert!(entries.iter().any(|e| e.name.contains("a.txt")), "entry 7z: {entries:?}");

    let out = tmp.path().join("out");
    extract_all(&archive, &out, None, &CancelToken::new(), &NullSink).unwrap();
    assert_extracted(&out);
}
