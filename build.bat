@echo off
chcp 65001 >nul 2>&1
title EasyDict Windows - Build

echo.
echo  ╔══════════════════════════════════╗
echo  ║      EasyDict Windows Build      ║
echo  ╚══════════════════════════════════╝
echo.

:: Check Node.js
where node >nul 2>&1
if %errorlevel% neq 0 (
    echo [ERROR] Node.js is not installed.
    echo         Download: https://nodejs.org/
    pause
    exit /b 1
)

:: Check Rust
where rustc >nul 2>&1
if %errorlevel% neq 0 (
    echo [ERROR] Rust is not installed.
    echo         Download: https://rustup.rs/
    pause
    exit /b 1
)

:: Install npm dependencies if needed
if not exist "node_modules" (
    echo [INFO] Installing dependencies...
    call npm install
    if %errorlevel% neq 0 (
        echo [ERROR] npm install failed.
        pause
        exit /b 1
    )
    echo.
)

echo [INFO] Building EasyDict Windows...
echo [INFO] This may take several minutes.
echo.

call npx tauri build

if %errorlevel% equ 0 (
    echo.
    echo [SUCCESS] Build complete!
    echo [INFO] Output: src-tauri\target\release\bundle\
    explorer "src-tauri\target\release\bundle\nsis"
) else (
    echo.
    echo [ERROR] Build failed.
)

pause
