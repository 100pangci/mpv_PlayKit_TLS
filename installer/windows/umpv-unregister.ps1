#requires -Version 5.1
<#
    mpv PlayKit - remove umpv associations written by umpv-register.ps1.

    Removes only the dedicated "mpv-playkit.umpv" ProgID and umpv.exe entries.
    The normal mpv-playkit associations are left untouched.
#>
[CmdletBinding()]
param(
    [string]$InstallDir = '',
    [switch]$Silent
)

$ErrorActionPreference = 'Stop'
$script:ProgId = 'mpv-playkit.umpv'
$script:PackageRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $InstallDir) { $InstallDir = $script:PackageRoot }
$InstallDir = [IO.Path]::GetFullPath($InstallDir).TrimEnd('\')

function Write-Log {
    param([string]$Message)
    if (-not $Silent) { Write-Host $Message }
}

function Remove-KeyIfPresent {
    param([string]$Path)
    if (Test-Path -LiteralPath $Path) {
        Remove-Item -LiteralPath $Path -Recurse -Force
    }
}

function Remove-ValueIfPresent {
    param([string]$KeyPath, [string]$Name)
    if (-not (Test-Path -LiteralPath $KeyPath)) { return }
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($KeyPath, $true)
    if ($null -eq $key) { return }
    try {
        $key.DeleteValue($Name, $false)
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

foreach ($ext in Get-MediaExtensions) {
    Remove-ValueIfPresent -KeyPath "Software\Classes\.$ext\OpenWithProgids" -Name $script:ProgId
    Remove-ValueIfPresent -KeyPath 'Software\Classes\Applications\umpv.exe\SupportedTypes' -Name ".$ext"
}

foreach ($perceived in @('video', 'audio')) {
    Remove-KeyIfPresent "HKCU:\Software\Classes\SystemFileAssociations\$perceived\OpenWithList\umpv.exe"
}

Remove-ValueIfPresent -KeyPath 'Software\RegisteredApplications' -Name $script:ProgId
Remove-KeyIfPresent 'HKCU:\Software\Clients\Media\mpv-playkit-umpv'
Remove-KeyIfPresent 'HKCU:\Software\Classes\Applications\umpv.exe'
Remove-KeyIfPresent 'HKCU:\Software\Classes\mpv-playkit.umpv'
Remove-KeyIfPresent 'HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths\umpv.exe'

Write-Log 'umpv associations removed for the current user.'