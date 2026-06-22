//! Error types untuk core.
//!
//! Core memakai enum error tersendiri agar konsumen (frontend/CLI) bisa
//! mencocokkan kasus tertentu (mis. password salah, path tidak aman).

use std::fmt;

/// Alias `Result` standar untuk seluruh core.
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Format archive tidak dikenali / tidak didukung.
    UnsupportedFormat,
    /// Entry archive mencoba keluar dari direktori tujuan (Zip Slip).
    UnsafePath(String),
    /// Rasio/ukuran dekompresi melebihi batas aman (indikasi zip bomb).
    DecompressionLimit,
    /// Password salah atau dibutuhkan tapi tidak diberikan.
    Password,
    /// Operasi dibatalkan oleh user.
    Cancelled,
    /// Kesalahan I/O.
    Io(std::io::Error),
    /// Lain-lain (sementara, untuk scaffold).
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnsupportedFormat => write!(f, "format archive tidak didukung"),
            Error::UnsafePath(p) => write!(f, "path tidak aman (Zip Slip): {p}"),
            Error::DecompressionLimit => write!(f, "batas dekompresi terlampaui (zip bomb?)"),
            Error::Password => write!(f, "password salah atau dibutuhkan"),
            Error::Cancelled => write!(f, "operasi dibatalkan"),
            Error::Io(e) => write!(f, "I/O: {e}"),
            Error::Other(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
