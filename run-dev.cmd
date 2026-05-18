@echo off
setlocal

call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
set PINGINFO_BIND=0.0.0.0:18080
set PINGINFO_DB=%~dp0data\pinginfo.db

"C:\Users\charl\.cargo\bin\cargo.exe" run
