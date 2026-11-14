@echo off
setlocal
rem Launch the built desktop app with the bundled runtime DLLs on PATH.
set "APP=%~dp0"
set "EXE=%APP%target\debug\motor-selection.exe"
if not exist "%EXE%" (
  echo The app is not built yet. Run: build.cmd build
  pause
  exit /b 1
)
set "PATH=%APP%tools\rustup\toolchains\stable-x86_64-pc-windows-gnu\bin;%APP%tools\llvm-mingw-20261104-msvcrt-x86_64\bin;%PATH%"
start "MotionSizer Studio" "%EXE%" %*
