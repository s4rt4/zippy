//! Integration test interop lintas-tool (Planning Doc §10.2).
//!
//! Memastikan: (a) arsip yang dibuat tool eksternal (zip/tar/gzip/7z) bisa
//! dibuka Zippy, dan (b) arsip buatan Zippy bisa dibuka tool eksternal. Tiap
//! test di-skip otomatis bila tool yang dibutuhkan tidak terpasang.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use zippy_core::archive::{compress, extract_all, list};
use zippy_core::{CancelToken, Error, NullSink};

// ---------------------------------------------------------------------------
// helper
// ---------------------------------------------------------------------------

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("--help")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Jalankan tool eksternal; panik bila gagal (test memang ingin sukses).
fn run(prog: &str, args: &[&str], cwd: Option<&Path>) {
    let mut c = Command::new(prog);
    c.args(args).stdout(Stdio::null()).stderr(Stdio::null());
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    let status = c.status().unwrap_or_else(|e| panic!("{prog} tak bisa dijalankan: {e}"));
    assert!(status.success(), "{prog} {args:?} keluar dengan {status}");
}

/// Sukses/gagal tanpa panik (untuk kasus yang sengaja diharapkan gagal).
fn try_run(prog: &str, args: &[&str], cwd: Option<&Path>) -> bool {
    let mut c = Command::new(prog);
    c.args(args).stdout(Stdio::null()).stderr(Stdio::null());
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    c.status().map(|s| s.success()).unwrap_or(false)
}

fn mk_tree(root: &Path) {
    fs::create_dir_all(root.join("sub")).unwrap();
    fs::write(root.join("a.txt"), b"halo dunia\n").unwrap();
    fs::write(root.join("sub/b.txt"), b"isi kedua\n").unwrap();
}

fn check_tree(out: &Path) {
    assert_eq!(fs::read(out.join("a.txt")).unwrap(), b"halo dunia\n");
    assert_eq!(fs::read(out.join("sub/b.txt")).unwrap(), b"isi kedua\n");
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn nocancel() -> CancelToken {
    CancelToken::new()
}

// ---------------------------------------------------------------------------
// External → Zippy (Zippy membaca arsip tool lain)
// ---------------------------------------------------------------------------

#[test]
fn zippy_reads_system_zip() {
    if !have("zip") {
        eprintln!("skip: zip tidak terpasang");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    mk_tree(&src);
    let archive = tmp.path().join("ext.zip");
    run("zip", &["-q", "-r", s(&archive), "a.txt", "sub"], Some(&src));

    assert!(!list(&archive, None).unwrap().is_empty());
    let out = tmp.path().join("out");
    extract_all(&archive, &out, None, &nocancel(), &NullSink).unwrap();
    check_tree(&out);
}

#[test]
fn zippy_reads_system_targz() {
    if !have("tar") {
        eprintln!("skip: tar tidak terpasang");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    mk_tree(&src);
    let archive = tmp.path().join("ext.tar.gz");
    run("tar", &["czf", s(&archive), "a.txt", "sub"], Some(&src));

    let out = tmp.path().join("out");
    extract_all(&archive, &out, None, &nocancel(), &NullSink).unwrap();
    check_tree(&out);
}

#[test]
fn zippy_reads_system_gzip_single() {
    if !have("gzip") {
        eprintln!("skip: gzip tidak terpasang");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    fs::write(dir.join("note.txt"), b"satu berkas\n").unwrap();
    run("gzip", &["-k", "note.txt"], Some(dir));

    let out = tmp.path().join("out");
    extract_all(&dir.join("note.txt.gz"), &out, None, &nocancel(), &NullSink).unwrap();
    assert_eq!(fs::read(out.join("note.txt")).unwrap(), b"satu berkas\n");
}

#[test]
fn zippy_reads_system_7z() {
    if !have("7z") {
        eprintln!("skip: 7z tidak terpasang");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    mk_tree(&src);
    let archive = tmp.path().join("ext.7z");
    run("7z", &["a", "-y", s(&archive), "a.txt", "sub"], Some(&src));

    let out = tmp.path().join("out");
    extract_all(&archive, &out, None, &nocancel(), &NullSink).unwrap();
    check_tree(&out);
}

// ---------------------------------------------------------------------------
// Zippy → External (arsip Zippy dibaca tool lain)
// ---------------------------------------------------------------------------

#[test]
fn zippy_zip_opens_in_unzip() {
    if !have("unzip") {
        eprintln!("skip: unzip tidak terpasang");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    mk_tree(&src);
    let archive = tmp.path().join("z.zip");
    let a = src.join("a.txt");
    let sub = src.join("sub");
    let inputs: Vec<&Path> = vec![a.as_path(), sub.as_path()];
    compress(&inputs, &archive, None, &nocancel(), &NullSink).unwrap();

    let out = tmp.path().join("out");
    run("unzip", &["-q", s(&archive), "-d", s(&out)], None);
    check_tree(&out);
}

#[test]
fn zippy_targz_opens_in_tar() {
    if !have("tar") {
        eprintln!("skip: tar tidak terpasang");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    mk_tree(&src);
    let archive = tmp.path().join("z.tar.gz");
    let a = src.join("a.txt");
    let sub = src.join("sub");
    let inputs: Vec<&Path> = vec![a.as_path(), sub.as_path()];
    compress(&inputs, &archive, None, &nocancel(), &NullSink).unwrap();

    let out = tmp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    run("tar", &["xzf", s(&archive), "-C", s(&out)], None);
    check_tree(&out);
}

#[test]
fn zippy_7z_opens_in_7z() {
    if !have("7z") {
        eprintln!("skip: 7z tidak terpasang");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    mk_tree(&src);
    let archive = tmp.path().join("z.7z");
    let a = src.join("a.txt");
    let sub = src.join("sub");
    let inputs: Vec<&Path> = vec![a.as_path(), sub.as_path()];
    compress(&inputs, &archive, None, &nocancel(), &NullSink).unwrap();

    let out = tmp.path().join("out");
    run("7z", &["x", "-y", &format!("-o{}", s(&out)), s(&archive)], None);
    check_tree(&out);
}

// ---------------------------------------------------------------------------
// 7z password end-to-end (Planning Doc §10.1 — password via stdin)
// ---------------------------------------------------------------------------

#[test]
fn sevenzip_password_roundtrip() {
    if !have("7z") {
        eprintln!("skip: 7z tidak terpasang");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    mk_tree(&src);
    let archive = tmp.path().join("enc.7z");
    let a = src.join("a.txt");
    let sub = src.join("sub");
    let inputs: Vec<&Path> = vec![a.as_path(), sub.as_path()];

    // Compress dengan password (lewat stdin + flag -p).
    compress(&inputs, &archive, Some("rahasia"), &nocancel(), &NullSink).unwrap();

    // Password benar → sukses.
    let ok = tmp.path().join("ok");
    extract_all(&archive, &ok, Some("rahasia"), &nocancel(), &NullSink).unwrap();
    check_tree(&ok);

    // Tanpa password → HARUS gagal (membuktikan arsip benar-benar terenkripsi).
    let bad = tmp.path().join("bad");
    assert!(
        extract_all(&archive, &bad, None, &nocancel(), &NullSink).is_err(),
        "extract 7z terenkripsi tanpa password harusnya gagal"
    );

    // Tool eksternal pun tak boleh bisa extract dengan password salah
    // (-pWRONGPASS di argv: deterministik gagal, tak menunggu prompt stdin).
    let ext = tmp.path().join("ext");
    assert!(
        !try_run(
            "7z",
            &["x", "-y", "-pWRONGPASS", &format!("-o{}", s(&ext)), s(&archive)],
            None
        ),
        "7z eksternal harusnya gagal extract dengan password salah"
    );

    // Password salah → Error::Password.
    let wrong = tmp.path().join("wrong");
    match extract_all(&archive, &wrong, Some("salahbanget"), &nocancel(), &NullSink) {
        Err(Error::Password) => {}
        other => panic!("harusnya Error::Password, dapat {other:?}"),
    }
}
