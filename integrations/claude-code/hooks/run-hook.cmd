: << 'CMDBLOCK'
@echo off
rem Polyglot: cmd runs this block, bash skips it. Usage: run-hook.cmd session-start | tally <args...>
rem The no-Git-Bash (cmd.exe) path is UNVERIFIED; only Git Bash has been observed.
set "D=%~dp0"
set "D=%D:\=/%"
where bash >nul 2>nul && (bash "%D%session-start" %* & exit /b 0)
if exist "%ProgramFiles%\Git\bin\bash.exe" ("%ProgramFiles%\Git\bin\bash.exe" "%D%session-start" %* & exit /b 0)
if "%1"=="session-start" echo {"systemMessage": "Tally needs Git Bash (or bash on PATH) to start from Claude Code."}
exit /b 0
CMDBLOCK
exec bash "$(dirname "$0")/session-start" "$@"
