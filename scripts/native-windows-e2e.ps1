[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$InstallerPath,
  [string]$OutputDirectory = "test-results/native-windows",
  [int]$DebugPort = 9222
)

$ErrorActionPreference = "Stop"
if (-not $IsWindows -and $env:OS -ne "Windows_NT") {
  throw "The installed native harness only runs on Windows. Browser smoke uses npm run test:ui."
}

$installer = (Resolve-Path $InstallerPath).Path
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$installerHash = (Get-FileHash -Algorithm SHA256 $installer).Hash.ToLowerInvariant()
$appProcess = $null
$uninstaller = $null

function Find-AlicentExecutable {
  $uninstallRoots = @(
    "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*",
    "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*",
    "HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*"
  )
  foreach ($entry in Get-ItemProperty $uninstallRoots -ErrorAction SilentlyContinue) {
    if ($entry.DisplayName -ne "Alicent") { continue }
    if ($entry.UninstallString) {
      $script:uninstaller = [regex]::Match($entry.UninstallString, '"([^\"]+\.exe)"|([^\s]+\.exe)').Groups |
        Where-Object { $_.Value -match '\.exe$' } |
        Select-Object -First 1 -ExpandProperty Value
    }
    if ($entry.DisplayIcon) {
      $candidate = $entry.DisplayIcon.Trim('"').Split(',')[0]
      if (Test-Path $candidate) { return (Resolve-Path $candidate).Path }
    }
  }

  $candidates = @(
    (Join-Path $env:LOCALAPPDATA "Alicent\Alicent.exe"),
    (Join-Path $env:ProgramFiles "Alicent\Alicent.exe"),
    (Join-Path ${env:ProgramFiles(x86)} "Alicent\Alicent.exe")
  ) | Where-Object { $_ -and (Test-Path $_) }
  if ($candidates.Count -eq 0) {
    throw "Installer completed, but Alicent.exe was not found in uninstall metadata or standard install roots."
  }
  return (Resolve-Path $candidates[0]).Path
}

try {
  $install = Start-Process -FilePath $installer -ArgumentList "/S" -Wait -PassThru
  if ($install.ExitCode -ne 0) { throw "NSIS installer exited with code $($install.ExitCode)." }

  $executable = Find-AlicentExecutable
  $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$DebugPort --remote-allow-origins=*"
  $appProcess = Start-Process -FilePath $executable -PassThru

  & node scripts/native-e2e.mjs `
    --endpoint "http://127.0.0.1:$DebugPort" `
    --output $OutputDirectory `
    --installer-sha256 $installerHash `
    --executable-name ([System.IO.Path]::GetFileName($executable))
  if ($LASTEXITCODE -ne 0) { throw "Installed native E2E runner failed with code $LASTEXITCODE." }
}
finally {
  if ($appProcess -and -not $appProcess.HasExited) {
    Stop-Process -Id $appProcess.Id -Force -ErrorAction SilentlyContinue
  }
  if ($uninstaller -and (Test-Path $uninstaller)) {
    Start-Process -FilePath $uninstaller -ArgumentList "/S" -Wait -ErrorAction SilentlyContinue
  }
}
