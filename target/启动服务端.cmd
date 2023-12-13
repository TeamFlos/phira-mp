set RUST_LOG=INFO

set PHIRA_PORT=
set /p PHIRA_PORT=Select the port (default 23333): 
echo.

start ./release/phira-mp-server.exe
