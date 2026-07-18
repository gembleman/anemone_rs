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
$oldAppData = $env:APPDATA
$oldSmokeExit = $env:ANEMONE_SMOKE_EXIT
try {
    $env:LOCALAPPDATA = $localData
    $env:APPDATA = $localData
    $env:ANEMONE_SMOKE_EXIT = '1'
    $process = Start-Process -FilePath $installedExe -PassThru -Wait
    if ($process.ExitCode -ne 0) { throw "GUI smoke test exited with $($process.ExitCode)" }

    $dataDir = Join-Path $localData 'Anemone'
    if (Test-Path -LiteralPath $dataDir) {
        throw 'GUI incorrectly fell back to the user data directory'
    }
    foreach ($path in 'config.toml', 'llm_usage.json', 'logs\anemone.log') {
        if (Test-Path -LiteralPath (Join-Path $install $path)) {
            throw "GUI unexpectedly wrote runtime data in a read-only executable directory: $path"
        }
    }
    Write-Host 'Read-only installation did not redirect runtime data away from the executable.'
}
finally {
    $env:LOCALAPPDATA = $oldLocalAppData
    $env:APPDATA = $oldAppData
    $env:ANEMONE_SMOKE_EXIT = $oldSmokeExit
    Set-Acl -LiteralPath $install -AclObject $originalAcl
    Remove-Item -LiteralPath $root -Recurse -Force
}
