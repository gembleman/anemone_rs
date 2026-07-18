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
        [StructLayout(LayoutKind.Sequential)]
        private struct RECT { public int Left, Top, Right, Bottom; }
        [StructLayout(LayoutKind.Sequential)]
        private struct POINT { public int X, Y; }

        private const uint WM_COMMAND = 0x0111;
        private const uint BM_SETCHECK = 0x00F1;
        private const uint CB_GETCURSEL = 0x0147;
        private const uint CB_SETCURSEL = 0x014E;
        private const uint BST_UNCHECKED = 0;
        private const uint BST_CHECKED = 1;
        private const uint SWP_NOSIZE = 0x0001;
        private const uint SWP_NOZORDER = 0x0004;
        private const uint SWP_NOACTIVATE = 0x0010;
        private const uint SMTO_BLOCK = 0x0001;
        private const uint SMTO_ABORTIFHUNG = 0x0002;
        private const uint MessageTimeoutMilliseconds = 5000;

        private delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr lparam);

        [DllImport("user32.dll")]
        private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lparam);

        [DllImport("user32.dll")]
        private static extern bool EnumChildWindows(IntPtr parent, EnumWindowsProc callback, IntPtr lparam);

        [DllImport("user32.dll")]
        private static extern int GetDlgCtrlID(IntPtr hwnd);

        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        private static extern int GetClassNameW(IntPtr hwnd, StringBuilder className, int maxCount);

        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        private static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int maxCount);

        [DllImport("user32.dll")]
        private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);

        [DllImport("user32.dll", SetLastError = true)]
        private static extern IntPtr GetDlgItem(IntPtr dialog, int controlId);

        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr CreateWindowExW(uint exStyle, string className, string title,
            uint style, int x, int y, int width, int height, IntPtr parent, IntPtr menu,
            IntPtr instance, IntPtr parameter);

        [DllImport("user32.dll")]
        private static extern bool DestroyWindow(IntPtr hwnd);

        [DllImport("user32.dll")]
        private static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);

        [DllImport("user32.dll")]
        private static extern IntPtr GetForegroundWindow();

        [DllImport("user32.dll")]
        private static extern bool SetForegroundWindow(IntPtr hwnd);

        [DllImport("user32.dll", SetLastError = true)]
        private static extern bool SetWindowPos(IntPtr hwnd, IntPtr insertAfter,
            int x, int y, int width, int height, uint flags);

        [DllImport("user32.dll")]
        private static extern IntPtr WindowFromPoint(POINT point);

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
                StringBuilder ids = new StringBuilder();
                EnumChildWindows(dialog, delegate(IntPtr hwnd, IntPtr lparam)
                {
                    if (ids.Length > 0) ids.Append(", ");
                    ids.Append(GetDlgCtrlID(hwnd));
                    return true;
                }, IntPtr.Zero);
                throw new Win32Exception(Marshal.GetLastWin32Error(),
                    "Control not found: " + controlId + "; child IDs: " + ids);
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

        public static void AssertClickThrough(IntPtr overlay)
        {
            const uint WS_POPUP = 0x80000000;
            const uint WS_VISIBLE = 0x10000000;
            RECT rect;
            if (!GetWindowRect(overlay, out rect))
                throw new Win32Exception(Marshal.GetLastWin32Error(), "GetWindowRect failed");
            IntPtr target = CreateWindowExW(0, "BUTTON", "Anemone click target",
                WS_POPUP | WS_VISIBLE, rect.Left + 20, rect.Top + 20, 80, 80,
                IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
            if (target == IntPtr.Zero)
                throw new Win32Exception(Marshal.GetLastWin32Error(), "click target creation failed");
            try
            {
                SendCommand(overlay, 103, 0, IntPtr.Zero);
                POINT point = new POINT { X = rect.Left + 40, Y = rect.Top + 40 };
                IntPtr hit = WindowFromPoint(point);
                if (hit != target)
                    throw new InvalidOperationException(
                        "click-through hit-test returned " + hit + " instead of target " + target);
            }
            finally
            {
                SendCommand(overlay, 103, 0, IntPtr.Zero);
                DestroyWindow(target);
            }
        }

        public static void AssertMagneticFromSettings(IntPtr overlay, IntPtr settings)
        {
            const uint WS_EX_TOPMOST = 0x00000008;
            const uint WS_POPUP = 0x80000000;
            const uint WS_VISIBLE = 0x10000000;
            const int MagneticControlId = 1211;
            RECT overlayBefore;
            if (!GetWindowRect(overlay, out overlayBefore))
                throw new Win32Exception(Marshal.GetLastWin32Error(), "overlay GetWindowRect failed");

            IntPtr target = CreateWindowExW(WS_EX_TOPMOST, "BUTTON", "Anemone magnetic target",
                WS_POPUP | WS_VISIBLE, overlayBefore.Left + 240, overlayBefore.Top + 180,
                180, 100, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
            if (target == IntPtr.Zero)
                throw new Win32Exception(Marshal.GetLastWin32Error(), "magnetic target creation failed");

            IntPtr checkbox = RequireControl(settings, MagneticControlId);
            try
            {
                if (!SetForegroundWindow(target) || !SetForegroundWindow(settings) ||
                    GetForegroundWindow() != settings)
                    throw new InvalidOperationException("Could not prepare magnetic target z-order");

                Send(checkbox, BM_SETCHECK, new UIntPtr(BST_CHECKED), IntPtr.Zero);
                SendCommand(settings, MagneticControlId, 0, checkbox);
                System.Threading.Thread.Sleep(300);

                RECT targetBefore;
                if (!GetWindowRect(target, out targetBefore))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "target GetWindowRect failed");
                if (!SetWindowPos(target, IntPtr.Zero, targetBefore.Left + 30, targetBefore.Top + 20,
                    0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "target move failed");
                System.Threading.Thread.Sleep(500);

                RECT overlayAfter;
                if (!GetWindowRect(overlay, out overlayAfter))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "overlay GetWindowRect failed");
                if (overlayAfter.Left - overlayBefore.Left != 30 ||
                    overlayAfter.Top - overlayBefore.Top != 20)
                    throw new InvalidOperationException(
                        "magnetic overlay moved by (" +
                        (overlayAfter.Left - overlayBefore.Left) + ", " +
                        (overlayAfter.Top - overlayBefore.Top) + ") instead of (30, 20)");
            }
            finally
            {
                Send(checkbox, BM_SETCHECK, new UIntPtr(BST_UNCHECKED), IntPtr.Zero);
                SendCommand(settings, MagneticControlId, 0, checkbox);
                DestroyWindow(target);
            }
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
$runtimeDir = Join-Path $root 'runtime'
$dataDir = $runtimeDir
$runtimeExe = Join-Path $runtimeDir 'anemone_rs.exe'
$process = $null
$mainWindow = [IntPtr]::Zero

try {
    # Copy the executable and keep all runtime data beside it.
    New-Item -ItemType Directory -Path $runtimeDir | Out-Null
    Copy-Item -LiteralPath $resolvedExe -Destination $runtimeExe

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $runtimeExe
    $startInfo.WorkingDirectory = $runtimeDir
    $startInfo.UseShellExecute = $false
    $process = [System.Diagnostics.Process]::Start($startInfo)
    if ($null -eq $process) {
        throw 'GUI 프로세스를 시작하지 못했습니다.'
    }

    $mainWindow = Wait-ProcessWindow `
        -Process $process `
        -ClassName 'AnemoneWindowClass' `
        -Title '아네모네' `
        -Description '메인'

    # Verify that click-through skips the layered top-level overlay across processes.
    [AnemoneE2E.NativeMethods]::AssertClickThrough($mainWindow)

    # Open Settings through the same WM_COMMAND path used by the context menu.
    [AnemoneE2E.NativeMethods]::SendCommand($mainWindow, 108, 0, [IntPtr]::Zero)
    $settingsWindow = Wait-ProcessWindow `
        -Process $process `
        -ClassName '#32770' `
        -Title '아네모네 설정' `
        -Description '설정'

    # Enable magnetic mode through Settings and follow an external-process target window.
    [AnemoneE2E.NativeMethods]::AssertMagneticFromSettings($mainWindow, $settingsWindow)

    # Select Google (index 1) and deliver the normal CBN_SELCHANGE notification.
    $engineCombo = [AnemoneE2E.NativeMethods]::RequireControl($settingsWindow, 1260)
    [AnemoneE2E.NativeMethods]::SelectCombo($engineCombo, 1)
    [AnemoneE2E.NativeMethods]::SendCommand($settingsWindow, 1260, 1, $engineCombo)

    # Settings are persisted only when Apply is clicked.
    $applyButton = [AnemoneE2E.NativeMethods]::RequireControl($settingsWindow, 1301)
    [AnemoneE2E.NativeMethods]::SendCommand($settingsWindow, 1301, 0, $applyButton)
    $closeButton = [AnemoneE2E.NativeMethods]::RequireControl($settingsWindow, 1300)
    [AnemoneE2E.NativeMethods]::SendCommand($settingsWindow, 1300, 0, $closeButton)
    Wait-WindowClosed `
        -Process $process `
        -ClassName '#32770' `
        -Title '아네모네 설정' `
        -Description '설정'

    $configPath = Join-Path $dataDir 'config.toml'
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

    $logPath = Join-Path $dataDir 'logs\anemone.log'
    if (-not (Test-Path -LiteralPath $logPath)) {
        throw "로그 파일이 생성되지 않았습니다: $logPath"
    }

    Write-Host 'GUI click-through, magnetic, settings persistence, and clean-exit E2E test passed.'
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
