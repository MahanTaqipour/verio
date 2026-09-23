@echo off
setlocal
cd /d C:\dev\verio
set FAIL=0

rem --all-targets also lints test and bench code. That matters here specifically because clippy
rem only *checks* targets, it never launches them, so it can cover the `verio` bin that
rem `cargo test` cannot run (see the note below).
echo === cargo clippy --workspace --all-targets -- -D warnings ===
cargo clippy --workspace --all-targets -- -D warnings
if errorlevel 1 set FAIL=1

rem The `verio` bin target carries a requireAdministrator manifest (needed for global hotkeys),
rem so its test harness cannot be launched from a non-elevated shell: it dies with os error 740
rem and aborts the run before the later crates are reached. `cargo test --workspace` therefore
rem always reports failure on a healthy tree. Exclude it here and test its lib target separately.
echo === cargo test --workspace --exclude verio ===
cargo test --workspace --exclude verio
if errorlevel 1 set FAIL=1

echo === cargo test -p verio --lib ===
cargo test -p verio --lib
if errorlevel 1 set FAIL=1

if "%FAIL%"=="1" (
  echo.
  echo VERIFY FAILED
  exit /b 1
)
echo.
echo VERIFY OK
exit /b 0
