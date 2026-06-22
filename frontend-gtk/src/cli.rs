//! Parsing verb context-menu (`--extract-here`, `--add`, dll).
//!
//! Logika terpusat di CLI (Planning Doc §6.1): definisi menu tiap file manager
//! hanya memanggil `zippy <verb> <files>`, dan binary ini yang memutuskan
//! konteks. Verb didefinisikan di §6.1; implementasi penuh di v0.4 (Sprint 8-9).

use std::process::ExitCode;

/// Verb yang akan didukung (Planning Doc §6.1). Belum diimplementasikan.
pub const VERBS: &[&str] = &[
    "--open",
    "--extract-here",
    "--extract-to-subfolder",
    "--extract-to",
    "--extract-each",
    "--add",
    "--add-quick",
    "--test",
];

/// Coba tangani argumen sebagai verb CLI.
///
/// Mengembalikan `Some(exit_code)` bila argumen ditangani sebagai CLI (sehingga
/// caller harus keluar), atau `None` bila harus lanjut ke GUI.
pub fn try_dispatch<I>(args: I) -> Option<ExitCode>
where
    I: IntoIterator<Item = String>,
{
    let first = args.into_iter().next()?;

    match first.as_str() {
        "--help" | "-h" => {
            print_help();
            Some(ExitCode::SUCCESS)
        }
        "--version" | "-V" => {
            println!("zippy {}", zippy_core::VERSION);
            Some(ExitCode::SUCCESS)
        }
        v if VERBS.contains(&v) => {
            eprintln!("zippy: verb '{v}' belum diimplementasikan (dijadwalkan v0.4)");
            Some(ExitCode::FAILURE)
        }
        // Bukan verb (mis. path file dari MIME handler `zippy %F`) → lanjut GUI.
        _ => None,
    }
}

fn print_help() {
    println!("Zippy {} — archive manager untuk Linux", zippy_core::VERSION);
    println!();
    println!("PENGGUNAAN:");
    println!("  zippy                 Buka GUI");
    println!("  zippy <file>          Buka archive di GUI (MIME handler)");
    println!("  zippy <verb> <files>  Verb context-menu (lihat di bawah)");
    println!();
    println!("VERB (belum diimplementasikan — v0.4):");
    for v in VERBS {
        println!("  {v}");
    }
}
