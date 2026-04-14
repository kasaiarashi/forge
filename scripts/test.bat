@echo off
REM Runs cargo test with the MSVC toolchain env set up (see build.bat).
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
cargo test --release %*
