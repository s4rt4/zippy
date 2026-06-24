//! Robustness ("poor man's fuzz") di stable toolchain (Planning Doc §10.3).
//!
//! Target cargo-fuzz sebenarnya (`core/fuzz/`) butuh nightly + libFuzzer dan
//! tidak selalu tersedia di mesin dev/CI. Test ini menjalankan KETIGA entry
//! point yang sama (`formats::detect`, `safety::safe_join`, `archive::list`)
//! atas ribuan input acak deterministik + kasus tepi terkurasi, dan menjamin:
//! - tidak ada panic/overflow pada input korup/jahat;
//! - invarian keamanan Zip Slip: hasil `safe_join` yang diterima selalu di
//!   bawah direktori tujuan.
//!
//! Deterministik (LCG seed tetap) agar kegagalan bisa direproduksi.

use std::path::Path;

use zippy_core::{CancelToken, NullSink};

/// LCG sederhana (Numerical Recipes) — cukup untuk menyebar byte, tanpa crate rng.
struct Lcg(u64);
impl Lcg {
    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 32) as u32
    }
    fn byte(&mut self) -> u8 {
        (self.next_u32() & 0xff) as u8
    }
    fn range(&mut self, n: u32) -> u32 {
        self.next_u32() % n
    }
    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.byte()).collect()
    }
}

#[test]
fn detect_never_panics_on_garbage() {
    let mut rng = Lcg(0x1234_5678_9abc_def0);
    for _ in 0..20_000 {
        let len = rng.range(600) as usize; // termasuk >512 (ukuran head di detect)
        let buf = rng.bytes(len);
        // Tidak boleh panic/overflow.
        let _ = zippy_core::formats::detect(&buf);
    }
    // Kasus tepi: kosong, tepat di batas magic, dan magic palsu.
    for buf in [
        vec![],
        b"PK".to_vec(),
        b"PK\x03\x04".to_vec(),
        vec![0u8; 512],
        vec![0xffu8; 1024],
    ] {
        let _ = zippy_core::formats::detect(&buf);
    }
}

#[test]
fn safe_join_invariant_holds() {
    let dest = Path::new("/tmp/zippy-robustness-dest");

    // Kasus jahat terkurasi: apa pun hasilnya, bila Ok harus di bawah `dest`.
    let nasty = [
        "../etc/passwd",
        "a/../../b",
        "/abs/path",
        "....//....//x",
        "..\\..\\win",
        "foo/./bar",
        "foo//bar",
        "./../../x",
        "a/b/c/../../../../../../etc",
        "",
        ".",
        "..",
        "a/\u{0}/b",
        "très/long/ñame/with/ünïcode",
    ];
    for entry in nasty {
        if let Ok(joined) = zippy_core::safety::safe_join(dest, entry) {
            assert!(
                joined.starts_with(dest),
                "safe_join lolos di luar dest untuk {entry:?}: {joined:?}"
            );
        }
    }

    // Acak: string UTF-8 sembarang dari byte acak (yang valid UTF-8 saja diuji).
    let mut rng = Lcg(0x0bad_f00d_dead_beef);
    for _ in 0..20_000 {
        let len = rng.range(40) as usize;
        let raw = rng.bytes(len);
        if let Ok(entry) = std::str::from_utf8(&raw) {
            if let Ok(joined) = zippy_core::safety::safe_join(dest, entry) {
                assert!(
                    joined.starts_with(dest),
                    "safe_join lolos di luar dest untuk {entry:?}: {joined:?}"
                );
            }
        }
    }
}

#[test]
fn list_never_panics_on_garbage_and_truncations() {
    let tmp = tempfile::tempdir().unwrap();
    let scratch = tmp.path().join("scratch.bin");

    // (a) Byte acak penuh.
    let mut rng = Lcg(0xdead_cafe_babe_0001);
    for _ in 0..2_000 {
        let len = rng.range(2048) as usize;
        let buf = rng.bytes(len);
        std::fs::write(&scratch, &buf).unwrap();
        let _ = zippy_core::archive::list(&scratch, None);
    }

    // (b) Prefiks terpotong dari arsip valid — header parsial paling rawan.
    let src = tmp.path().join("src");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::write(src.join("a.txt"), b"halo dunia\n").unwrap();
    std::fs::write(src.join("sub/b.txt"), b"isi kedua\n").unwrap();
    let inputs = [src.join("a.txt"), src.join("sub")];
    let refs: Vec<&Path> = inputs.iter().map(|p| p.as_path()).collect();

    for ext in ["zip", "tar", "tar.gz", "tar.zst"] {
        let archive = tmp.path().join(format!("valid.{ext}"));
        zippy_core::archive::compress(&refs, &archive, None, &CancelToken::new(), &NullSink).unwrap();
        let full = std::fs::read(&archive).unwrap();

        // Potong di banyak panjang berbeda (langkah cukup halus di awal).
        let len = full.len();
        let step = (len / 64).max(1);
        let mut cut = 0;
        while cut <= len {
            std::fs::write(&scratch, &full[..cut]).unwrap();
            let _ = zippy_core::archive::list(&scratch, None);
            cut += step;
        }
    }
}
