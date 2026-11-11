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
# On Linux two more flags apply:
# - -Cforce-unwind-tables=no drops .eh_frame (about 27K).  Nothing unwinds
#   with immediate-abort panics; debuggers and profilers lose stack traces.
# - -z pack-relative-relocs stores the PIE relocations as DT_RELR (about 7K
#   smaller).  The binary then needs glibc 2.36 or later, so the flag is only
#   used when the build host's glibc is that new.
#
# Usage: scripts/build-tiny.sh [extra cargo args]
# The binary ends up in target/tiny/<host triple>/release/elvis-tiny-rs
# (or under $CARGO_TARGET_DIR if set).
set -eu

cd "$(dirname "$0")/.."
host=$(rustc -vV | sed -n 's/^host: //p')
dir=${CARGO_TARGET_DIR:-target/tiny}

flags="-Zunstable-options -Cpanic=immediate-abort"
case $host in
*-linux-*)
    flags="$flags -Cforce-unwind-tables=no"
    # `getconf GNU_LIBC_VERSION` prints e.g. "glibc 2.39"; it fails on musl.
    glibc=$(getconf GNU_LIBC_VERSION 2>/dev/null | sed -n 's/^glibc //p')
    major=${glibc%%.*}
    minor=${glibc#*.}
    minor=${minor%%.*}
    if [ -n "$glibc" ] && { [ "$major" -gt 2 ] ||
        { [ "$major" -eq 2 ] && [ "$minor" -ge 36 ]; }; }; then
        flags="$flags -Clink-arg=-Wl,-z,pack-relative-relocs"
    fi
    ;;
esac

RUSTC_BOOTSTRAP=1 \
RUSTFLAGS="${RUSTFLAGS:-} $flags" \
cargo build --release \
    -Zbuild-std=std,panic_abort \
    -Zpanic-immediate-abort \
    --config 'profile.release.panic="immediate-abort"' \
    --target "$host" \
    --target-dir "$dir" \
    "$@"

echo "$dir/$host/release/elvis-tiny-rs"
