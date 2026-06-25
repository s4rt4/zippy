//! Konfigurasi persisten ringan (tanpa serde): satu file `key=value` di XDG
//! config dir + daftar favorit terpisah. Filosofi proyek = seringan mungkin,
//! jadi sengaja tidak menambah dependensi parser.

use std::fs;
use std::path::{Path, PathBuf};

use zippy_core::Level;

/// Skema warna libadwaita yang dipilih user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    /// Ikuti pengaturan sistem.
    Default,
    Light,
    Dark,
}

impl Scheme {
    pub fn as_str(self) -> &'static str {
        match self {
            Scheme::Default => "default",
            Scheme::Light => "light",
            Scheme::Dark => "dark",
        }
    }
    fn parse(s: &str) -> Scheme {
        match s {
            "light" => Scheme::Light,
            "dark" => Scheme::Dark,
            _ => Scheme::Default,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Tingkat kompresi default untuk dialog "Add".
    pub level: Level,
    pub scheme: Scheme,
    /// Tampilkan dialog konfirmasi sebelum menghapus entri.
    pub confirm_delete: bool,
    /// Ekstensi (lowercase, tanpa titik) yang dilewati saat extract — padanan
    /// "File types to exclude from extracting" WinRAR. Kosong = tidak memfilter.
    pub prohibited: Vec<String>,
    /// Pindahkan arsip ke Trash setelah extract sukses (WinRAR "Delete archive").
    pub delete_after_extract: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            level: Level::Normal,
            scheme: Scheme::Default,
            confirm_delete: true,
            prohibited: Vec::new(),
            delete_after_extract: false,
        }
    }
}

/// Pecah teks daftar ekstensi (dipisah spasi/koma) jadi Vec lowercase tanpa
/// titik/asterisk: `"*.desktop, sh"` → `["desktop", "sh"]`.
pub fn parse_prohibited(s: &str) -> Vec<String> {
    s.split([',', ' ', '\t', ';'])
        .map(|t| t.trim().trim_start_matches('*').trim_start_matches('.'))
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

fn level_str(l: Level) -> &'static str {
    match l {
        Level::Store => "store",
        Level::Fastest => "fastest",
        Level::Normal => "normal",
        Level::Best => "best",
    }
}

fn level_parse(s: &str) -> Level {
    match s {
        "store" => Level::Store,
        "fastest" => Level::Fastest,
        "best" => Level::Best,
        _ => Level::Normal,
    }
}

fn config_dir() -> PathBuf {
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
        if !x.is_empty() {
            return PathBuf::from(x).join("zippy");
        }
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".config").join("zippy")
}

fn config_file() -> PathBuf {
    config_dir().join("config.ini")
}

fn favorites_file() -> PathBuf {
    config_dir().join("favorites")
}

impl Config {
    pub fn load() -> Config {
        let mut c = Config::default();
        let Ok(txt) = fs::read_to_string(config_file()) else {
            return c;
        };
        for line in txt.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let v = v.trim();
                match k.trim() {
                    "compression_level" => c.level = level_parse(v),
                    "color_scheme" => c.scheme = Scheme::parse(v),
                    "confirm_delete" => c.confirm_delete = v != "false",
                    "prohibited" => c.prohibited = parse_prohibited(v),
                    "delete_after_extract" => c.delete_after_extract = v == "true",
                    _ => {}
                }
            }
        }
        c
    }

    pub fn save(&self) {
        let _ = fs::create_dir_all(config_dir());
        let body = format!(
            "compression_level={}\ncolor_scheme={}\nconfirm_delete={}\nprohibited={}\ndelete_after_extract={}\n",
            level_str(self.level),
            self.scheme.as_str(),
            self.confirm_delete,
            self.prohibited.join(" "),
            self.delete_after_extract,
        );
        let _ = fs::write(config_file(), body);
    }
}

// ---------------------------------------------------------------------------
// Favorit
// ---------------------------------------------------------------------------

pub fn favorites_load() -> Vec<PathBuf> {
    fs::read_to_string(favorites_file())
        .map(|t| {
            t.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

fn favorites_save(list: &[PathBuf]) {
    let _ = fs::create_dir_all(config_dir());
    let body: String = list.iter().map(|p| format!("{}\n", p.display())).collect();
    let _ = fs::write(favorites_file(), body);
}

/// Tambah `path` (idempoten). Mengembalikan daftar terbaru.
pub fn favorites_add(path: &Path) -> Vec<PathBuf> {
    let mut list = favorites_load();
    if !list.iter().any(|p| p == path) {
        list.push(path.to_path_buf());
        favorites_save(&list);
    }
    list
}

/// Buang `path`. Mengembalikan daftar terbaru.
pub fn favorites_remove(path: &Path) -> Vec<PathBuf> {
    let mut list = favorites_load();
    list.retain(|p| p != path);
    favorites_save(&list);
    list
}

pub fn favorites_contains(path: &Path) -> bool {
    favorites_load().iter().any(|p| p == path)
}
