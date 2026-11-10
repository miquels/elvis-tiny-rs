#!/bin/sh
# Builds the smallest elvis-tiny binary the current toolchain allows.
#
# This rebuilds the standard library from source with panics that abort
# immediately, which drops std's panic printing and backtrace code.  It needs
# unstable cargo/rustc flags, enabled here on a stable toolchain through
# RUSTC_BOOTSTRAP=1, and the rust-src component (rustup component add
# rust-src; Homebrew's rust ships it).  The normal `cargo build --release`
# stays the supported build.
#
# Trade-off: a panic aborts without running the panic hook, so the terminal
# is not restored if the editor ever panics.
#
# Usage: scripts/build-tiny.sh [extra cargo args]
# The binary ends up in target/tiny/<host triple>/release/elvis-tiny-rs
# (or under $CARGO_TARGET_DIR if set).
set -eu

cd "$(dirname "$0")/.."
host=$(rustc -vV | sed -n 's/^host: //p')
dir=${CARGO_TARGET_DIR:-target/tiny}

RUSTC_BOOTSTRAP=1 \
RUSTFLAGS="${RUSTFLAGS:-} -Zunstable-options -Cpanic=immediate-abort" \
cargo build --release \
    -Zbuild-std=std,panic_abort \
    -Zpanic-immediate-abort \
    --config 'profile.release.panic="immediate-abort"' \
    --target "$host" \
    --target-dir "$dir" \
    "$@"

echo "$dir/$host/release/elvis-tiny-rs"
