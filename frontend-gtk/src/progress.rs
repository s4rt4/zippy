//! Jembatan progress worker-thread → UI thread.
//!
//! Operasi berat (extract/compress) berjalan di `std::thread`. Worker memanggil
//! [`ProgressSink::emit`] dari thread itu; [`ChannelSink`] meneruskan tiap event
//! lewat `async-channel`. Sisi UI menerimanya di main loop via
//! `glib::spawn_future_local` dan menyentuh widget di sana (Planning Doc §2.3).
//!
//! glib 0.20 menghapus `MainContext::channel`, jadi kita pakai `async-channel`
//! (`send_blocking` dari worker, `recv().await` di UI) sebagai gantinya.

use async_channel::Sender;
use zippy_core::{ProgressEvent, ProgressSink};

/// [`ProgressSink`] yang mengirim tiap event ke channel async.
pub struct ChannelSink {
    tx: Sender<ProgressEvent>,
}

impl ChannelSink {
    pub fn new(tx: Sender<ProgressEvent>) -> Self {
        Self { tx }
    }
}

impl ProgressSink for ChannelSink {
    fn emit(&self, event: ProgressEvent) {
        // Blocking dari worker thread; abaikan bila receiver (UI) sudah ditutup
        // — mis. window keburu ditutup di tengah operasi.
        let _ = self.tx.send_blocking(event);
    }
}
