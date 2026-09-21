#!/usr/bin/env bash
# Launch Lattice on NixOS.
#
# Why this exists: `cargo run -p app` produces a binary that links against
# libX11/libxkbcommon/vulkan-loader by soname, but NixOS keeps those outside the
# default loader path, so a bare launch dies with:
#   "opening library failed (libX11.so.6: cannot open shared object file)"
# This script points the loader at the Nix store copies and gives wgpu a
# software Vulkan ICD (lavapipe) so it works even without a GPU driver.
#
# Usage:  ./run.sh [--debug] [-- <args>]

set -euo pipefail
shopt -s nocaseglob
cd "$(dirname "$0")"

PROFILE=release
for arg in "$@"; do
    [ "$arg" = "--debug" ] && PROFILE=debug
done

glob_libdirs() {
    local out=""
    for pattern in "$@"; do
        for dir in /nix/store/${pattern}/lib /nix/store/${pattern}/lib64; do
            [ -d "$dir" ] && out="${out:+$out:}$dir"
        done
    done
    printf '%s' "$out"
}

NIX_LIBS="$(glob_libdirs \
    '*-libX11-*' '*-libXcursor-*' '*-libXrandr-*' '*-libXi-*' '*-libXext-*' \
    '*-libXrender-*' '*-libXinerama-*' '*-libXau-*' '*-libXdmcp-*' \
    '*-libXfixes-*' '*-libxcb-*' '*-xcb-util*' '*-libxkbcommon-*' \
    '*-vulkan-loader-*' '*-mesa-2*')"

export LD_LIBRARY_PATH="${NIX_LIBS}${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

# wgpu needs an ICD to talk to; lavapipe is Mesa's software rasteriser.
ICD="$(ls -d /nix/store/*-mesa-2*/share/vulkan/icd.d/lvp_icd.x86_64.json 2>/dev/null | head -1 || true)"
[ -n "$ICD" ] && export VK_ICD_FILENAMES="$ICD" VK_DRIVER_FILES="$ICD"
DRI="$(ls -d /nix/store/*-mesa-2*/lib/dri 2>/dev/null | head -1 || true)"
[ -n "$DRI" ] && export LIBGL_DRIVERS_PATH="$DRI"

if [ "$PROFILE" = release ]; then
    exec cargo run --release -p app "$@"
else
    exec cargo run -p app "$@"
fi
