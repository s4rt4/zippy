# Fuzzing (cargo-fuzz)

Target fuzz untuk parser & guard keamanan (Planning Doc §10.3). Butuh toolchain
**nightly** + `cargo-fuzz`:

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
```

Jalankan dari direktori `core/`:

```sh
cargo +nightly fuzz run detect       # deteksi magic bytes
cargo +nightly fuzz run safe_join    # guard Zip Slip (invarian: tak keluar dest)
cargo +nightly fuzz run list         # parser arsip (zip/tar) atas input korup
```

| Target | Yang diuji |
|--------|-----------|
| `detect`    | `formats::detect` tak panic atas byte sembarang |
| `safe_join` | `safety::safe_join` tak pernah meloloskan path di luar tujuan |
| `list`      | `archive::list` (deteksi + parser) tak panic/overflow pada arsip korup |

Crate ini **detached** dari workspace induk (punya `[workspace]` sendiri) agar
build rilis biasa tidak butuh nightly/libfuzzer.
