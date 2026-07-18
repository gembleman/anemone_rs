$ErrorActionPreference = 'Stop'

$root = Join-Path ([System.IO.Path]::GetTempPath()) ("anemone-readonly-" + [guid]::NewGuid())
$install = Join-Path $root 'install'
$localData = Join-Path $root 'local-data'
New-Item -ItemType Directory -Path $install, $localData | Out-Null
$sourceExe = Join-Path $PSScriptRoot '..\target\i686-pc-windows-msvc\release\anemone_rs.exe'
$installedExe = Join-Path $install 'anemone_rs.exe'
Copy-Item -LiteralPath $sourceExe -Destination $installedExe

$identity = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
$originalAcl = Get-Acl -LiteralPath $install
$lockedAcl = Get-Acl -LiteralPath $install
$rights = [System.Security.AccessControl.FileSystemRights]::WriteData `
    -bor [System.Security.AccessControl.FileSystemRights]::CreateFiles `
    -bor [System.Security.AccessControl.FileSystemRights]::AppendData `
    -bor [System.Security.AccessControl.FileSystemRights]::Delete
$rule = [System.Security.AccessControl.FileSystemAccessRule]::new(
    $identity,
    $rights,
    [System.Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit',
    [System.Security.AccessControl.PropagationFlags]::None,
    [System.Security.AccessControl.AccessControlType]::Deny
)
$lockedAcl.AddAccessRule($rule)
Set-Acl -LiteralPath $install -AclObject $lockedAcl

$oldLocalAppData = $env:LOCALAPPDATA
$oldSmokeExit = $env:ANEMONE_SMOKE_EXIT
try {
    $env:LOCALAPPDATA = $localData
    $env:ANEMONE_SMOKE_EXIT = '1'
    $process = Start-Process -FilePath $installedExe -PassThru -Wait
    if ($process.ExitCode -ne 0) { throw "GUI smoke test exited with $($process.ExitCode)" }

    $dataDir = Join-Path $localData 'Anemone'
    if (-not (Test-Path -LiteralPath (Join-Path $dataDir 'config.toml'))) {
        throw 'GUI did not create config.toml in the user data directory'
    }
    if (-not (Test-Path -LiteralPath (Join-Path $dataDir 'logs\anemone.log'))) {
        throw 'GUI did not create its log in the user data directory'
    }
    foreach ($name in 'config.toml', 'anemone.log', 'llm_usage.json') {
        if (Test-Path -LiteralPath (Join-Path $install $name)) {
            throw "GUI wrote runtime data beside the executable: $name"
        }
    }
    Write-Host 'Read-only installation GUI smoke test passed.'
}
finally {
    $env:LOCALAPPDATA = $oldLocalAppData
    $env:ANEMONE_SMOKE_EXIT = $oldSmokeExit
    Set-Acl -LiteralPath $install -AclObject $originalAcl
    Remove-Item -LiteralPath $root -Recurse -Force
}
