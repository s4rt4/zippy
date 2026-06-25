//! Wrapper CLI untuk backend eksternal (7z, unrar).
//!
//! Aturan keras (Planning Doc §2.2, §10.4):
//! - Spawn child dengan `LC_ALL=C` agar parsing stdout tidak pecah oleh locale.
//! - Password dikirim via **stdin**, tidak pernah lewat argv.
//! - Cleanup output parsial saat operasi di-cancel.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::archive::Entry;
use crate::cancel::CancelToken;
use crate::error::{Error, Result};
use crate::progress::{ProgressEvent, ProgressSink};

/// Bangun `Command` dengan environment aman untuk parsing (`LC_ALL=C`).
pub fn hardened_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.env("LC_ALL", "C");
    cmd
}

/// Apakah biner `7z` tersedia di PATH.
pub fn sevenzip_available() -> bool {
    hardened_command("7z")
        .arg("--help")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Apakah biner `unrar` tersedia di PATH (exit code diabaikan — cukup bisa
/// di-spawn).
pub fn unrar_available() -> bool {
    hardened_command("unrar")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

// ---------------------------------------------------------------------------
// 7-Zip
// ---------------------------------------------------------------------------

/// List isi archive 7z via `7z l -slt`.
pub fn sevenzip_list(archive: &Path) -> Result<Vec<Entry>> {
    let out = run_capture(
        hardened_command("7z")
            .arg("l")
            .arg("-slt")
            .arg("-y")
            .arg("--")
            .arg(archive),
        None,
        None,
    )?;
    Ok(parse_7z_slt(&out))
}

/// Extract seluruh isi archive 7z ke `dest`.
pub fn sevenzip_extract(
    archive: &Path,
    dest: &Path,
    password: Option<&str>,
    mode: crate::archive::OverwriteMode,
    prohibited: &[String],
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let start = Instant::now();
    progress.emit(ProgressEvent::Started { total_files: 0 });

    let mut cmd = hardened_command("7z");
    cmd.arg("x")
        .arg("-y")
        .arg(mode.sevenzip_flag())
        .arg(format!("-o{}", dest.display()));
    // Kecualikan tipe terlarang secara rekursif: `-x!*.ext` (case-insensitive
    // via `-scsUTF-8` tak perlu; 7z pola tidak peka huruf di Linux? gunakan -x).
    for ext in prohibited {
        cmd.arg(format!("-x!*.{ext}"));
    }
    cmd.arg("--").arg(archive);
    run_capture(&mut cmd, password, Some(cancel))?;

    progress.emit(ProgressEvent::Finished {
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
    Ok(())
}

/// Buat archive 7z baru dari `inputs`.
pub fn sevenzip_compress(
    inputs: &[&Path],
    dest: &Path,
    password: Option<&str>,
    level: crate::archive::Level,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let start = Instant::now();
    progress.emit(ProgressEvent::Started {
        total_files: inputs.len(),
    });

    let mut cmd = hardened_command("7z");
    cmd.arg("a").arg("-y").arg(format!("-mx={}", level.sevenzip_mx()));
    // 7z hanya mengenkripsi bila flag `-p` ADA (tanpa nilai) — passwordnya
    // sendiri tetap dikirim via stdin (bukan argv, agar tidak bocor lewat
    // /proc/<pid>/cmdline). Tanpa `-p`, password di stdin diabaikan diam-diam.
    if password.is_some() {
        cmd.arg("-p");
    }
    cmd.arg("--").arg(dest);
    for input in inputs {
        cmd.arg(input);
    }
    run_capture(&mut cmd, password, Some(cancel))?;

    progress.emit(ProgressEvent::Finished {
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
    Ok(())
}

/// Uji integritas archive 7z via `7z t`.
pub fn sevenzip_test(
    archive: &Path,
    password: Option<&str>,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let start = Instant::now();
    progress.emit(ProgressEvent::Started { total_files: 0 });
    let mut cmd = hardened_command("7z");
    cmd.arg("t").arg("-y").arg("--").arg(archive);
    run_capture(&mut cmd, password, Some(cancel))?;
    progress.emit(ProgressEvent::Finished {
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
    Ok(())
}

/// Hapus entri `names` dari archive 7z via `7z d` (edit in-place). `-r` agar
/// nama direktori ikut menghapus isinya.
pub fn sevenzip_delete(
    archive: &Path,
    names: &[&str],
    password: Option<&str>,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let start = Instant::now();
    progress.emit(ProgressEvent::Started { total_files: 0 });

    let mut cmd = hardened_command("7z");
    cmd.arg("d").arg("-y").arg("-r");
    if password.is_some() {
        cmd.arg("-p");
    }
    cmd.arg("--").arg(archive);
    for name in names {
        cmd.arg(name.trim_end_matches('/'));
    }
    run_capture(&mut cmd, password, Some(cancel))?;

    progress.emit(ProgressEvent::Finished {
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
    Ok(())
}

/// Extract satu entry `name` dari archive 7z ke bawah `dest_dir`.
pub fn sevenzip_extract_entry(
    archive: &Path,
    name: &str,
    dest_dir: &Path,
    password: Option<&str>,
    cancel: &CancelToken,
) -> Result<()> {
    let mut cmd = hardened_command("7z");
    cmd.arg("x")
        .arg("-y")
        .arg(format!("-o{}", dest_dir.display()))
        .arg("--")
        .arg(archive)
        .arg(name);
    run_capture(&mut cmd, password, Some(cancel))?;
    Ok(())
}

/// Parse output `7z l -slt` menjadi daftar [`Entry`].
#[derive(Default)]
struct SltAcc {
    path: Option<String>,
    size: u64,
    packed: u64,
    is_dir: bool,
    modified: Option<String>,
    crc: Option<u32>,
}

fn parse_7z_slt(stdout: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut started = false;
    let mut acc = SltAcc::default();

    let flush = |entries: &mut Vec<Entry>, acc: &mut SltAcc| {
        if let Some(name) = acc.path.take() {
            entries.push(Entry {
                name,
                size: acc.size,
                compressed_size: acc.packed,
                is_dir: acc.is_dir,
                modified: acc.modified.take(),
                crc32: acc.crc.take(),
            });
        }
        *acc = SltAcc::default();
    };

    for line in stdout.lines() {
        if !started {
            // Daftar entry dimulai setelah baris pemisah "----------".
            if line.starts_with("----------") {
                started = true;
            }
            continue;
        }
        let line = line.trim_end();
        if line.is_empty() {
            flush(&mut entries, &mut acc);
            continue;
        }
        if let Some((k, v)) = line.split_once(" = ") {
            match k {
                "Path" => acc.path = Some(v.to_string()),
                "Size" => acc.size = v.parse().unwrap_or(0),
                "Packed Size" => acc.packed = v.parse().unwrap_or(0),
                "Folder" => acc.is_dir = v == "+",
                "Attributes" => {
                    if v.starts_with('D') || v.contains("D_") {
                        acc.is_dir = true;
                    }
                }
                // "2005-09-03 13:32:50" → ambil menit-presisi "YYYY-MM-DD HH:MM".
                "Modified" if v.len() >= 16 => acc.modified = Some(v[..16].to_string()),
                "CRC" => acc.crc = u32::from_str_radix(v.trim(), 16).ok(),
                _ => {}
            }
        }
    }
    flush(&mut entries, &mut acc);
    entries
}

// ---------------------------------------------------------------------------
// RAR (extract only — format proprietary)
// ---------------------------------------------------------------------------

/// List isi archive RAR via `unrar lb` (nama saja).
pub fn unrar_list(archive: &Path) -> Result<Vec<Entry>> {
    let out = run_capture(
        hardened_command("unrar").arg("lb").arg("--").arg(archive),
        None,
        None,
    )?;
    Ok(out
        .lines()
        .filter(|l| !l.is_empty())
        .map(|name| Entry::basic(name.to_string(), 0, 0, name.ends_with('/')))
        .collect())
}

/// Extract seluruh isi RAR ke `dest`.
pub fn unrar_extract(
    archive: &Path,
    dest: &Path,
    password: Option<&str>,
    mode: crate::archive::OverwriteMode,
    prohibited: &[String],
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let start = Instant::now();
    progress.emit(ProgressEvent::Started { total_files: 0 });

    let mut dest_arg = dest.as_os_str().to_os_string();
    dest_arg.push("/"); // unrar butuh trailing slash untuk direktori tujuan

    let mut cmd = hardened_command("unrar");
    cmd.arg("x").arg(mode.unrar_flag());
    // Kecualikan tipe terlarang: unrar `-x*.ext`.
    for ext in prohibited {
        cmd.arg(format!("-x*.{ext}"));
    }
    cmd.arg("--").arg(archive).arg(dest_arg);
    run_capture(&mut cmd, password, Some(cancel))?;

    progress.emit(ProgressEvent::Finished {
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
    Ok(())
}

/// Uji integritas archive RAR via `unrar t`.
pub fn unrar_test(
    archive: &Path,
    password: Option<&str>,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let start = Instant::now();
    progress.emit(ProgressEvent::Started { total_files: 0 });
    let mut cmd = hardened_command("unrar");
    cmd.arg("t").arg("--").arg(archive);
    run_capture(&mut cmd, password, Some(cancel))?;
    progress.emit(ProgressEvent::Finished {
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
    Ok(())
}

/// Extract satu entry `name` dari archive RAR ke bawah `dest_dir`.
pub fn unrar_extract_entry(
    archive: &Path,
    name: &str,
    dest_dir: &Path,
    password: Option<&str>,
    cancel: &CancelToken,
) -> Result<()> {
    let mut dest_arg = dest_dir.as_os_str().to_os_string();
    dest_arg.push("/");
    let mut cmd = hardened_command("unrar");
    cmd.arg("x")
        .arg("-y")
        .arg("--")
        .arg(archive)
        .arg(name)
        .arg(dest_arg);
    run_capture(&mut cmd, password, Some(cancel))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// util
// ---------------------------------------------------------------------------

/// Jalankan command, kirim `password` (bila ada) via stdin, kumpulkan stdout.
///
/// Bila `cancel` diberikan, child di-poll: saat dibatalkan child di-`kill` dan
/// fungsi mengembalikan [`Error::Cancelled`]. stdout/stderr dikuras di thread
/// terpisah agar pipe yang penuh tidak men-deadlock proses (mis. 7z mencetak
/// daftar file panjang).
fn run_capture(
    cmd: &mut Command,
    password: Option<&str>,
    cancel: Option<&CancelToken>,
) -> Result<String> {
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            Error::Other(format!("biner backend tidak ditemukan: {e}"))
        }
        _ => Error::Io(e),
    })?;

    // Password via stdin (tidak pernah argv). 7z meminta konfirmasi dua kali
    // saat membuat archive, jadi kirim dua baris untuk amannya.
    if let Some(mut stdin) = child.stdin.take() {
        if let Some(pw) = password {
            let _ = writeln!(stdin, "{pw}");
            let _ = writeln!(stdin, "{pw}");
        }
        // stdin di-drop di sini → EOF.
    }

    // Kuras stdout & stderr di thread sendiri (cegah deadlock pipe penuh).
    let out_reader = drain_thread(child.stdout.take());
    let err_reader = drain_thread(child.stderr.take());

    let status = loop {
        if let Some(token) = cancel {
            if token.is_cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                let _ = out_reader.join();
                let _ = err_reader.join();
                return Err(Error::Cancelled);
            }
        }
        match child.try_wait()? {
            Some(status) => break status,
            None => thread::sleep(Duration::from_millis(50)),
        }
    };

    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();

    if !status.success() {
        let combined = format!("{stdout}{stderr}").to_lowercase();
        if combined.contains("password")
            || combined.contains("wrong password")
            || combined.contains("crc failed")
        {
            return Err(Error::Password);
        }
        return Err(Error::Other(format!(
            "backend keluar dengan status {}: {}",
            status,
            stderr.trim()
        )));
    }
    Ok(stdout)
}

/// Hasil mentah menjalankan proses: kode keluar + stdout + stderr.
///
/// Berbeda dari [`run_capture`], fungsi ini TIDAK menganggap exit non-zero
/// sebagai error — beberapa tool memakai kode keluar sebagai sinyal hasil
/// (mis. `clamscan` keluar 1 saat menemukan virus, `par2` keluar 1 saat butuh
/// perbaikan). Pemanggil yang menafsirkan kode keluar sendiri.
pub struct ProcOutput {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Jalankan `cmd`, opsional kirim `input` ke stdin, hormati `cancel`, kembalikan
/// kode+output mentah.
pub fn run_status(
    cmd: &mut Command,
    input: Option<&str>,
    cancel: Option<&CancelToken>,
) -> Result<ProcOutput> {
    cmd.stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            Error::Other(format!("biner tidak ditemukan: {e}"))
        }
        _ => Error::Io(e),
    })?;

    if let Some(mut stdin) = child.stdin.take() {
        if let Some(s) = input {
            let _ = stdin.write_all(s.as_bytes());
        }
        // stdin di-drop → EOF.
    }

    let out_reader = drain_thread(child.stdout.take());
    let err_reader = drain_thread(child.stderr.take());

    let status = loop {
        if let Some(token) = cancel {
            if token.is_cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                let _ = out_reader.join();
                let _ = err_reader.join();
                return Err(Error::Cancelled);
            }
        }
        match child.try_wait()? {
            Some(status) => break status,
            None => thread::sleep(Duration::from_millis(50)),
        }
    };

    Ok(ProcOutput {
        code: status.code(),
        stdout: out_reader.join().unwrap_or_default(),
        stderr: err_reader.join().unwrap_or_default(),
    })
}

/// Spawn thread yang membaca seluruh `reader` menjadi `String` (lossy UTF-8).
fn drain_thread<R: Read + Send + 'static>(reader: Option<R>) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut r) = reader {
            let _ = r.read_to_end(&mut buf);
        }
        String::from_utf8_lossy(&buf).into_owned()
    })
}
