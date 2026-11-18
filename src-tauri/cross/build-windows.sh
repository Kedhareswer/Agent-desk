#!/usr/bin/env bash
# Cross-compile a Windows preview build from Linux (needs mingw-w64, cmake
# and clang). The normal way to build is `npx tauri build` on Windows; this
# script exists so a test build can be made without a Windows machine.
set -euo pipefail
cd "$(dirname "$0")/.."
target=x86_64-pc-windows-gnu
shim="$PWD/cross/mingw-shim.h"
export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc-posix
export CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++-posix
export CFLAGS_x86_64_pc_windows_gnu="-include $shim"
export CXXFLAGS_x86_64_pc_windows_gnu="-include $shim"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc-posix

# whisper.cpp's MinGW build names its libraries "ggml.a" rather than
# "libggml.a", which the Rust linker step cannot find. Build once so the
# libraries exist, give them the expected names, then build again.
cargo build --release --target "$target" || true
libs="$PWD/target/$target/release/whisper-libs"
mkdir -p "$libs"
for lib in target/"$target"/release/build/whisper-rs-sys-*/out/lib/*.a; do
  name=$(basename "$lib")
  case "$name" in lib*) ;; *) name="lib$name" ;; esac
  cp -f "$lib" "$libs/$name"
done
RUSTFLAGS="-L native=$libs" cargo build --release --target "$target"

# Collect the program and the runtime libraries it needs.
out="$PWD/target/$target/release/package/Agent Desk"
rm -rf "${out:?}" && mkdir -p "$out"
cp "target/$target/release/agent-desk.exe" "target/$target/release/WebView2Loader.dll" "$out/"
for dll in libstdc++-6.dll libgcc_s_seh-1.dll libwinpthread-1.dll; do
  found=$(x86_64-w64-mingw32-gcc-posix -print-file-name="$dll")
  [ -f "$found" ] && cp "$found" "$out/"
done
echo "Packaged in: $out"
ls -la "$out"
