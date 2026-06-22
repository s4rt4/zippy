//! Wrapper CLI untuk backend eksternal (7z, unrar).
//!
//! Aturan keras (Planning Doc §2.2, §10.4):
//! - Spawn child dengan `LC_ALL=C` agar parsing stdout tidak pecah oleh locale.
//! - Password dikirim via **stdin**, tidak pernah lewat argv.
//! - Cleanup output parsial saat operasi di-cancel.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::archive::Entry;
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
    )?;
    Ok(parse_7z_slt(&out))
}

/// Extract seluruh isi archive 7z ke `dest`.
pub fn sevenzip_extract(
    archive: &Path,
    dest: &Path,
    password: Option<&str>,
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
    run_capture(&mut cmd, password)?;

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
    run_capture(&mut cmd, password)?;

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
    let out = run_capture(hardened_command("unrar").arg("lb").arg("--").arg(archive), None)?;
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
    progress: &dyn ProgressSink,
) -> Result<()> {
    let start = Instant::now();
    progress.emit(ProgressEvent::Started { total_files: 0 });

    let mut dest_arg = dest.as_os_str().to_os_string();
    dest_arg.push("/"); // unrar butuh trailing slash untuk direktori tujuan

    let mut cmd = hardened_command("unrar");
    cmd.arg("x").arg("-y").arg("--").arg(archive).arg(dest_arg);
    run_capture(&mut cmd, password)?;

    progress.emit(ProgressEvent::Finished {
        elapsed_ms: start.elapsed().as_millis() as u64,
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// util
// ---------------------------------------------------------------------------

/// Jalankan command, kirim `password` (bila ada) via stdin, kumpulkan stdout.
fn run_capture(cmd: &mut Command, password: Option<&str>) -> Result<String> {
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
    if let Some(stdin) = child.stdin.take() {
        let mut stdin = stdin;
        if let Some(pw) = password {
            let _ = writeln!(stdin, "{pw}");
            let _ = writeln!(stdin, "{pw}");
        }
        // stdin di-drop di sini → EOF.
    }

    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}{}", String::from_utf8_lossy(&output.stdout), stderr);
        if combined.to_lowercase().contains("password")
            || combined.to_lowercase().contains("wrong password")
            || combined.contains("CRC failed")
        {
            return Err(Error::Password);
        }
        return Err(Error::Other(format!(
            "backend keluar dengan status {}: {}",
            output.status,
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
