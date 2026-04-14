@echo off
REM Builds the forge workspace with the MSVC toolchain env set up.
REM Usage: scripts\build.bat [extra cargo args]
REM
REM This repo's VS 2022 install lives on D:, but vcvars64.bat delegates to
REM vswhere.exe (only present in the default Program Files install) to
REM locate the MSVC tool paths — and even when vswhere is on PATH, it
REM doesn't discover the D: install, so INCLUDE/LIB end up missing the
REM compiler's own headers and libs. We inject the MSVC 14.44 paths
REM directly after vcvars64 to work around that.
setlocal
set "MSVC=D:\Softwares\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.44.35207"
set "PATH=C:\Program Files (x86)\Microsoft Visual Studio\Installer;%PATH%"
call "D:\Softwares\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >nul
if errorlevel 1 (
    echo vcvars64.bat failed 1>&2
    exit /b 1
)
set "INCLUDE=%MSVC%\include;%INCLUDE%"
set "LIB=%MSVC%\lib\x64;%LIB%"
set "PATH=%MSVC%\bin\Hostx64\x64;%PATH%"
cargo build --release %*
