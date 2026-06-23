//! Fuzz parser arsip: tulis byte sembarang ke file lalu coba `list`. Parser
//! (deteksi + zip/tar native) tidak boleh panic / overflow / hang pada input
//! korup/jahat (Planning Doc §10.3).
#![no_main]

use std::io::Write;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // File sementara unik per-proses (libfuzzer single-process).
    let path = std::env::temp_dir().join(format!("zippy-fuzz-{}.bin", std::process::id()));
    if std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(data))
        .is_ok()
    {
        // Hasil diabaikan; yang diuji adalah ketiadaan panic.
        let _ = zippy_core::archive::list(&path, None);
    }
    let _ = std::fs::remove_file(&path);
});
