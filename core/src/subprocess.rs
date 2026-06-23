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
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let start = Instant::now();
    progress.emit(ProgressEvent::Started { total_files: 0 });

    let mut cmd = hardened_command("7z");
    cmd.arg("x")
        .arg("-y")
        .arg(format!("-o{}", dest.display()))
        .arg("--")
        .arg(archive);
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
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let start = Instant::now();
    progress.emit(ProgressEvent::Started {
        total_files: inputs.len(),
    });

    let mut cmd = hardened_command("7z");
    cmd.arg("a").arg("-y").arg("--").arg(dest);
    for input in inputs {
        cmd.arg(input);
    }
    run_capture(&mut cmd, password, Some(cancel))?;

    progress.emit(ProgressEvent::Finished {
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
    Ok(())
}

/// Parse output `7z l -slt` menjadi daftar [`Entry`].
fn parse_7z_slt(stdout: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut started = false;
    let mut path: Option<String> = None;
    let mut size: u64 = 0;
    let mut packed: u64 = 0;
    let mut is_dir = false;

    let flush = |entries: &mut Vec<Entry>,
                 path: &mut Option<String>,
                 size: &mut u64,
                 packed: &mut u64,
                 is_dir: &mut bool| {
        if let Some(name) = path.take() {
            entries.push(Entry {
                name,
                size: *size,
                compressed_size: *packed,
                is_dir: *is_dir,
            });
        }
        *size = 0;
        *packed = 0;
        *is_dir = false;
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
            flush(&mut entries, &mut path, &mut size, &mut packed, &mut is_dir);
            continue;
        }
        if let Some((k, v)) = line.split_once(" = ") {
            match k {
                "Path" => path = Some(v.to_string()),
                "Size" => size = v.parse().unwrap_or(0),
                "Packed Size" => packed = v.parse().unwrap_or(0),
                "Folder" => is_dir = v == "+",
                "Attributes" => {
                    if v.starts_with('D') || v.contains("D_") {
                        is_dir = true;
                    }
                }
                _ => {}
            }
        }
    }
    flush(&mut entries, &mut path, &mut size, &mut packed, &mut is_dir);
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
        .map(|name| Entry {
            name: name.to_string(),
            size: 0,
            compressed_size: 0,
            is_dir: name.ends_with('/'),
        })
        .collect())
}

/// Extract seluruh isi RAR ke `dest`.
pub fn unrar_extract(
    archive: &Path,
    dest: &Path,
    password: Option<&str>,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<()> {
    let start = Instant::now();
    progress.emit(ProgressEvent::Started { total_files: 0 });

    let mut dest_arg = dest.as_os_str().to_os_string();
    dest_arg.push("/"); // unrar butuh trailing slash untuk direktori tujuan

    let mut cmd = hardened_command("unrar");
    cmd.arg("x").arg("-y").arg("--").arg(archive).arg(dest_arg);
    run_capture(&mut cmd, password, Some(cancel))?;

    progress.emit(ProgressEvent::Finished {
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
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
