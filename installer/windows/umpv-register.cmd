@echo off
rem Register umpv.exe as a per-user "Open with" handler (HKCU only).
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0umpv-register.ps1" %*
if errorlevel 1 pause