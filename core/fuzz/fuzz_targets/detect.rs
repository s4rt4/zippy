//! Fuzz deteksi format dari magic bytes (Planning Doc §10.3).
//! Fungsi murni atas byte sembarang — tidak boleh panic / overflow.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = zippy_core::formats::detect(data);
});
