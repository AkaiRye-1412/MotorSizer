@echo off
setlocal
rem Launch the built desktop app: prefer the release build, fall back to debug.
rem Keep this file ASCII-only: cmd.exe decodes .cmd files as ANSI/GBK.
set "APP=%~dp0"
set "EXE=%APP%target\release\motor-selection.exe"
if not exist "%EXE%" set "EXE=%APP%target\debug\motor-selection.exe"
if not exist "%EXE%" (
  echo The app is not built yet. Run: build.cmd build  or  build.cmd build --release
  pause
  exit /b 1
)
rem Runtime DLLs: MinGW-w64 bin plus Rust GNU rustlib bin.
set "MINGW=%LOCALAPPDATA%\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin"
set "RUSTLIB=%USERPROFILE%\.rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin"
if exist "%MINGW%"   set "PATH=%MINGW%;%PATH%"
if exist "%RUSTLIB%" set "PATH=%RUSTLIB%;%PATH%"
start "MotionSizer Studio" "%EXE%" %*
