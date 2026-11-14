@echo off
setlocal
rem Run cargo with the bundled GNU toolchain and llvm-mingw on PATH.
set "APP=%~dp0"
set "PATH=%APP%tools\rustup\toolchains\stable-x86_64-pc-windows-gnu\bin;%APP%tools\llvm-mingw-20261104-msvcrt-x86_64\bin;%PATH%"
set "CARGO_HOME=%APP%tools\cargo"
set "RUSTUP_HOME=%APP%tools\rustup"
"%APP%tools\rustup\toolchains\stable-x86_64-pc-windows-gnu\bin\cargo.exe" %*
