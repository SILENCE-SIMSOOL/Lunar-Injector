@echo off
setlocal DisableDelayedExpansion
pushd "%~dp0"
if errorlevel 1 goto failed

where cargo >nul 2>&1
if errorlevel 1 goto missing_rust
where rustup >nul 2>&1
if errorlevel 1 goto missing_rust

rustup target list --installed | findstr /X /C:"x86_64-pc-windows-msvc" >nul
if errorlevel 1 (
	echo Installing the Windows x64 MSVC target...
	call rustup target add x86_64-pc-windows-msvc
	if errorlevel 1 goto failed
)

echo [1/3] Testing shared settings...
call cargo test -p lunar-shared --target x86_64-pc-windows-msvc
if errorlevel 1 goto failed

echo [2/3] Building Windows x64 release...
call cargo build --workspace --release --target x86_64-pc-windows-msvc
if errorlevel 1 goto failed

set "RELEASE=target\x86_64-pc-windows-msvc\release"
if not exist "%RELEASE%\LunarInjector.exe" goto missing_output
if not exist "%RELEASE%\lunar_bridge.dll" goto missing_output

if not exist "dist" mkdir "dist"
if errorlevel 1 goto failed

echo [3/3] Copying executable and bridge DLL...
copy /Y "%RELEASE%\LunarInjector.exe" "dist\LunarInjector.exe" >nul
if errorlevel 1 goto failed
copy /Y "%RELEASE%\lunar_bridge.dll" "dist\lunar_bridge.dll" >nul
if errorlevel 1 goto failed

echo.
echo Build succeeded. Launch with run.bat.
popd
exit /b 0

:missing_rust
echo ERROR: Rust and rustup are required. Install the MSVC Rust toolchain.
goto failed

:missing_output
echo ERROR: Build reported success, but the EXE or DLL is missing.
goto failed

:failed
echo.
echo BUILD FAILED. Check the messages above.
echo Make sure Visual Studio C++ Build Tools and Windows SDK are installed.
popd
if not defined CI pause
exit /b 1
