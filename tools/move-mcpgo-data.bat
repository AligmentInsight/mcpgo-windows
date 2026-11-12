@echo off
setlocal EnableExtensions

rem  Moves the McpGo data directory to another drive and leaves a
rem  link (junction) behind, so the app keeps working unchanged.
rem  Usage:  move-mcpgo-data.bat [destination]
rem  Default destination: F:\McpGo

set "SRC=%APPDATA%\com.mcpgo.windows"
set "DST=%~1"
if "%DST%"=="" set "DST=F:\McpGo"
if "%DST:~-1%"=="\" set "DST=%DST:~0,-1%"

rem  Destination drive must exist
for /f "delims=" %%D in ("%DST%") do set "DSTDRIVE=%%~dD"
if not defined DSTDRIVE (
    echo [ERROR] "%DST%" is not a valid path.
    pause
    exit /b 1
)
if not exist "%DSTDRIVE%\" (
    echo [ERROR] Drive %DSTDRIVE% not found.
    echo Check that the drive is connected.
    pause
    exit /b 1
)

rem  Already moved? (the old path is already a link)
dir "%APPDATA%" /AL /B 2>nul | findstr /I /X "com.mcpgo.windows" >nul
if not errorlevel 1 (
    echo The data has already been moved - the link already exists.
    echo Nothing to do.
    pause
    exit /b 0
)

rem  Create the destination folder
if not exist "%DST%" mkdir "%DST%"

rem  Move the existing data
if exist "%SRC%" (
    echo Moving data to %DST% ...
    robocopy "%SRC%" "%DST%" /E /MOVE /NFL /NDL /NJH /NJS /NP >nul
    if exist "%SRC%" rmdir /S /Q "%SRC%"
    if exist "%SRC%" (
        echo [ERROR] Could not remove the old folder.
        echo Close the McpGo app and all its windows, then run this file again.
        pause
        exit /b 1
    )
)

rem  Create the junction (the app keeps using the old path)
echo Creating the link ...
mklink /J "%SRC%" "%DST%"
if errorlevel 1 (
    echo [ERROR] Could not create the link.
    echo Make a screenshot of this window and send it to me.
    pause
    exit /b 1
)

echo.
echo DONE! All McpGo data now lives in %DST%
echo You can start McpGo again.
echo.
pause
