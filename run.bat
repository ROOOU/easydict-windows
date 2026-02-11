@echo off
chcp 65001 >nul 2>&1
title EasyDict Windows

echo.
echo  ╔══════════════════════════════════╗
echo  ║     EasyDict Windows Launcher    ║
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

echo [INFO] Starting EasyDict Windows...
echo [INFO] First build may take a few minutes (compiling Rust).
echo.

call npx tauri dev

pause
