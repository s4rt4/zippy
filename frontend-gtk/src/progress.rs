//! Progress bar widget.
//!
//! GtkProgressBar di dalam revealer — muncul saat operasi berlangsung, hilang
//! saat selesai (Planning Doc §5.2). Menerima [`ProgressEvent`] dari worker
//! thread via channel, di-marshal ke UI thread lewat `glib` idle.
//!
//! Status: **Sprint 0 — stub**. Implementasi di v0.2/v0.3.

#[allow(unused_imports)]
use zippy_core::ProgressEvent;
