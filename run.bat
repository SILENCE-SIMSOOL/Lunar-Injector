@echo off
setlocal DisableDelayedExpansion
set "DIST=%~dp0dist"
if not exist "%DIST%\LunarInjector.exe" goto missing
if not exist "%DIST%\lunar_bridge.dll" goto missing

pushd "%DIST%"
if errorlevel 1 goto failed
start "" "%DIST%\LunarInjector.exe"
if errorlevel 1 goto start_failed
popd
exit /b 0

:start_failed
popd
echo ERROR: Failed to start LunarInjector.exe.
goto failed

:missing
echo ERROR: Build output is missing. Run build.bat first.
goto failed

:failed
if not defined CI pause
exit /b 1
