//! Wrapper CLI untuk backend eksternal (7z, unrar).
//!
//! Aturan keras (Planning Doc §2.2, §10.4):
//! - Spawn child dengan `LC_ALL=C` agar parsing stdout tidak pecah oleh locale.
//! - Password dikirim via **stdin**, tidak pernah lewat argv (cegah bocor
//!   lewat `ps`/`/proc`).
//! - Cleanup output parsial saat operasi di-cancel.
//!
//! Status: **Sprint 0 — stub**. Implementasi di v0.1 (Sprint 1-3).

use std::process::Command;

/// Bangun `Command` dengan environment yang aman untuk parsing
/// (`LC_ALL=C`). Helper dipakai semua pemanggil subprocess.
pub fn hardened_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.env("LC_ALL", "C");
    cmd
}
