#requires -Version 5.1
<#
    mpv PlayKit - umpv association registration.

    Registers umpv.exe as a per-user "Open with" handler under HKCU only.
    It uses the dedicated ProgID "mpv-playkit.umpv" and never touches the
    normal mpv-playkit associations created by install.ps1.

    Usage:
      powershell -NoProfile -ExecutionPolicy Bypass -File umpv-register.ps1
      powershell -NoProfile -ExecutionPolicy Bypass -File umpv-register.ps1 -InstallDir "D:\Apps\mpv-playkit" -Silent
#>
[CmdletBinding()]
param(
    [string]$InstallDir = '',
    [switch]$Silent
)

$ErrorActionPreference = 'Stop'
$script:ProgId = 'mpv-playkit.umpv'
$script:Capabilities = 'HKCU:\Software\Clients\Media\mpv-playkit-umpv\Capabilities'
$script:PackageRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $InstallDir) { $InstallDir = $script:PackageRoot }
$InstallDir = [IO.Path]::GetFullPath($InstallDir).TrimEnd('\')
$script:ExePath = Join-Path $InstallDir 'umpv.exe'

function Write-Log {
    param([string]$Message)
    if (-not $Silent) { Write-Host $Message }
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
    $conf = Join-Path $InstallDir 'portable_config\script-opts\uosc.conf'
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

if (-not (Test-Path -LiteralPath $script:ExePath)) {
    throw "umpv.exe not found: $($script:ExePath)"
}

$classes = 'HKCU:\Software\Classes'
$icon = Join-Path $InstallDir 'installer\windows\mpv-playkit.ico'
$progRoot = Join-Path $classes $script:ProgId
New-Item -Path "$progRoot\DefaultIcon" -Force | Out-Null
New-Item -Path "$progRoot\shell\open\command" -Force | Out-Null
Set-ItemProperty -Path $progRoot -Name '(default)' -Value 'mpv PlayKit (single instance)'
Set-ItemProperty -Path $progRoot -Name 'FriendlyTypeName' -Value 'mpv PlayKit media file'
New-ItemProperty -Path $progRoot -Name 'EditFlags' -PropertyType DWord -Value 65536 -Force | Out-Null
Set-ItemProperty -Path "$progRoot\DefaultIcon" -Name '(default)' -Value "$icon,0"
Set-ItemProperty -Path $progRoot -Name 'AlwaysShowInOpenWith' -Value ''
Set-ItemProperty -Path "$progRoot\shell\open\command" -Name '(default)' -Value "`"$($script:ExePath)`" `"%1`""

$appRoot = Join-Path $classes 'Applications\umpv.exe'
New-Item -Path "$appRoot\DefaultIcon" -Force | Out-Null
New-Item -Path "$appRoot\shell\open\command" -Force | Out-Null
New-Item -Path "$appRoot\SupportedTypes" -Force | Out-Null
Set-ItemProperty -Path $appRoot -Name '(default)' -Value 'mpv PlayKit (single instance)'
Set-ItemProperty -Path $appRoot -Name 'FriendlyAppName' -Value 'mpv PlayKit (single instance)'
Set-ItemProperty -Path $appRoot -Name 'ApplicationCompany' -Value 'mpv PlayKit TLS contributors'
Set-ItemProperty -Path "$appRoot\DefaultIcon" -Name '(default)' -Value "$icon,0"
Set-ItemProperty -Path "$appRoot\shell\open\command" -Name '(default)' -Value "`"$($script:ExePath)`" `"%1`""

$appPaths = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths'
$appPath = Join-Path $appPaths 'umpv.exe'
New-Item -Path $appPath -Force | Out-Null
Set-ItemProperty -Path $appPath -Name '(default)' -Value $script:ExePath
Set-ItemProperty -Path $appPath -Name 'Path' -Value $InstallDir

New-Item -Path "$script:Capabilities\FileAssociations" -Force | Out-Null
Set-ItemProperty -Path $script:Capabilities -Name 'ApplicationName' -Value 'mpv PlayKit (single instance)'
Set-ItemProperty -Path $script:Capabilities -Name 'ApplicationDescription' -Value 'Open media in one mpv PlayKit window'
New-Item -Path 'HKCU:\Software\RegisteredApplications' -Force | Out-Null
Set-ItemProperty -Path 'HKCU:\Software\RegisteredApplications' -Name $script:ProgId `
    -Value 'Software\Clients\Media\mpv-playkit-umpv\Capabilities'

foreach ($perceived in @('video', 'audio')) {
    New-Item -Path "HKCU:\Software\Classes\SystemFileAssociations\$perceived\OpenWithList\umpv.exe" -Force | Out-Null
}

foreach ($ext in Get-MediaExtensions) {
    try {
        Set-RegNone -KeyPath 'Software\Classes\Applications\umpv.exe\SupportedTypes' -Name ".$ext"
    } catch {
        Write-Log "Skipped SupportedTypes for .$ext : $($_.Exception.Message)"
    }
    try {
        New-Item -Path (Join-Path $classes ".$ext\OpenWithProgids") -Force -ErrorAction Stop | Out-Null
        Set-RegNone -KeyPath "Software\Classes\.$ext\OpenWithProgids" -Name $script:ProgId
        Set-ItemProperty -Path "$script:Capabilities\FileAssociations" -Name ".$ext" -Value $script:ProgId
    } catch {
        Write-Log "Skipped OpenWithProgids for .$ext : $($_.Exception.Message)"
    }
}

Write-Log "umpv registered for the current user: $($script:ExePath)"