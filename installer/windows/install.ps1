#requires -Version 5.1
<#
    mpv PlayKit - Windows installer.

    Installs a copy of the portable package into a per-user directory, creates
    Start Menu/desktop shortcuts and registers "Open with" entries and context
    menu verbs under HKCU only (no administrator rights, no system-wide
    changes).  Original code, licensed GPL-3.0-or-later.

    GUI:     install.cmd
    Silent:  powershell -NoProfile -ExecutionPolicy Bypass -File install.ps1 -Silent
#>
[CmdletBinding()]
param(
    [string]$InstallDir = '',
    [switch]$Silent,
    [switch]$NoDesktopShortcut,
    [switch]$NoStartMenuShortcut,
    [switch]$NoContextMenu,
    [switch]$NoFileAssociations,
    [switch]$Launch
)

$ErrorActionPreference = 'Stop'
$script:ProgId = 'mpv-playkit'
$script:UninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\mpv-playkit'

$script:PackageRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $InstallDir) {
    $InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\mpv-playkit'
}
$InstallDir = [IO.Path]::GetFullPath($InstallDir).TrimEnd('\')
$script:TargetExe = Join-Path $InstallDir 'mpv.exe'

function Write-Log {
    param([string]$Message)
    if (-not $Silent) { Write-Host $Message }
}

function Get-PackageVersion {
    $meta = Join-Path $script:PackageRoot 'playkit-version.json'
    if (Test-Path -LiteralPath $meta) {
        try {
            $info = Get-Content -LiteralPath $meta -Raw -Encoding UTF8 | ConvertFrom-Json
            if ($info.playkit_version) { return [string]$info.playkit_version }
        } catch { }
    }
    return 'unknown'
}

function Get-RepositoryUrl {
    $meta = Join-Path $script:PackageRoot 'manifest.json'
    if (Test-Path -LiteralPath $meta) {
        try {
            $info = Get-Content -LiteralPath $meta -Raw -Encoding UTF8 | ConvertFrom-Json
            if ($info.repository) { return [string]$info.repository }
        } catch { }
    }
    return ''
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

function Invoke-Updater {
    param([string]$Root, [string[]]$Arguments)
    $updater = Join-Path $Root 'updater\playkit-updater.exe'
    if (-not (Test-Path -LiteralPath $updater)) {
        return $false
    }
    & $updater @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "更新器执行失败（退出码 $LASTEXITCODE）：$($Arguments -join ' ')"
    }
    return $true
}

function Copy-Package {
    param([string]$Source, [string]$Target)
    $sourceFull = [IO.Path]::GetFullPath($Source).TrimEnd('\')
    $targetFull = [IO.Path]::GetFullPath($Target).TrimEnd('\')
    New-Item -ItemType Directory -Force -Path $targetFull | Out-Null
    if ($sourceFull -ieq $targetFull) { return }

    # Capture direct edits before the old official configuration is replaced.
    # The new package is then copied over the installation, preserving user/
    # (and runtime state) and re-applying the captured patches afterwards.
    if (Test-Path -LiteralPath (Join-Path $targetFull 'playkit-version.json')) {
        $oldVersion = 'unknown'
        try {
            $oldMeta = Get-Content -Raw -LiteralPath (Join-Path $targetFull 'playkit-version.json') | ConvertFrom-Json
            if ($oldMeta.playkit_version) {
                $oldVersion = ([string]$oldMeta.playkit_version) -replace '[^0-9A-Za-z._-]', '_'
            }
        } catch {
            Write-Log '无法读取旧版本号，备份目录将标记为 unknown。'
        }
        Write-Log '正在保存旧版本中的用户配置修改...'
        try {
            $captured = Invoke-Updater -Root $targetFull -Arguments @('capture', '--root', $targetFull, '--quiet')
            if (-not $captured) {
                # A pre-updater install has no helper to capture direct edits.
                # Keep the old generated configs before the new package replaces
                # them; the updater can only migrate files it can still read.
                $backupRoot = Join-Path $targetFull "portable_config\user\backup\pre-update-$oldVersion"
                foreach ($relative in @('mpv.conf', 'input.conf', 'profiles.conf', 'platform.conf')) {
                    $source = Join-Path $targetFull "portable_config\$relative"
                    if (Test-Path -LiteralPath $source) {
                        $destination = Join-Path $backupRoot $relative
                        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destination) | Out-Null
                        Copy-Item -LiteralPath $source -Destination $destination -Force
                    }
                }
                $scriptOpts = Join-Path $targetFull 'portable_config\script-opts'
                if (Test-Path -LiteralPath $scriptOpts) {
                    Get-ChildItem -LiteralPath $scriptOpts -Filter '*.conf' -File | ForEach-Object {
                        $destination = Join-Path $backupRoot "script-opts\$($_.Name)"
                        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destination) | Out-Null
                        Copy-Item -LiteralPath $_.FullName -Destination $destination -Force
                    }
                }
                Write-Log "旧版本没有更新器，已把原配置备份到 portable_config\user\backup\pre-update-$oldVersion。"
            }
        } catch {
            throw "无法保存旧版本的用户配置，安装已停止：$($_.Exception.Message)"
        }
    }

    Get-ChildItem -LiteralPath $sourceFull -Force | ForEach-Object {
        Copy-Item -LiteralPath $_.FullName -Destination $targetFull -Recurse -Force
    }
    # Runtime status belongs to this installation, not to the package.  Keep
    # saved properties, watch-later data and update-check state; only remove
    # files that would show a stale update result.
    foreach ($relative in @(
        'portable_config\.playkit\update-status.json',
        'portable_config\.playkit\updater.log'
    )) {
        $path = Join-Path $targetFull $relative
        if (Test-Path -LiteralPath $path) {
            Remove-Item -LiteralPath $path -Force
        }
    }
}

function New-Shortcut {
    param(
        [string]$Path,
        [string]$TargetPath,
        [string]$Arguments = '',
        [string]$WorkingDirectory = '',
        [string]$IconLocation = '',
        [string]$Description = ''
    )
    $shell = New-Object -ComObject WScript.Shell
    try {
        $shortcut = $shell.CreateShortcut($Path)
        $shortcut.TargetPath = $TargetPath
        if ($Arguments) { $shortcut.Arguments = $Arguments }
        if ($WorkingDirectory) { $shortcut.WorkingDirectory = $WorkingDirectory }
        if ($IconLocation) { $shortcut.IconLocation = $IconLocation }
        if ($Description) { $shortcut.Description = $Description }
        $shortcut.Save()
    } finally {
        [void][Runtime.InteropServices.Marshal]::ReleaseComObject($shell)
    }
}

function Set-RegNone {
    param([string]$KeyPath, [string]$Name)
    $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($KeyPath)
    try {
        $key.SetValue($Name, [byte[]]::new(0), [Microsoft.Win32.RegistryValueKind]::None)
    } finally {
        $key.Dispose()
    }
}

function Get-MediaExtensions {
    $list = New-Object System.Collections.Generic.List[string]
    $conf = Join-Path $script:PackageRoot 'portable_config\script-opts\uosc.conf'
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

function Register-Integration {
    param(
        [string]$Target,
        [string]$ExePath,
        [string[]]$Extensions,
        [bool]$ContextMenu,
        [bool]$FileAssociations,
        [string]$Version,
        [string]$Repository
    )

    $classes = 'HKCU:\Software\Classes'
    $icon = Join-Path $Target 'installer\windows\mpv-playkit.ico'

    $progRoot = Join-Path $classes $script:ProgId
    New-Item -Path "$progRoot\DefaultIcon" -Force | Out-Null
    New-Item -Path "$progRoot\shell\open\command" -Force | Out-Null
    New-Item -Path "$progRoot\shell\play\command" -Force | Out-Null
    Set-ItemProperty -Path $progRoot -Name '(default)' -Value 'mpv PlayKit 媒体文件'
    Set-ItemProperty -Path $progRoot -Name 'FriendlyTypeName' -Value 'mpv PlayKit 媒体文件'
    New-ItemProperty -Path $progRoot -Name 'EditFlags' -PropertyType DWord -Value 65536 -Force | Out-Null
    Set-ItemProperty -Path "$progRoot\DefaultIcon" -Name '(default)' -Value "$icon,0"
    # Match mpv's own registration: the default verb is "play"; the duplicate
    # "open" verb is hidden from the context menu.
    Set-ItemProperty -Path "$progRoot\shell" -Name '(default)' -Value 'play'
    Set-ItemProperty -Path "$progRoot\shell\open" -Name 'LegacyDisable' -Value ''
    Set-ItemProperty -Path "$progRoot\shell\open\command" -Name '(default)' -Value "`"$ExePath`" `"%1`""
    Set-ItemProperty -Path "$progRoot\shell\play" -Name '(default)' -Value '&Play'
    Set-ItemProperty -Path "$progRoot\shell\play\command" -Name '(default)' -Value "`"$ExePath`" `"%1`""

    $appRoot = Join-Path $classes 'Applications\mpv.exe'
    New-Item -Path "$appRoot\DefaultIcon" -Force | Out-Null
    New-Item -Path "$appRoot\shell\open\command" -Force | Out-Null
    New-Item -Path "$appRoot\SupportedTypes" -Force | Out-Null
    Set-ItemProperty -Path $appRoot -Name '(default)' -Value 'mpv PlayKit'
    Set-ItemProperty -Path $appRoot -Name 'FriendlyAppName' -Value 'mpv PlayKit'
    Set-ItemProperty -Path $appRoot -Name 'ApplicationCompany' -Value 'mpv PlayKit TLS contributors'
    Set-ItemProperty -Path "$appRoot\DefaultIcon" -Name '(default)' -Value "$icon,0"
    Set-ItemProperty -Path "$appRoot\shell\open\command" -Name '(default)' -Value "`"$ExePath`" `"%1`""

    # Per-user "App Paths": lets ShellExecute/Run find mpv.exe and mpv.com.
    $appPaths = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths'
    $appPathEntries = @(
        @{ Name = 'mpv.exe'; Path = $ExePath },
        @{ Name = 'mpv.com'; Path = (Join-Path $Target 'mpv.com') }
    )
    foreach ($entry in $appPathEntries) {
        $key = Join-Path $appPaths $entry.Name
        New-Item -Path $key -Force | Out-Null
        Set-ItemProperty -Path $key -Name '(default)' -Value $entry.Path
        Set-ItemProperty -Path $key -Name 'Path' -Value $Target
    }
    New-ItemProperty -Path (Join-Path $appPaths 'mpv.exe') -Name 'UseUrl' -PropertyType DWord -Value 1 -Force | Out-Null

    if ($FileAssociations) {
        # "Default Programs" entry: Windows Settings > Default Apps can list
        # mpv PlayKit and let the user choose it for the registered types.
        $capabilities = 'HKCU:\Software\Clients\Media\mpv-playkit\Capabilities'
        New-Item -Path "$capabilities\FileAssociations" -Force | Out-Null
        Set-ItemProperty -Path $capabilities -Name 'ApplicationName' -Value 'mpv PlayKit'
        Set-ItemProperty -Path $capabilities -Name 'ApplicationDescription' -Value 'mpv PlayKit media player'
        New-Item -Path 'HKCU:\Software\RegisteredApplications' -Force | Out-Null
        Set-ItemProperty -Path 'HKCU:\Software\RegisteredApplications' -Name $script:ProgId `
            -Value 'Software\Clients\Media\mpv-playkit\Capabilities'

        # Match the original registration: offer mpv PlayKit in the "Open
        # with" list for every video/audio file, not only the extensions
        # enumerated below.
        foreach ($perceived in @('video', 'audio')) {
            New-Item -Path "HKCU:\Software\Classes\SystemFileAssociations\$perceived\OpenWithList\mpv.exe" -Force | Out-Null
        }

        foreach ($ext in $Extensions) {
            # SupportedTypes holds value names (".mkv" -> empty), not subkeys;
            # Windows reads it to decide which files the application opens.
            try {
                Set-RegNone -KeyPath 'Software\Classes\Applications\mpv.exe\SupportedTypes' -Name ".$ext"
            } catch {
                Write-Log "跳过 .$ext 的 SupportedTypes 注册：$($_.Exception.Message)"
            }
            Set-ItemProperty -Path "$capabilities\FileAssociations" -Name ".$ext" -Value $script:ProgId
            try {
                New-Item -Path (Join-Path $classes ".$ext\OpenWithProgids") -Force -ErrorAction Stop | Out-Null
                Set-RegNone -KeyPath "Software\Classes\.$ext\OpenWithProgids" -Name $script:ProgId
            } catch {
                # Some file types have protected OpenWithProgids keys (for
                # example when another player owns them).  Registering the
                # remaining types must still succeed.
                Write-Log "跳过 .$ext 的“打开方式”注册：$($_.Exception.Message)"
            }
        }
    }

    if ($ContextMenu) {
        $fileVerb = Join-Path $classes '*\shell\mpv-playkit'
        New-Item -Path "$fileVerb\command" -Force | Out-Null
        Set-ItemProperty -Path $fileVerb -Name '(default)' -Value '用 mpv PlayKit 播放'
        Set-ItemProperty -Path $fileVerb -Name 'Icon' -Value "$icon,0"
        Set-ItemProperty -Path $fileVerb -Name 'MultiSelectModel' -Value 'Player'
        Set-ItemProperty -Path "$fileVerb\command" -Name '(default)' -Value "`"$ExePath`" `"%1`""

        $dirVerb = Join-Path $classes 'Directory\shell\mpv-playkit'
        New-Item -Path "$dirVerb\command" -Force | Out-Null
        Set-ItemProperty -Path $dirVerb -Name '(default)' -Value '用 mpv PlayKit 播放文件夹'
        Set-ItemProperty -Path $dirVerb -Name 'Icon' -Value "$icon,0"
        Set-ItemProperty -Path "$dirVerb\command" -Name '(default)' -Value "`"$ExePath`" `"%1`""
    }

    New-Item -Path $script:UninstallKey -Force | Out-Null
    Set-ItemProperty -Path $script:UninstallKey -Name 'DisplayName' -Value 'mpv PlayKit'
    Set-ItemProperty -Path $script:UninstallKey -Name 'DisplayVersion' -Value $Version
    Set-ItemProperty -Path $script:UninstallKey -Name 'DisplayIcon' -Value "$icon,0"
    Set-ItemProperty -Path $script:UninstallKey -Name 'Publisher' -Value 'mpv PlayKit TLS contributors'
    Set-ItemProperty -Path $script:UninstallKey -Name 'InstallLocation' -Value $Target
    $uninstallScript = Join-Path $Target 'installer\windows\uninstall.ps1'
    $uninstallCommand = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$uninstallScript`""
    Set-ItemProperty -Path $script:UninstallKey -Name 'UninstallString' -Value $uninstallCommand
    Set-ItemProperty -Path $script:UninstallKey -Name 'QuietUninstallString' -Value "$uninstallCommand -Silent"
    if ($Repository) {
        Set-ItemProperty -Path $script:UninstallKey -Name 'URLInfoAbout' -Value $Repository
    }
    New-ItemProperty -Path $script:UninstallKey -Name 'NoModify' -PropertyType DWord -Value 1 -Force | Out-Null
    New-ItemProperty -Path $script:UninstallKey -Name 'NoRepair' -PropertyType DWord -Value 1 -Force | Out-Null
}

function Install-PlayKit {
    param(
        [string]$Source,
        [string]$Target,
        [bool]$DesktopShortcut,
        [bool]$StartMenuShortcut,
        [bool]$ContextMenu,
        [bool]$FileAssociations
    )

    if (-not (Test-Path -LiteralPath (Join-Path $Source 'mpv.exe'))) {
        throw "安装包不完整：找不到 mpv.exe（$Source）"
    }
    if (-not (Test-Path -LiteralPath (Join-Path $Source 'portable_config'))) {
        throw "安装包不完整：找不到 portable_config（$Source）"
    }

    Write-Log '正在复制文件...'
    Copy-Package -Source $Source -Target $Target

    # A fresh package starts from its official configuration.  Re-apply the
    # preserved user patches (or the patches captured above) to it.
    if (Test-Path -LiteralPath (Join-Path $Target 'updater\playkit-updater.exe')) {
        Write-Log '正在应用用户配置 patch...'
        Invoke-Updater -Root $Target -Arguments @('repair', '--root', $Target, '--quiet') | Out-Null
    } else {
        throw '安装包缺少 updater\playkit-updater.exe，无法应用用户配置。'
    }

    $exe = Join-Path $Target 'mpv.exe'
    $icon = Join-Path $Target 'installer\windows\mpv-playkit.ico'
    $version = Get-PackageVersion
    $repository = Get-RepositoryUrl

    if ($StartMenuShortcut) {
        $startMenu = Join-Path ([Environment]::GetFolderPath('Programs')) 'mpv PlayKit'
        New-Item -ItemType Directory -Force -Path $startMenu | Out-Null
        New-Shortcut -Path (Join-Path $startMenu 'mpv PlayKit.lnk') -TargetPath $exe `
            -WorkingDirectory $Target -IconLocation "$icon,0" -Description 'mpv PlayKit'
        $uninstallScript = Join-Path $Target 'installer\windows\uninstall.ps1'
        New-Shortcut -Path (Join-Path $startMenu '卸载 mpv PlayKit.lnk') -TargetPath 'powershell.exe' `
            -Arguments "-NoProfile -ExecutionPolicy Bypass -File `"$uninstallScript`"" `
            -WorkingDirectory $Target -IconLocation "$icon,0" -Description '卸载 mpv PlayKit'
    }

    if ($DesktopShortcut) {
        $desktop = [Environment]::GetFolderPath('Desktop')
        New-Shortcut -Path (Join-Path $desktop 'mpv PlayKit.lnk') -TargetPath $exe `
            -WorkingDirectory $Target -IconLocation "$icon,0" -Description 'mpv PlayKit'
    }

    Write-Log '正在注册快捷方式与右键菜单...'
    Register-Integration -Target $Target -ExePath $exe -Extensions (Get-MediaExtensions) `
        -ContextMenu $ContextMenu -FileAssociations $FileAssociations `
        -Version $version -Repository $repository
}

function Show-Gui {
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    [System.Windows.Forms.Application]::EnableVisualStyles()

    $form = New-Object System.Windows.Forms.Form
    $form.Text = 'mpv PlayKit 安装程序'
    $form.StartPosition = 'CenterScreen'
    $form.FormBorderStyle = [System.Windows.Forms.FormBorderStyle]::FixedDialog
    $form.MaximizeBox = $false
    $form.MinimizeBox = $false
    $form.ClientSize = New-Object System.Drawing.Size(560, 340)
    $form.Font = [System.Drawing.SystemFonts]::MessageBoxFont

    $label = New-Object System.Windows.Forms.Label
    $label.Text = '安装位置：'
    $label.Location = New-Object System.Drawing.Point(16, 18)
    $label.AutoSize = $true
    $form.Controls.Add($label)

    $pathBox = New-Object System.Windows.Forms.TextBox
    $pathBox.Location = New-Object System.Drawing.Point(16, 40)
    $pathBox.Size = New-Object System.Drawing.Size(440, 24)
    $pathBox.Text = $InstallDir
    $form.Controls.Add($pathBox)

    $browse = New-Object System.Windows.Forms.Button
    $browse.Text = '浏览...'
    $browse.Location = New-Object System.Drawing.Point(464, 38)
    $browse.Size = New-Object System.Drawing.Size(80, 28)
    $browse.Add_Click({
        $dialog = New-Object System.Windows.Forms.FolderBrowserDialog
        $dialog.Description = '选择 mpv PlayKit 的安装位置'
        $dialog.SelectedPath = $pathBox.Text
        if ($dialog.ShowDialog($form) -eq [System.Windows.Forms.DialogResult]::OK) {
            $pathBox.Text = $dialog.SelectedPath
        }
    })
    $form.Controls.Add($browse)

    $desktopBox = New-Object System.Windows.Forms.CheckBox
    $desktopBox.Text = '创建桌面快捷方式'
    $desktopBox.Location = New-Object System.Drawing.Point(16, 84)
    $desktopBox.AutoSize = $true
    $desktopBox.Checked = $true
    $form.Controls.Add($desktopBox)

    $startBox = New-Object System.Windows.Forms.CheckBox
    $startBox.Text = '创建开始菜单快捷方式'
    $startBox.Location = New-Object System.Drawing.Point(16, 112)
    $startBox.AutoSize = $true
    $startBox.Checked = $true
    $form.Controls.Add($startBox)

    $contextBox = New-Object System.Windows.Forms.CheckBox
    $contextBox.Text = '添加“用 mpv PlayKit 播放”右键菜单'
    $contextBox.Location = New-Object System.Drawing.Point(16, 140)
    $contextBox.AutoSize = $true
    $contextBox.Checked = $true
    $form.Controls.Add($contextBox)

    $assocBox = New-Object System.Windows.Forms.CheckBox
    $assocBox.Text = '在“打开方式”中注册常见音视频格式'
    $assocBox.Location = New-Object System.Drawing.Point(16, 168)
    $assocBox.AutoSize = $true
    $assocBox.Checked = $true
    $form.Controls.Add($assocBox)

    $status = New-Object System.Windows.Forms.Label
    $status.Text = '安装程序只写入当前用户（HKCU），不需要管理员权限。'
    $status.Location = New-Object System.Drawing.Point(16, 206)
    $status.Size = New-Object System.Drawing.Size(528, 40)
    $form.Controls.Add($status)

    $install = New-Object System.Windows.Forms.Button
    $install.Text = '安装'
    $install.Location = New-Object System.Drawing.Point(384, 284)
    $install.Size = New-Object System.Drawing.Size(76, 32)

    $cancel = New-Object System.Windows.Forms.Button
    $cancel.Text = '取消'
    $cancel.Location = New-Object System.Drawing.Point(468, 284)
    $cancel.Size = New-Object System.Drawing.Size(76, 32)
    $cancel.Add_Click({ $form.Close() })
    $form.Controls.Add($cancel)

    $install.Add_Click({
        $target = $pathBox.Text.Trim()
        if (-not $target) {
            [System.Windows.Forms.MessageBox]::Show($form, '请选择安装位置。', 'mpv PlayKit', 'OK', 'Warning') | Out-Null
            return
        }
        try {
            $target = [IO.Path]::GetFullPath($target).TrimEnd('\')
        } catch {
            [System.Windows.Forms.MessageBox]::Show($form, '安装位置无效。', 'mpv PlayKit', 'OK', 'Warning') | Out-Null
            return
        }
        if ((Test-Path -LiteralPath (Join-Path $target 'mpv.exe')) -and
            ([IO.Path]::GetFullPath($target) -ine [IO.Path]::GetFullPath($script:PackageRoot))) {
            $answer = [System.Windows.Forms.MessageBox]::Show(
                $form,
                "目标目录已存在 mpv PlayKit，是否覆盖安装？`n`n$target",
                'mpv PlayKit', 'YesNo', 'Question')
            if ($answer -ne 'Yes') { return }
        }

        $running = @(Get-Process -Name 'mpv' -ErrorAction SilentlyContinue | Where-Object {
            try { $_.Path -and ([IO.Path]::GetFullPath($_.Path) -ieq [IO.Path]::GetFullPath((Join-Path $target 'mpv.exe'))) } catch { $false }
        })
        if ($running.Count -gt 0) {
            [System.Windows.Forms.MessageBox]::Show($form, '请先关闭正在运行的 mpv PlayKit。', 'mpv PlayKit', 'OK', 'Warning') | Out-Null
            return
        }

        $install.Enabled = $false
        $browse.Enabled = $false
        $status.Text = '正在安装，请稍候...'
        $form.Cursor = [System.Windows.Forms.Cursors]::WaitCursor
        $form.Refresh()
        try {
            Install-PlayKit -Source $script:PackageRoot -Target $target `
                -DesktopShortcut $desktopBox.Checked -StartMenuShortcut $startBox.Checked `
                -ContextMenu $contextBox.Checked -FileAssociations $assocBox.Checked
            $form.Close()
            $answer = [System.Windows.Forms.MessageBox]::Show(
                "mpv PlayKit 已安装到：`n$target`n`n是否立即运行？",
                'mpv PlayKit', 'YesNo', 'Information')
            if ($answer -eq 'Yes') {
                Start-Process -FilePath (Join-Path $target 'mpv.exe') -WorkingDirectory $target
            }
        } catch {
            $install.Enabled = $true
            $browse.Enabled = $true
            $form.Cursor = [System.Windows.Forms.Cursors]::Default
            $status.Text = '安装失败。'
            [System.Windows.Forms.MessageBox]::Show($form, $_.Exception.Message, 'mpv PlayKit 安装失败', 'OK', 'Error') | Out-Null
        }
    })
    $form.Controls.Add($install)

    $form.AcceptButton = $install
    $form.CancelButton = $cancel
    [void]$form.ShowDialog()
}

try {
    if ($Silent) {
        Write-Log "mpv PlayKit installer: $script:PackageRoot -> $InstallDir"
        $running = @(Get-RunningMpv)
        if ($running.Count -gt 0) {
            throw 'mpv PlayKit 正在运行，请先关闭后再安装。'
        }
        Install-PlayKit -Source $script:PackageRoot -Target $InstallDir `
            -DesktopShortcut (-not $NoDesktopShortcut) -StartMenuShortcut (-not $NoStartMenuShortcut) `
            -ContextMenu (-not $NoContextMenu) -FileAssociations (-not $NoFileAssociations)
        Write-Log '安装完成。'
        if ($Launch) {
            Start-Process -FilePath $script:TargetExe -WorkingDirectory $InstallDir
        }
        exit 0
    }

    Show-Gui
    exit 0
} catch {
    if ($Silent) {
        [Console]::Error.WriteLine($_.Exception.Message)
        [Console]::Error.WriteLine($_.ScriptStackTrace)
        exit 1
    }
    Add-Type -AssemblyName System.Windows.Forms
    [System.Windows.Forms.MessageBox]::Show($_.Exception.Message, 'mpv PlayKit 安装失败', 'OK', 'Error') | Out-Null
    exit 1
}
