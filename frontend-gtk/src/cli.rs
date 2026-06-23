//! Verb context-menu CLI (Planning Doc §6.1).
//!
//! Biner yang sama melayani GUI dan verb command-line. Definisi menu tiap file
//! manager hanya memanggil `zippy <verb> <files>`; biner ini yang memutuskan
//! konteks dan menjalankan operasi lewat `zippy_core` (tanpa GUI untuk verb
//! batch). Lihat juga `data/zippy-nautilus.py` untuk integrasi Nautilus.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use zippy_core::{CancelToken, Error, ProgressEvent, ProgressSink};

/// Hasil dispatch argumen.
pub enum Dispatch {
    /// Argumen ditangani sebagai CLI; caller harus keluar dengan kode ini.
    Handled(ExitCode),
    /// Lanjut ke GUI, opsional membuka satu archive.
    Gui(Option<PathBuf>),
    /// Lanjut ke GUI dengan dialog buat-archive untuk berkas-berkas ini.
    GuiCompress(Vec<PathBuf>),
}

/// Putuskan apa yang harus dilakukan dari argumen (tanpa GUI untuk verb batch).
pub fn dispatch(args: &[String]) -> Dispatch {
    let Some(first) = args.first() else {
        return Dispatch::Gui(None);
    };
    let rest = &args[1..];

    match first.as_str() {
        "--help" | "-h" => {
            print_help();
            Dispatch::Handled(ExitCode::SUCCESS)
        }
        "--version" | "-V" => {
            println!("zippy {}", zippy_core::VERSION);
            Dispatch::Handled(ExitCode::SUCCESS)
        }
        // Buka di GUI (MIME handler / "Open with Zippy").
        "--open" => Dispatch::Gui(rest.first().map(PathBuf::from)),
        // "Extract To…": serahkan ke dialog GUI.
        "--extract-to" => Dispatch::Gui(rest.first().map(PathBuf::from)),

        "--extract-here" => Dispatch::Handled(verb_extract(rest, ExtractMode::Here)),
        "--extract-to-subfolder" => Dispatch::Handled(verb_extract(rest, ExtractMode::Subfolder)),
        "--test" => Dispatch::Handled(verb_test(rest)),
        // "Add to archive…" → dialog GUI. "Add to <nama>.zip" → quick headless.
        "--add" => Dispatch::GuiCompress(rest.iter().map(PathBuf::from).collect()),
        "--add-quick" => Dispatch::Handled(verb_add_quick(rest)),

        // Verb tak dikenal.
        v if v.starts_with('-') => {
            eprintln!("zippy: verb tidak dikenal: {v}");
            Dispatch::Handled(ExitCode::FAILURE)
        }
        // Path file polos (mis. dari MIME handler `zippy %f`) → buka di GUI.
        _ => Dispatch::Gui(Some(PathBuf::from(first))),
    }
}

// ---------------------------------------------------------------------------
// Verb extract
// ---------------------------------------------------------------------------

enum ExtractMode {
    /// Extract ke direktori induk archive (WinRAR "Extract Here").
    Here,
    /// Extract ke sub-folder bernama sesuai archive.
    Subfolder,
}

fn verb_extract(archives: &[String], mode: ExtractMode) -> ExitCode {
    if archives.is_empty() {
        eprintln!("zippy: tidak ada archive untuk di-extract");
        return ExitCode::FAILURE;
    }
    let cancel = CancelToken::new();
    let mut failed = false;

    for a in archives {
        let archive = Path::new(a);
        let parent = archive.parent().unwrap_or_else(|| Path::new("."));
        let dest = match mode {
            ExtractMode::Here => parent.to_path_buf(),
            ExtractMode::Subfolder => parent.join(strip_archive_ext(archive)),
        };
        eprintln!("Extract {} → {}", archive.display(), dest.display());
        match zippy_core::archive::extract_all(archive, &dest, None, &cancel, &CliSink) {
            Ok(()) => {}
            Err(Error::Password) => {
                eprintln!("  ! terenkripsi — buka via GUI: zippy {}", archive.display());
                failed = true;
            }
            Err(e) => {
                eprintln!("  ! gagal: {e}");
                failed = true;
            }
        }
    }
    exit(failed)
}

// ---------------------------------------------------------------------------
// Verb test
// ---------------------------------------------------------------------------

fn verb_test(archives: &[String]) -> ExitCode {
    if archives.is_empty() {
        eprintln!("zippy: tidak ada archive untuk diuji");
        return ExitCode::FAILURE;
    }
    let cancel = CancelToken::new();
    let mut failed = false;

    for a in archives {
        let archive = Path::new(a);
        eprint!("Test {} … ", archive.display());
        match zippy_core::archive::test(archive, None, &cancel, &CliSink) {
            Ok(()) => eprintln!("OK"),
            Err(e) => {
                eprintln!("GAGAL: {e}");
                failed = true;
            }
        }
    }
    exit(failed)
}

// ---------------------------------------------------------------------------
// Verb add (kompres cepat)
// ---------------------------------------------------------------------------

fn verb_add_quick(inputs: &[String]) -> ExitCode {
    if inputs.is_empty() {
        eprintln!("zippy: tidak ada berkas untuk dikompres");
        return ExitCode::FAILURE;
    }
    let paths: Vec<PathBuf> = inputs.iter().map(PathBuf::from).collect();
    let refs: Vec<&Path> = paths.iter().map(|p| p.as_path()).collect();

    // Nama archive (auto): satu input → "<nama>.zip"; banyak input →
    // "<folder-induk>.zip" (Planning Doc §6.2 konteks A/C).
    let parent = paths[0].parent().unwrap_or_else(|| Path::new("."));
    let dest = if paths.len() == 1 {
        // Buang ekstensi terakhir (mis. "photo.png" → "photo.zip"), bukan
        // menumpuknya jadi "photo.png.zip".
        let name = paths[0]
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "archive".to_string());
        parent.join(format!("{}.zip", strip_last_ext(&name)))
    } else {
        let folder = parent
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "archive".to_string());
        parent.join(format!("{folder}.zip"))
    };

    eprintln!("Kompres {} berkas → {}", paths.len(), dest.display());
    let cancel = CancelToken::new();
    match zippy_core::archive::compress(&refs, &dest, None, &cancel, &CliSink) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("  ! gagal: {e}");
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------------------
// util
// ---------------------------------------------------------------------------

/// Sink progress yang mencetak nama berkas ke stderr (verb CLI).
struct CliSink;

impl ProgressSink for CliSink {
    fn emit(&self, ev: ProgressEvent) {
        match ev {
            ProgressEvent::FileProcessed { name, .. } => eprintln!("  {name}"),
            ProgressEvent::Error { message } => eprintln!("  ! {message}"),
            _ => {}
        }
    }
}

/// Buang ekstensi archive (termasuk `.tar.gz` dua-lapis) untuk nama sub-folder.
fn strip_archive_ext(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "extracted".to_string());
    let lower = name.to_lowercase();
    for ext in [
        ".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst", ".tgz", ".tbz2", ".txz",
    ] {
        if lower.ends_with(ext) {
            return name[..name.len() - ext.len()].to_string();
        }
    }
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => name,
    }
}

/// Buang satu ekstensi terakhir dari nama (untuk penamaan archive). Nama tanpa
/// titik atau dotfile (".bashrc") dibiarkan apa adanya.
fn strip_last_ext(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    }
}

fn exit(failed: bool) -> ExitCode {
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn print_help() {
    println!("Zippy {} — archive manager untuk Linux", zippy_core::VERSION);
    println!();
    println!("PENGGUNAAN:");
    println!("  zippy                       Buka GUI");
    println!("  zippy <file>                Buka archive di GUI (MIME handler)");
    println!("  zippy <verb> <files…>       Verb context-menu");
    println!();
    println!("VERB:");
    println!("  --open <file>               Buka di GUI");
    println!("  --extract-here <arc…>       Extract ke folder archive");
    println!("  --extract-to-subfolder <arc…>  Extract ke sub-folder senama");
    println!("  --extract-to <arc>          Extract via dialog GUI");
    println!("  --test <arc…>               Uji integritas");
    println!("  --add <files…>              Buat archive via dialog GUI");
    println!("  --add-quick <files…>        Kompres cepat jadi .zip (auto-nama)");
}
