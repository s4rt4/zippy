//! Fuzz guard path-traversal (Zip Slip). Untuk input apa pun, hasil tidak boleh
//! keluar dari direktori tujuan, dan tidak boleh panic (Planning Doc §10.3-4).
#![no_main]

use std::path::Path;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(entry) = std::str::from_utf8(data) {
        let dest = Path::new("/tmp/zippy-fuzz-dest");
        if let Ok(joined) = zippy_core::safety::safe_join(dest, entry) {
            // Invarian keamanan: hasil yang diterima HARUS di bawah `dest`.
            assert!(joined.starts_with(dest), "safe_join lolos di luar dest: {joined:?}");
        }
    }
});
