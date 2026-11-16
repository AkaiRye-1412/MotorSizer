@echo off
setlocal
rem Build release and pack a portable zip under dist\.
rem Keep this file ASCII-only: cmd.exe decodes .cmd files as ANSI/GBK.
set "APP=%~dp0"
set "MINGW=%LOCALAPPDATA%\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin"
if exist "%MINGW%" set "PATH=%MINGW%;%PATH%"
cd /d "%APP%"

rem Read version from Cargo.toml so the folder name never drifts.
for /f "tokens=2 delims==" %%v in ('findstr /b /c:"version = " Cargo.toml') do set "VERRAW=%%v"
set "VER=%VERRAW: =%"
set VER=%VER:"=%

set "NAME=MotionSizer-Studio-%VER%-win64"
set "DISTROOT=%APP%dist"
set "DIST=%DISTROOT%\%NAME%"

echo [1/3] cargo build --release
cargo build --release
if errorlevel 1 goto :fail

echo [2/3] stage files
if exist "%DISTROOT%" rmdir /s /q "%DISTROOT%"
mkdir "%DIST%"
copy /y "target\release\motor-selection.exe" "%DIST%\" >nul
if exist "README.md" copy /y "README.md" "%DIST%\README.md" >nul
if exist "..\README.md" copy /y "..\README.md" "%DIST%\README.md" >nul
mkdir "%DIST%\docs"
copy /y "docs\MECHANICS.md" "%DIST%\docs\" >nul
copy /y "docs\mechanics.json" "%DIST%\docs\" >nul

echo [3/3] pack zip
powershell -NoProfile -ExecutionPolicy Bypass -Command "Compress-Archive -Path '%DIST%' -DestinationPath '%DISTROOT%\%NAME%.zip' -Force"
if errorlevel 1 goto :fail

echo.
echo Packed: %DISTROOT%\%NAME%.zip
exit /b 0

:fail
echo.
echo Packaging failed.
exit /b 1
