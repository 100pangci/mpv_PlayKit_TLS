@echo off
rem Test mode: ignore the portable configuration and load only test.conf.
setlocal
set "ROOT=%~dp0..\.."
pushd "%ROOT%" >nul || exit /b 1
if not exist "%ROOT%\mpv.com" (
	echo mpv.com not found in "%ROOT%". 1>&2
	popd >nul
	exit /b 1
)
"%ROOT%\mpv.com" --config=no --include="%ROOT%\installer\modes\test.conf" %*
set "CODE=%ERRORLEVEL%"
popd >nul
exit /b %CODE%