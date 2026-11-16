@echo off
setlocal
rem Build with system cargo + WinLibs MinGW-w64 (target x86_64-pc-windows-gnu).
rem Keep this file ASCII-only: cmd.exe decodes .cmd files as ANSI/GBK.
set "APP=%~dp0"
set "MINGW=%LOCALAPPDATA%\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin"
if exist "%MINGW%" set "PATH=%MINGW%;%PATH%"
cd /d "%APP%"
cargo %*
