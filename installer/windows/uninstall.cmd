@echo off
setlocal
rem mpv PlayKit uninstaller launcher. Original code, GPL-3.0-or-later.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0uninstall.ps1" %*
if errorlevel 1 (
    echo.
    echo Uninstallation failed. See the message above.
    pause
)
endlocal
