@echo off
rem Remove the per-user umpv.exe associations created by umpv-register.cmd.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0umpv-unregister.ps1" %*
if errorlevel 1 pause