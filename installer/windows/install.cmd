@echo off
setlocal
rem mpv PlayKit installer launcher. Original code, GPL-3.0-or-later.
rem Run with -Silent for an unattended per-user installation, e.g.:
rem   install.cmd -Silent -InstallDir "D:\Apps\mpv-playkit"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" %*
if errorlevel 1 (
    echo.
    echo Installation failed. See the message above.
    pause
)
endlocal
