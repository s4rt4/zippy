#!/usr/bin/env bash
# Build paket rilis Zippy: .deb (cargo-deb) + .rpm (cargo-generate-rpm).
# Metadata paket ada di frontend-gtk/Cargo.toml ([package.metadata.deb] &
# [package.metadata.generate-rpm]).
#
# Prasyarat (rustup user-local di ~/.cargo/bin):
#   cargo install cargo-deb cargo-generate-rpm
#
# Catatan: di Fedora, deps pustaka-bersama .deb ditulis eksplisit (dpkg-shlibdeps
# tak punya DB Debian); .rpm memakai find-requires native sehingga auto.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"
cd "$REPO"

for tool in cargo-deb cargo-generate-rpm; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "ERROR: '$tool' tidak ditemukan. Pasang dengan:" >&2
        echo "  cargo install cargo-deb cargo-generate-rpm" >&2
        exit 1
    fi
done

echo "==> Build release"
cargo build --release -p zippy

echo "==> .deb (cargo-deb)"
cargo deb -p zippy --no-build

echo "==> .rpm (cargo-generate-rpm)"
cargo generate-rpm -p frontend-gtk

echo
echo "Selesai. Artefak:"
ls -1 target/debian/*.deb target/generate-rpm/*.rpm
