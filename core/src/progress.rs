//! Event progress dari worker thread → UI thread.
//!
//! Kompresi/ekstraksi adalah pekerjaan CPU-bound. Operasi berat berjalan di
//! worker thread (`std::thread`), dan progress dikirim ke UI lewat channel.
//! Di GTK, marshaling balik ke UI thread dilakukan via `glib::MainContext` /
//! `idle_add` agar widget hanya disentuh dari main loop (Planning Doc §2.3).

/// Event yang dikirim worker thread selama operasi berlangsung.
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    Started { total_files: usize },
    FileProcessed { name: String, index: usize },
    BytesDone { bytes: u64, total: u64 },
    Finished { elapsed_ms: u64 },
    Error { message: String },
}

/// Sink generik untuk event progress.
///
/// Worker thread memanggil `emit`. Implementasi konkret (mis. `mpsc::Sender`)
/// menyusul saat operasi nyata diimplementasikan di v0.1.
pub trait ProgressSink: Send {
    fn emit(&self, event: ProgressEvent);
}

/// Sink no-op untuk operasi tanpa pelaporan progress (mis. dari unit test).
pub struct NullSink;

impl ProgressSink for NullSink {
    fn emit(&self, _event: ProgressEvent) {}
}
