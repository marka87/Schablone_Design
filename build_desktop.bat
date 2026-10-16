@echo off
rem Baut die Desktop-App (stencilcraft-desktop.exe) aus den aktuellen Web-Quellen.
setlocal
set "ROOT=%~dp0"
set "PATH=%LOCALAPPDATA%\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin;%USERPROFILE%\.cargo\bin;%PATH%"

rem 1. Web-Oberflaeche (static/ + templates/) in die Desktop-UI uebernehmen
copy /Y "%ROOT%static\app.js" "%ROOT%stencilcraft-desktop\ui\app.js" >nul || goto :error
copy /Y "%ROOT%static\style.css" "%ROOT%stencilcraft-desktop\ui\style.css" >nul || goto :error
powershell -NoProfile -Command "(Get-Content -Raw -Encoding UTF8 '%ROOT%templates\index.html').Replace('href=\"/static/style.css\"','href=\"./style.css\"').Replace('<script src=\"/static/app.js\"></script>','<script src=\"./bridge.js\"></script>' + [char]10 + '    <script src=\"./app.js\"></script>' + [char]10 + '    <script src=\"./desktop.js\"></script>') | Set-Content -NoNewline -Encoding UTF8 '%ROOT%stencilcraft-desktop\ui\index.html'" || goto :error

rem 2. Tests der Engine und Release-Build (Assets eingebettet)
cd /d "%ROOT%stencilcraft-desktop\src-tauri" || goto :error
cargo test -p stencil_core || goto :error
cargo build --release --features tauri/custom-protocol || goto :error
echo.
echo Fertig: %ROOT%stencilcraft-desktop\src-tauri\target\release\stencilcraft-desktop.exe
exit /b 0

:error
echo Build fehlgeschlagen.
exit /b 1
