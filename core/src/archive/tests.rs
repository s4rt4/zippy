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
fn overwrite_mode_skip_rename_overwrite() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let srcs = make_src(&src);
    let src_refs: Vec<&Path> = srcs.iter().map(|p| p.as_path()).collect();
    let archive = tmp.path().join("out.zip");
    compress(&src_refs, &archive, None, &CancelToken::new(), &NullSink).unwrap();

    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    // Pre-seed a.txt dengan isi berbeda untuk memicu konflik.
    fs::write(out.join("a.txt"), b"LAMA").unwrap();

    // Skip: a.txt yang sudah ada tidak berubah; sub/b.txt baru tetap dibuat.
    extract_all_with(
        &archive,
        &out,
        None,
        OverwriteMode::Skip,
        &[],
        &CancelToken::new(),
        &NullSink,
    )
    .unwrap();
    assert_eq!(fs::read(out.join("a.txt")).unwrap(), b"LAMA");
    assert_eq!(fs::read(out.join("sub/b.txt")).unwrap(), b"isi kedua\n");

    // Rename: a.txt asli tetap, salinan baru jadi "a (1).txt".
    extract_all_with(
        &archive,
        &out,
        None,
        OverwriteMode::Rename,
        &[],
        &CancelToken::new(),
        &NullSink,
    )
    .unwrap();
    assert_eq!(fs::read(out.join("a.txt")).unwrap(), b"LAMA");
    assert_eq!(fs::read(out.join("a (1).txt")).unwrap(), b"halo dunia\n");

    // Overwrite: a.txt ditimpa isi dari arsip.
    extract_all_with(
        &archive,
        &out,
        None,
        OverwriteMode::Overwrite,
        &[],
        &CancelToken::new(),
        &NullSink,
    )
    .unwrap();
    assert_eq!(fs::read(out.join("a.txt")).unwrap(), b"halo dunia\n");
}

#[test]
fn prohibited_ext_excluded_from_extract() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("ok.txt"), b"aman\n").unwrap();
    fs::write(src.join("evil.desktop"), b"[Desktop Entry]\n").unwrap();
    let srcs = [src.join("ok.txt"), src.join("evil.desktop")];
    let refs: Vec<&Path> = srcs.iter().map(|p| p.as_path()).collect();
    let archive = tmp.path().join("p.zip");
    compress(&refs, &archive, None, &CancelToken::new(), &NullSink).unwrap();

    let out = tmp.path().join("out");
    let banned = vec!["desktop".to_string()];
    extract_all_with(
        &archive,
        &out,
        None,
        OverwriteMode::Overwrite,
        &banned,
        &CancelToken::new(),
        &NullSink,
    )
    .unwrap();
    assert!(out.join("ok.txt").exists(), "berkas aman harus diekstrak");
    assert!(
        !out.join("evil.desktop").exists(),
        "berkas terlarang tidak boleh diekstrak"
    );
}

#[test]
fn convert_zip_to_targz_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let srcs = make_src(&src);
    let refs: Vec<&Path> = srcs.iter().map(|p| p.as_path()).collect();
    let zip = tmp.path().join("a.zip");
    compress(&refs, &zip, None, &CancelToken::new(), &NullSink).unwrap();

    let tgz = tmp.path().join("a.tar.gz");
    convert(&zip, &tgz, None, None, Level::Normal, &CancelToken::new(), &NullSink).unwrap();
    assert_eq!(detect_kind(&tgz).unwrap(), ArchiveKind::TarGz);

    let out = tmp.path().join("out");
    extract_all(&tgz, &out, None, &CancelToken::new(), &NullSink).unwrap();
    assert_extracted(&out);
}

#[test]
fn zip_comment_set_and_read() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let srcs = make_src(&src);
    let refs: Vec<&Path> = srcs.iter().map(|p| p.as_path()).collect();
    let zip = tmp.path().join("c.zip");
    compress(&refs, &zip, None, &CancelToken::new(), &NullSink).unwrap();

    assert_eq!(read_comment(&zip).unwrap(), "");
    set_comment(&zip, "Halo komentar", &CancelToken::new(), &NullSink).unwrap();
    assert_eq!(read_comment(&zip).unwrap(), "Halo komentar");

    // Entri harus tetap utuh setelah set komentar.
    let out = tmp.path().join("out");
    extract_all(&zip, &out, None, &CancelToken::new(), &NullSink).unwrap();
    assert_extracted(&out);
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
fn test_passes_on_valid_zip() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let srcs = make_src(&src);
    let src_refs: Vec<&Path> = srcs.iter().map(|p| p.as_path()).collect();
    let archive = tmp.path().join("ok.zip");
    compress(&src_refs, &archive, None, &CancelToken::new(), &NullSink).unwrap();

    test(&archive, None, &CancelToken::new(), &NullSink).expect("archive valid harus lulus test");
}

#[test]
fn test_detects_corruption() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let srcs = make_src(&src);
    let src_refs: Vec<&Path> = srcs.iter().map(|p| p.as_path()).collect();
    let archive = tmp.path().join("bad.zip");
    compress(&src_refs, &archive, None, &CancelToken::new(), &NullSink).unwrap();

    // Potong ekor → central directory rusak → test harus gagal.
    let data = fs::read(&archive).unwrap();
    fs::write(&archive, &data[..data.len() - 20]).unwrap();
    assert!(test(&archive, None, &CancelToken::new(), &NullSink).is_err());
}

#[test]
fn extract_entry_single_file_from_zip() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let srcs = make_src(&src);
    let src_refs: Vec<&Path> = srcs.iter().map(|p| p.as_path()).collect();
    let archive = tmp.path().join("e.zip");
    compress(&src_refs, &archive, None, &CancelToken::new(), &NullSink).unwrap();

    let out_dir = tmp.path().join("view");
    let out = extract_entry(&archive, "a.txt", &out_dir, None, &CancelToken::new()).unwrap();
    assert_eq!(fs::read(&out).unwrap(), b"halo dunia\n");
    assert_eq!(out, out_dir.join("a.txt"));
}

#[test]
fn tar_compress_emits_per_file_progress() {
    use crate::progress::{ProgressEvent, ProgressSink};
    use std::sync::Mutex;

    // Sink yang mengumpulkan nama berkas dari FileProcessed + total dari Started.
    #[derive(Default)]
    struct CollectSink {
        names: Mutex<Vec<String>>,
        total: Mutex<usize>,
    }
    impl ProgressSink for CollectSink {
        fn emit(&self, ev: ProgressEvent) {
            match ev {
                ProgressEvent::Started { total_files } => *self.total.lock().unwrap() = total_files,
                ProgressEvent::FileProcessed { name, .. } => {
                    self.names.lock().unwrap().push(name)
                }
                _ => {}
            }
        }
    }

    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let srcs = make_src(&src); // a.txt + sub/ (berisi b.txt) → 2 berkas
    let src_refs: Vec<&Path> = srcs.iter().map(|p| p.as_path()).collect();

    let archive = tmp.path().join("out.tar");
    let sink = CollectSink::default();
    compress(&src_refs, &archive, None, &CancelToken::new(), &sink).unwrap();

    let names = sink.names.lock().unwrap();
    assert_eq!(*sink.total.lock().unwrap(), 2, "total_files harus jumlah berkas");
    assert_eq!(names.len(), 2, "harus ada FileProcessed per berkas: {names:?}");
    assert!(names.iter().any(|n| n.contains("a.txt")), "{names:?}");
    assert!(names.iter().any(|n| n.contains("b.txt")), "{names:?}");
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

// ---------------------------------------------------------------------------
// Delete (hapus entri in-place)
// ---------------------------------------------------------------------------

/// Hapus satu berkas dari zip; sisanya tetap utuh & bisa di-extract.
#[test]
fn delete_zip_removes_file() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let srcs = make_src(&src);
    let src_refs: Vec<&Path> = srcs.iter().map(|p| p.as_path()).collect();

    let archive = tmp.path().join("out.zip");
    compress(&src_refs, &archive, None, &CancelToken::new(), &NullSink).unwrap();

    delete(&archive, &["a.txt"], None, &CancelToken::new(), &NullSink).unwrap();

    let names: Vec<String> = list(&archive, None).unwrap().into_iter().map(|e| e.name).collect();
    assert!(!names.iter().any(|n| n == "a.txt"), "a.txt harusnya terhapus: {names:?}");
    assert!(names.iter().any(|n| n.contains("b.txt")), "b.txt harus tetap ada: {names:?}");

    // Sisa archive masih valid & ter-extract.
    let out = tmp.path().join("out");
    extract_all(&archive, &out, None, &CancelToken::new(), &NullSink).unwrap();
    assert_eq!(fs::read(out.join("sub/b.txt")).unwrap(), b"isi kedua\n");
    assert!(!out.join("a.txt").exists());
}

/// Menghapus nama direktori ikut menghapus seluruh isinya (prefiks).
#[test]
fn delete_zip_removes_directory_recursively() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let srcs = make_src(&src);
    let src_refs: Vec<&Path> = srcs.iter().map(|p| p.as_path()).collect();

    let archive = tmp.path().join("out.zip");
    compress(&src_refs, &archive, None, &CancelToken::new(), &NullSink).unwrap();

    delete(&archive, &["sub"], None, &CancelToken::new(), &NullSink).unwrap();

    let names: Vec<String> = list(&archive, None).unwrap().into_iter().map(|e| e.name).collect();
    assert!(!names.iter().any(|n| n.contains("b.txt")), "isi sub/ harusnya terhapus: {names:?}");
    assert!(names.iter().any(|n| n == "a.txt"), "a.txt harus tetap ada: {names:?}");
}

/// Hapus dari tar.gz: tulis-ulang stream tanpa entri terpilih.
#[test]
fn delete_targz_removes_file() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let srcs = make_src(&src);
    let src_refs: Vec<&Path> = srcs.iter().map(|p| p.as_path()).collect();

    let archive = tmp.path().join("out.tar.gz");
    compress(&src_refs, &archive, None, &CancelToken::new(), &NullSink).unwrap();

    delete(&archive, &["a.txt"], None, &CancelToken::new(), &NullSink).unwrap();
    assert_eq!(detect_kind(&archive).unwrap(), ArchiveKind::TarGz);

    let out = tmp.path().join("out");
    extract_all(&archive, &out, None, &CancelToken::new(), &NullSink).unwrap();
    assert!(!out.join("a.txt").exists(), "a.txt harusnya terhapus");
    assert_eq!(fs::read(out.join("sub/b.txt")).unwrap(), b"isi kedua\n");
}

/// Hapus dari zip AES-256: entri tersisa harus tetap bisa didekripsi dengan
/// password yang sama (jalur dekripsi→enkripsi-ulang, bukan salin mentah).
#[test]
fn delete_aes_zip_keeps_remaining_decryptable() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let srcs = make_src(&src);
    let src_refs: Vec<&Path> = srcs.iter().map(|p| p.as_path()).collect();

    let archive = tmp.path().join("enc.zip");
    compress(&src_refs, &archive, Some("rahasia"), &CancelToken::new(), &NullSink).unwrap();

    // Tanpa password → harus minta password (Error::Password), bukan merusak.
    let err = delete(&archive, &["a.txt"], None, &CancelToken::new(), &NullSink).unwrap_err();
    assert!(matches!(err, Error::Password), "harusnya Error::Password, dapat {err:?}");

    // Dengan password → sukses; sisa tetap terenkripsi & ter-extract benar.
    delete(&archive, &["a.txt"], Some("rahasia"), &CancelToken::new(), &NullSink).unwrap();

    // Masih terenkripsi (extract tanpa password gagal).
    let nopw = tmp.path().join("nopw");
    assert!(extract_all(&archive, &nopw, None, &CancelToken::new(), &NullSink).is_err());

    // Password benar → b.txt utuh, a.txt hilang.
    let out = tmp.path().join("out");
    extract_all(&archive, &out, Some("rahasia"), &CancelToken::new(), &NullSink).unwrap();
    assert_eq!(fs::read(out.join("sub/b.txt")).unwrap(), b"isi kedua\n");
    assert!(!out.join("a.txt").exists());
}

/// RAR & stream tunggal tidak mendukung hapus → error (bukan panik).
#[test]
fn delete_single_stream_unsupported() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("note.txt");
    fs::write(&input, b"x\n").unwrap();
    let archive = tmp.path().join("note.txt.gz");
    compress(&[input.as_path()], &archive, None, &CancelToken::new(), &NullSink).unwrap();

    let err = delete(&archive, &["note.txt"], None, &CancelToken::new(), &NullSink).unwrap_err();
    assert!(matches!(err, Error::Other(_)), "harusnya Error::Other, dapat {err:?}");
}

// ---------------------------------------------------------------------------
// Compression level
// ---------------------------------------------------------------------------

/// `Level::Store` menghasilkan zip tanpa kompresi (compressed ≈ ukuran asli),
/// `Level::Best` lebih kecil — dan keduanya tetap round-trip benar.
#[test]
fn zip_levels_affect_size_and_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("big.txt");
    // Data sangat kompresibel (berulang) agar selisih level jelas.
    fs::write(&input, "ABCD".repeat(50_000)).unwrap();
    let refs = [input.as_path()];

    let stored = tmp.path().join("stored.zip");
    let best = tmp.path().join("best.zip");
    compress_with_level(&refs, &stored, None, Level::Store, &CancelToken::new(), &NullSink).unwrap();
    compress_with_level(&refs, &best, None, Level::Best, &CancelToken::new(), &NullSink).unwrap();

    let stored_sz = fs::metadata(&stored).unwrap().len();
    let best_sz = fs::metadata(&best).unwrap().len();
    assert!(best_sz < stored_sz, "Best ({best_sz}) harus < Store ({stored_sz})");

    // Round-trip kedua level.
    for arc in [&stored, &best] {
        let out = tmp.path().join(format!("out-{}", arc.file_stem().unwrap().to_string_lossy()));
        extract_all(arc, &out, None, &CancelToken::new(), &NullSink).unwrap();
        assert_eq!(fs::read(out.join("big.txt")).unwrap(), "ABCD".repeat(50_000).into_bytes());
    }
}

// ---------------------------------------------------------------------------
// Deteksi enkripsi lemah
// ---------------------------------------------------------------------------

/// AES-256 → bukan enkripsi lemah; archive tanpa password juga `false`.
#[test]
fn weak_encryption_false_for_aes_and_plain() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let srcs = make_src(&src);
    let src_refs: Vec<&Path> = srcs.iter().map(|p| p.as_path()).collect();

    let plain = tmp.path().join("plain.zip");
    compress(&src_refs, &plain, None, &CancelToken::new(), &NullSink).unwrap();
    assert!(!has_weak_encryption(&plain).unwrap());

    let aes = tmp.path().join("aes.zip");
    compress(&src_refs, &aes, Some("rahasia"), &CancelToken::new(), &NullSink).unwrap();
    assert!(!has_weak_encryption(&aes).unwrap(), "AES-256 bukan enkripsi lemah");
}

// (Deteksi ZipCrypto legasi diuji di tests/interop.rs lewat tool `zip` sistem;
// API enkripsi-deprecated crate `zip` bersifat pub(crate) sehingga tak bisa
// dibuat dari unit test ini.)

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
