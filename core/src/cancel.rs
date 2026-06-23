//! Token pembatalan operasi (Cancel) — Planning Doc §4.1, §10.4.
//!
//! Operasi berat (extract/compress) berjalan di worker thread; UI memegang
//! clone dari [`CancelToken`] yang sama dan memanggil [`CancelToken::cancel`]
//! saat user menekan tombol Cancel. Backend memeriksa token di batas tiap entry
//! dan di dalam loop salin byte, lalu mengembalikan [`Error::Cancelled`] +
//! membersihkan output parsial.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::error::{Error, Result};

/// Bendera pembatalan yang bisa dibagikan antar-thread (cheap clone via `Arc`).
///
/// `Default` menghasilkan token yang tidak pernah dibatalkan — dipakai pemanggil
/// yang tak butuh Cancel (mis. unit test).
#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// Token baru (belum dibatalkan).
    pub fn new() -> Self {
        Self::default()
    }

    /// Tandai dibatalkan. Idempoten; aman dipanggil dari thread mana pun.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Apakah sudah dibatalkan.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// Kembalikan [`Error::Cancelled`] bila sudah dibatalkan, selain itu `Ok`.
    /// Helper untuk menjaga loop backend tetap ringkas.
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}
