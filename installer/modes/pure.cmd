@echo off
rem Pure mode: start mpv without any configuration.
rem Useful to verify the bundled mpv build itself before blaming the config.
setlocal
set "ROOT=%~dp0..\.."
pushd "%ROOT%" >nul || exit /b 1
if not exist "%ROOT%\mpv.com" (
	echo mpv.com not found in "%ROOT%". 1>&2
	popd >nul
	exit /b 1
)
"%ROOT%\mpv.com" --config=no --log-file=mpv-pure.log --vo=gpu-next --idle=once --force-window=yes --keep-open=yes --hidpi-window-scale=no --autofit-smaller=500x500 %*
set "CODE=%ERRORLEVEL%"
popd >nul
exit /b %CODE%