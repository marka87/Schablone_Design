@echo off
title SMT Stencil Craft
echo ========================================================
echo  SMT Stencil Craft - Schablonendesign & IPC-Pruefung
echo ========================================================
echo.
echo Starte lokalen Server...
start "" http://127.0.0.1:5055
python app.py
pause

