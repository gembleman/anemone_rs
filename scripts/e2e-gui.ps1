param(
    [string]$ExePath = (Join-Path $PSScriptRoot '..\target\i686-pc-windows-msvc\release\anemone_rs.exe')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not ('AnemoneE2E.NativeMethods' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

namespace AnemoneE2E
{
    public static class NativeMethods
    {
        private const uint WM_COMMAND = 0x0111;
        private const uint CB_GETCURSEL = 0x0147;
        private const uint CB_SETCURSEL = 0x014E;
        private const uint SMTO_BLOCK = 0x0001;
        private const uint SMTO_ABORTIFHUNG = 0x0002;
        private const uint MessageTimeoutMilliseconds = 5000;

        private delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr lparam);

        [DllImport("user32.dll")]
        private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lparam);

        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        private static extern int GetClassNameW(IntPtr hwnd, StringBuilder className, int maxCount);

        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        private static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int maxCount);

        [DllImport("user32.dll")]
        private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);

        [DllImport("user32.dll", SetLastError = true)]
        private static extern IntPtr GetDlgItem(IntPtr dialog, int controlId);

        [DllImport("user32.dll", SetLastError = true)]
        private static extern IntPtr SendMessageTimeoutW(
            IntPtr hwnd,
            uint message,
            UIntPtr wparam,
            IntPtr lparam,
            uint flags,
            uint timeout,
            out UIntPtr result);

        public static IntPtr FindProcessWindow(int processId, string className, string title)
        {
            IntPtr found = IntPtr.Zero;
            EnumWindows(delegate(IntPtr hwnd, IntPtr lparam)
            {
                uint ownerProcessId;
                GetWindowThreadProcessId(hwnd, out ownerProcessId);
                if (ownerProcessId != (uint)processId)
                {
                    return true;
                }

                StringBuilder actualClass = new StringBuilder(256);
                StringBuilder actualTitle = new StringBuilder(512);
                GetClassNameW(hwnd, actualClass, actualClass.Capacity);
                GetWindowTextW(hwnd, actualTitle, actualTitle.Capacity);
                if (actualClass.ToString() == className && actualTitle.ToString() == title)
                {
                    found = hwnd;
                    return false;
                }
                return true;
            }, IntPtr.Zero);
            return found;
        }

        public static IntPtr RequireControl(IntPtr dialog, int controlId)
        {
            IntPtr control = GetDlgItem(dialog, controlId);
            if (control == IntPtr.Zero)
            {
                throw new Win32Exception(Marshal.GetLastWin32Error(),
                    "Control not found: " + controlId);
            }
            return control;
        }

        public static void SelectCombo(IntPtr combo, int index)
        {
            long selected = Send(combo, CB_SETCURSEL, new UIntPtr((uint)index), IntPtr.Zero);
            if (selected != index)
            {
                throw new InvalidOperationException(
                    "Could not select combo index " + index + "; result was " + selected);
            }
        }

        public static int GetComboSelection(IntPtr combo)
        {
            return checked((int)Send(combo, CB_GETCURSEL, UIntPtr.Zero, IntPtr.Zero));
        }

        public static void SendCommand(IntPtr window, int controlId, int notification, IntPtr control)
        {
            ulong value = (ulong)(ushort)controlId | ((ulong)(ushort)notification << 16);
            Send(window, WM_COMMAND, new UIntPtr(value), control);
        }

        private static long Send(IntPtr hwnd, uint message, UIntPtr wparam, IntPtr lparam)
        {
            UIntPtr result;
            IntPtr succeeded = SendMessageTimeoutW(
                hwnd,
                message,
                wparam,
                lparam,
                SMTO_BLOCK | SMTO_ABORTIFHUNG,
                MessageTimeoutMilliseconds,
                out result);
            if (succeeded == IntPtr.Zero)
            {
                throw new Win32Exception(Marshal.GetLastWin32Error(),
                    "Timed out sending Win32 message 0x" + message.ToString("X"));
            }
            return unchecked((long)result.ToUInt64());
        }
    }
}
'@
}

function Wait-ProcessWindow {
    param(
        [System.Diagnostics.Process]$Process,
        [string]$ClassName,
        [string]$Title,
        [string]$Description,
        [int]$TimeoutMilliseconds = 15000
    )

    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMilliseconds)
    do {
        $Process.Refresh()
        if ($Process.HasExited) {
            throw "$Description 대기 중 앱이 종료되었습니다. exit code: $($Process.ExitCode)"
        }

        $window = [AnemoneE2E.NativeMethods]::FindProcessWindow(
            $Process.Id,
            $ClassName,
            $Title
        )
        if ($window -ne [IntPtr]::Zero) {
            return $window
        }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)

    throw "$Description 창이 ${TimeoutMilliseconds}ms 안에 나타나지 않았습니다."
}

function Wait-WindowClosed {
    param(
        [System.Diagnostics.Process]$Process,
        [string]$ClassName,
        [string]$Title,
        [string]$Description,
        [int]$TimeoutMilliseconds = 5000
    )

    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMilliseconds)
    do {
        $window = [AnemoneE2E.NativeMethods]::FindProcessWindow(
            $Process.Id,
            $ClassName,
            $Title
        )
        if ($window -eq [IntPtr]::Zero) {
            return
        }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)

    throw "$Description 창이 ${TimeoutMilliseconds}ms 안에 닫히지 않았습니다."
}

$resolvedExe = (Resolve-Path -LiteralPath $ExePath).Path
$root = Join-Path ([System.IO.Path]::GetTempPath()) ("anemone-gui-e2e-" + [guid]::NewGuid())
$install = Join-Path $root 'install'
$localData = Join-Path $root 'local-data'
$installedExe = Join-Path $install 'anemone_rs.exe'
$process = $null
$mainWindow = [IntPtr]::Zero

try {
    # Keep the test independent from config.toml or portable markers beside the build artifact.
    New-Item -ItemType Directory -Path $install, $localData | Out-Null
    Copy-Item -LiteralPath $resolvedExe -Destination $installedExe

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $installedExe
    $startInfo.WorkingDirectory = $install
    $startInfo.UseShellExecute = $false
    $startInfo.EnvironmentVariables['LOCALAPPDATA'] = $localData
    $startInfo.EnvironmentVariables['APPDATA'] = $localData
    $process = [System.Diagnostics.Process]::Start($startInfo)
    if ($null -eq $process) {
        throw 'GUI 프로세스를 시작하지 못했습니다.'
    }

    $mainWindow = Wait-ProcessWindow `
        -Process $process `
        -ClassName 'AnemoneWindowClass' `
        -Title '아네모네' `
        -Description '메인'

    # Open Settings through the same WM_COMMAND path used by the context menu.
    [AnemoneE2E.NativeMethods]::SendCommand($mainWindow, 108, 0, [IntPtr]::Zero)
    $settingsWindow = Wait-ProcessWindow `
        -Process $process `
        -ClassName '#32770' `
        -Title '아네모네 설정' `
        -Description '설정'

    # Select Google (index 1) and deliver the normal CBN_SELCHANGE notification.
    $engineCombo = [AnemoneE2E.NativeMethods]::RequireControl($settingsWindow, 1260)
    [AnemoneE2E.NativeMethods]::SelectCombo($engineCombo, 1)
    [AnemoneE2E.NativeMethods]::SendCommand($settingsWindow, 1260, 1, $engineCombo)

    # Closing the dialog flushes pending settings through the production save path.
    $closeButton = [AnemoneE2E.NativeMethods]::RequireControl($settingsWindow, 1300)
    [AnemoneE2E.NativeMethods]::SendCommand($settingsWindow, 1300, 0, $closeButton)
    Wait-WindowClosed `
        -Process $process `
        -ClassName '#32770' `
        -Title '아네모네 설정' `
        -Description '설정'

    $configPath = Join-Path $localData 'Anemone\config.toml'
    if (-not (Test-Path -LiteralPath $configPath)) {
        throw "설정 파일이 생성되지 않았습니다: $configPath"
    }
    $configText = Get-Content -Raw -LiteralPath $configPath
    if ($configText -notmatch '(?m)^engine\s*=\s*"google"\s*$') {
        throw 'GUI에서 선택한 Google 엔진이 config.toml에 저장되지 않았습니다.'
    }

    # Reopen the dialog to verify the running app also retained the changed state.
    [AnemoneE2E.NativeMethods]::SendCommand($mainWindow, 108, 0, [IntPtr]::Zero)
    $reopenedSettings = Wait-ProcessWindow `
        -Process $process `
        -ClassName '#32770' `
        -Title '아네모네 설정' `
        -Description '재오픈한 설정'
    $reopenedCombo = [AnemoneE2E.NativeMethods]::RequireControl($reopenedSettings, 1260)
    $selectedEngine = [AnemoneE2E.NativeMethods]::GetComboSelection($reopenedCombo)
    if ($selectedEngine -ne 1) {
        throw "설정창 재오픈 후 엔진 선택이 유지되지 않았습니다: $selectedEngine"
    }

    $reopenedClose = [AnemoneE2E.NativeMethods]::RequireControl($reopenedSettings, 1300)
    [AnemoneE2E.NativeMethods]::SendCommand($reopenedSettings, 1300, 0, $reopenedClose)
    Wait-WindowClosed `
        -Process $process `
        -ClassName '#32770' `
        -Title '아네모네 설정' `
        -Description '재오픈한 설정'

    [AnemoneE2E.NativeMethods]::SendCommand($mainWindow, 110, 0, [IntPtr]::Zero)
    if (-not $process.WaitForExit(10000)) {
        throw '종료 명령 후 앱이 10000ms 안에 종료되지 않았습니다.'
    }
    if ($process.ExitCode -ne 0) {
        throw "GUI 앱이 비정상 종료했습니다. exit code: $($process.ExitCode)"
    }

    $logPath = Join-Path $localData 'Anemone\logs\anemone.log'
    if (-not (Test-Path -LiteralPath $logPath)) {
        throw "로그 파일이 생성되지 않았습니다: $logPath"
    }

    Write-Host 'GUI settings persistence and clean-exit E2E test passed.'
}
finally {
    if ($null -ne $process) {
        $process.Refresh()
        if (-not $process.HasExited) {
            if ($mainWindow -ne [IntPtr]::Zero) {
                try {
                    [AnemoneE2E.NativeMethods]::SendCommand($mainWindow, 110, 0, [IntPtr]::Zero)
                    [void]$process.WaitForExit(3000)
                }
                catch {
                    # The process is forcibly cleaned up below if graceful shutdown failed.
                }
            }
            $process.Refresh()
            if (-not $process.HasExited) {
                $process.Kill()
                [void]$process.WaitForExit(3000)
            }
        }
        $process.Dispose()
    }
    if (Test-Path -LiteralPath $root) {
        Remove-Item -LiteralPath $root -Recurse -Force
    }
}
