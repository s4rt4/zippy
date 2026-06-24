# Fuzzing (cargo-fuzz)

Target fuzz untuk parser & guard keamanan (Planning Doc §10.3). Butuh toolchain
**nightly** + `cargo-fuzz`:

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
```

Jalankan dari direktori `core/`. Bila `rustup` dipasang user-local tanpa ubah
PATH (mesin dev ini), prefiks PATH satu-kali agar tak mengubah default Rust
sistem:

```sh
cd core
PATH="$HOME/.cargo/bin:$PATH" cargo fuzz run detect    -- -max_total_time=60  # deteksi magic bytes
PATH="$HOME/.cargo/bin:$PATH" cargo fuzz run safe_join -- -max_total_time=60  # guard Zip Slip
PATH="$HOME/.cargo/bin:$PATH" cargo fuzz run list      -- -max_total_time=60  # parser arsip korup
```

**Status (2026-06-24):** ketiga target dijalankan bersih, nol crash —
`detect` 32,5 jt eksekusi, `safe_join` 21,7 jt, `list` 15,3 rb (lebih lambat
karena I/O file per-iterasi). `detect` otomatis menemukan seluruh magic-byte
sebagai dictionary (bukti coverage menembus logika deteksi).

| Target | Yang diuji |
|--------|-----------|
| `detect`    | `formats::detect` tak panic atas byte sembarang |
| `safe_join` | `safety::safe_join` tak pernah meloloskan path di luar tujuan |
| `list`      | `archive::list` (deteksi + parser) tak panic/overflow pada arsip korup |

Crate ini **detached** dari workspace induk (punya `[workspace]` sendiri) agar
build rilis biasa tidak butuh nightly/libfuzzer.

## Tanpa nightly: robustness test (stable)

Bila nightly/`cargo-fuzz` tidak tersedia (mis. CI atau mesin dev tanpa rustup),
jalankan `core/tests/robustness.rs` — "poor man's fuzz" deterministik yang
menghantam KETIGA entry point yang sama (`detect`, `safe_join`, `list`) dengan
ribuan input acak ber-seed tetap + kasus tepi terkurasi, dan menegakkan invarian
yang sama (no-panic + Zip Slip):

```sh
cargo test -p zippy-core --test robustness
```

Ini bukan pengganti penuh libFuzzer (tak ada coverage-guided mutation), tapi
menjaga regresi no-panic/keamanan tetap terjaga di stable.
