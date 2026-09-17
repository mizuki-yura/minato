@echo off
rem Drag and drop a ghost home directory (the folder containing talks\main.mnt)
rem onto this file. Requires minato_check.exe in the same folder.
"%~dp0minato_check.exe" %*
set EXITCODE=%ERRORLEVEL%
echo.
pause
exit /b %EXITCODE%
