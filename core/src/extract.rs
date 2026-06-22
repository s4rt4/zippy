//! Ekstraksi + guard path traversal.
//!
//! Logika extract per-format yang memanggil [`safety::safe_join`](crate::safety::safe_join)
//! untuk setiap entry sebelum menulis ke disk, dan
//! [`safety::DecompressionGuard`](crate::safety::DecompressionGuard) untuk
//! membatasi output (zip bomb).
//!
//! Status: **Sprint 0 — stub**. Implementasi di v0.1 (Sprint 1-3).
