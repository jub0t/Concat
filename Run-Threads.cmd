@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\run-threads.ps1"
if errorlevel 1 (
    echo.
    echo Concat could not start. See the message above.
    pause
)
