#requires -Version 5.1
<#
    mpv PlayKit - Windows uninstaller.

    Removes the shortcuts, HKCU registrations and files created by install.ps1.
    Original code, licensed GPL-3.0-or-later.

    GUI:     uninstall.cmd (or the Start Menu shortcut)
    Silent:  powershell -NoProfile -ExecutionPolicy Bypass -File uninstall.ps1 -Silent
#>
[CmdletBinding()]
param(
    [switch]$Silent,
    [switch]$KeepUserData
)

$ErrorActionPreference = 'Stop'
$script:ProgId = 'mpv-playkit'
$script:InstallDir = [IO.Path]::GetFullPath(
    (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
).TrimEnd('\')
$script:TargetExe = Join-Path $script:InstallDir 'mpv.exe'
$script:UninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\mpv-playkit'

function Write-Log {
    param([string]$Message)
    if (-not $Silent) { Write-Host $Message }
}

function Get-MediaExtensions {
    $list = New-Object System.Collections.Generic.List[string]
    $conf = Join-Path $script:InstallDir 'portable_config\script-opts\uosc.conf'
    if (Test-Path -LiteralPath $conf) {
        foreach ($line in Get-Content -LiteralPath $conf -Encoding UTF8) {
            if ($line -match '^\s*(video|audio|image|subtitle)_types\s*=\s*(.+)$') {
                foreach ($item in $matches[2].Split(',')) {
                    $ext = $item.Trim().TrimStart('.').ToLowerInvariant()
                    if ($ext -match '^[a-z0-9]{1,10}$' -and -not $list.Contains($ext)) {
                        $list.Add($ext)
                    }
                }
            }
        }
    }
    if ($list.Count -eq 0) {
        foreach ($ext in @('mkv', 'mp4', 'webm', 'avi', 'mov', 'flac', 'mp3', 'opus', 'wav')) {
            $list.Add($ext)
        }
    }
    return $list.ToArray()
}

function Get-RunningMpv {
    Get-Process -Name 'mpv' -ErrorAction SilentlyContinue | Where-Object {
        try {
            $_.Path -and ([IO.Path]::GetFullPath($_.Path) -ieq [IO.Path]::GetFullPath($script:TargetExe))
        } catch {
            $false
        }
    }
}

function Remove-OwnedAppPath {
    param([string]$Name, [string]$ExpectedPath)
    $key = "HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths\$Name"
    if (-not (Test-Path -LiteralPath $key)) { return }
    try {
        $current = (Get-ItemProperty -LiteralPath $key -Name '(default)' -ErrorAction Stop).'(default)'
        if ($current -and ([IO.Path]::GetFullPath($current) -ieq [IO.Path]::GetFullPath($ExpectedPath))) {
            Remove-Item -LiteralPath $key -Recurse -Force
        }
    } catch {
        Write-Log "跳过 App Paths\$Name：$($_.Exception.Message)"
    }
}

function Remove-Integration {
    param([string[]]$Extensions)

    $startMenu = Join-Path ([Environment]::GetFolderPath('Programs')) 'mpv PlayKit'
    if (Test-Path -LiteralPath $startMenu) {
        Remove-Item -LiteralPath $startMenu -Recurse -Force
    }
    $desktopShortcut = Join-Path ([Environment]::GetFolderPath('Desktop')) 'mpv PlayKit.lnk'
    if (Test-Path -LiteralPath $desktopShortcut) {
        Remove-Item -LiteralPath $desktopShortcut -Force
    }

    $keys = @(
        'HKCU:\Software\Classes\mpv-playkit',
        'HKCU:\Software\Classes\mpv-playkit.umpv',
        'HKCU:\Software\Classes\Applications\mpv.exe',
        'HKCU:\Software\Classes\Applications\umpv.exe',
        'HKCU:\Software\Classes\*\shell\mpv-playkit',
        'HKCU:\Software\Classes\Directory\shell\mpv-playkit',
        'HKCU:\Software\Classes\SystemFileAssociations\video\OpenWithList\mpv.exe',
        'HKCU:\Software\Classes\SystemFileAssociations\audio\OpenWithList\mpv.exe',
        'HKCU:\Software\Classes\SystemFileAssociations\video\OpenWithList\umpv.exe',
        'HKCU:\Software\Classes\SystemFileAssociations\audio\OpenWithList\umpv.exe',
        'HKCU:\Software\Clients\Media\mpv-playkit',
        'HKCU:\Software\Clients\Media\mpv-playkit-umpv',
        $script:UninstallKey
    )
    foreach ($key in $keys) {
        if (Test-Path -LiteralPath $key) {
            Remove-Item -LiteralPath $key -Recurse -Force
        }
    }

    $registeredApps = 'HKCU:\Software\RegisteredApplications'
    if (Test-Path -LiteralPath $registeredApps) {
        Remove-ItemProperty -Path $registeredApps -Name $script:ProgId -ErrorAction SilentlyContinue
        Remove-ItemProperty -Path $registeredApps -Name 'mpv-playkit.umpv' -ErrorAction SilentlyContinue
    }

    Remove-OwnedAppPath -Name 'mpv.exe' -ExpectedPath $script:TargetExe
    Remove-OwnedAppPath -Name 'mpv.com' -ExpectedPath (Join-Path $script:InstallDir 'mpv.com')
    Remove-OwnedAppPath -Name 'umpv.exe' -ExpectedPath (Join-Path $script:InstallDir 'umpv.exe')

    foreach ($ext in $Extensions) {
        foreach ($progId in @($script:ProgId, 'mpv-playkit.umpv')) {
            try {
                $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey(
                    "Software\Classes\.$ext\OpenWithProgids", $true)
                if ($key) {
                    try {
                        $key.DeleteValue($progId, $false)
                    } finally {
                        $key.Dispose()
                    }
                }
            } catch {
                # Protected keys are left untouched; the registration simply never
                # existed for them.
                Write-Log "跳过 .$ext：$($_.Exception.Message)"
            }
        }
        try {
            $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey(
                'Software\Classes\Applications\umpv.exe\SupportedTypes', $true)
            if ($key) {
                try {
                    $key.DeleteValue(".$ext", $false)
                } finally {
                    $key.Dispose()
                }
            }
        } catch { }
    }
}

function Remove-Files {
    param([bool]$KeepData)

    Set-Location -LiteralPath $env:TEMP
    if ($KeepData) {
        if (Test-Path -LiteralPath $script:InstallDir) {
            Get-ChildItem -LiteralPath $script:InstallDir -Force |
                Where-Object { $_.Name -ne 'portable_config' } |
                ForEach-Object { Remove-Item -LiteralPath $_.FullName -Recurse -Force }
        }
        return
    }

    try {
        Remove-Item -LiteralPath $script:InstallDir -Recurse -Force
    } catch {
        # The script itself lives inside the directory; if a handle is still
        # open, let a detached shell remove the rest after we exit.
        Start-Process -FilePath 'cmd.exe' -WindowStyle Hidden -ArgumentList @(
            '/c', "ping 127.0.0.1 -n 3 >nul & rmdir /s /q `"$script:InstallDir`""
        ) | Out-Null
    }
}

function Uninstall-PlayKit {
    param([bool]$KeepData)

    $running = @(Get-RunningMpv)
    if ($running.Count -gt 0) {
        Write-Log '正在关闭 mpv PlayKit...'
        $running | Stop-Process -Force
        Start-Sleep -Milliseconds 500
    }

    Write-Log '正在移除快捷方式与注册表项...'
    Remove-Integration -Extensions (Get-MediaExtensions)

    Write-Log '正在删除文件...'
    Remove-Files -KeepData $KeepData
    Write-Log '卸载完成。'
}

function Show-Gui {
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    [System.Windows.Forms.Application]::EnableVisualStyles()

    $form = New-Object System.Windows.Forms.Form
    $form.Text = '卸载 mpv PlayKit'
    $form.StartPosition = 'CenterScreen'
    $form.FormBorderStyle = [System.Windows.Forms.FormBorderStyle]::FixedDialog
    $form.MaximizeBox = $false
    $form.MinimizeBox = $false
    $form.ClientSize = New-Object System.Drawing.Size(520, 190)
    $form.Font = [System.Drawing.SystemFonts]::MessageBoxFont

    $label = New-Object System.Windows.Forms.Label
    $label.Text = "将卸载以下位置的 mpv PlayKit：`n$($script:InstallDir)"
    $label.Location = New-Object System.Drawing.Point(16, 16)
    $label.Size = New-Object System.Drawing.Size(488, 44)
    $form.Controls.Add($label)

    $keepBox = New-Object System.Windows.Forms.CheckBox
    $keepBox.Text = '保留 portable_config（配置、脚本选项与个人数据）'
    $keepBox.Location = New-Object System.Drawing.Point(16, 72)
    $keepBox.AutoSize = $true
    $form.Controls.Add($keepBox)

    $uninstall = New-Object System.Windows.Forms.Button
    $uninstall.Text = '卸载'
    $uninstall.Location = New-Object System.Drawing.Point(344, 136)
    $uninstall.Size = New-Object System.Drawing.Size(76, 32)
    $form.Controls.Add($uninstall)

    $cancel = New-Object System.Windows.Forms.Button
    $cancel.Text = '取消'
    $cancel.Location = New-Object System.Drawing.Point(428, 136)
    $cancel.Size = New-Object System.Drawing.Size(76, 32)
    $cancel.Add_Click({ $form.Close() })
    $form.Controls.Add($cancel)

    $uninstall.Add_Click({
        $answer = [System.Windows.Forms.MessageBox]::Show(
            $form, '确定要卸载 mpv PlayKit 吗？', 'mpv PlayKit', 'YesNo', 'Question')
        if ($answer -ne 'Yes') { return }
        $uninstall.Enabled = $false
        $form.Cursor = [System.Windows.Forms.Cursors]::WaitCursor
        $form.Refresh()
        try {
            Uninstall-PlayKit -KeepData $keepBox.Checked
            $form.Close()
            [System.Windows.Forms.MessageBox]::Show(
                'mpv PlayKit 已卸载。', 'mpv PlayKit', 'OK', 'Information') | Out-Null
        } catch {
            $uninstall.Enabled = $true
            $form.Cursor = [System.Windows.Forms.Cursors]::Default
            [System.Windows.Forms.MessageBox]::Show(
                $form, $_.Exception.Message, '卸载失败', 'OK', 'Error') | Out-Null
        }
    })

    $form.AcceptButton = $uninstall
    $form.CancelButton = $cancel
    [void]$form.ShowDialog()
}

try {
    if ($Silent) {
        Uninstall-PlayKit -KeepData ([bool]$KeepUserData)
        exit 0
    }
    Show-Gui
    exit 0
} catch {
    if ($Silent) {
        [Console]::Error.WriteLine($_.Exception.Message)
        exit 1
    }
    Add-Type -AssemblyName System.Windows.Forms
    [System.Windows.Forms.MessageBox]::Show($_.Exception.Message, '卸载失败', 'OK', 'Error') | Out-Null
    exit 1
}
