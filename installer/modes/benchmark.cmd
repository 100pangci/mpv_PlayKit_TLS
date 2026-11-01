@echo off
rem Benchmark mode: ignore the portable configuration and load benchmark.conf.
rem Extra arguments are passed through; later --profile options override the
rem defaults selected inside benchmark.conf.
setlocal
set "ROOT=%~dp0..\.."
pushd "%ROOT%" >nul || exit /b 1
if not exist "%ROOT%\mpv.com" (
	echo mpv.com not found in "%ROOT%". 1>&2
	popd >nul
	exit /b 1
)
"%ROOT%\mpv.com" --config=no --include="%ROOT%\installer\modes\benchmark.conf" %*
set "CODE=%ERRORLEVEL%"
popd >nul
exit /b %CODE%