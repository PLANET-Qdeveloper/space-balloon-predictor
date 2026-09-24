@echo off
setlocal
cd /d "%~dp0.."

if exist bun.lock goto try_bun
if exist bun.lockb goto try_bun
goto after_bun
:try_bun
where bun >nul 2>&1
if %ERRORLEVEL% equ 0 (
  bun run %*
  exit /b %ERRORLEVEL%
)
:after_bun

if exist pnpm-lock.yaml (
  where pnpm >nul 2>&1
  if %ERRORLEVEL% equ 0 (
    pnpm run %*
    exit /b %ERRORLEVEL%
  )
)

if exist package-lock.json (
  where npm >nul 2>&1
  if %ERRORLEVEL% equ 0 (
    npm run %*
    exit /b %ERRORLEVEL%
  )
)

where bun >nul 2>&1
if %ERRORLEVEL% equ 0 (
  bun run %*
  exit /b %ERRORLEVEL%
)
where npm >nul 2>&1
if %ERRORLEVEL% equ 0 (
  npm run %*
  exit /b %ERRORLEVEL%
)
where pnpm >nul 2>&1
if %ERRORLEVEL% equ 0 (
  pnpm run %*
  exit /b %ERRORLEVEL%
)

echo [tauri-run] bun, npm, or pnpm is required to run: %*
exit /b 1
