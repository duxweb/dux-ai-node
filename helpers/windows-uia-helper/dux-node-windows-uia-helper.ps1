$ErrorActionPreference = 'Stop'

Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class DuxWin32 {
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hWnd);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetWindowTextLength(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);

    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow);

    [DllImport("user32.dll")]
    public static extern bool IsIconic(IntPtr hWnd);
}
"@

function Write-Response {
    param(
        [AllowNull()]$Id,
        [bool]$Ok,
        [AllowNull()]$Result,
        [AllowNull()]$Meta,
        [AllowNull()][string]$Error
    )

    $payload = [ordered]@{
        id = $Id
        ok = $Ok
        result = $Result
        meta = $Meta
        error = $Error
    }
    $payload | ConvertTo-Json -Depth 20 -Compress
}

function Get-StringValue {
    param([AllowNull()]$Value)
    if ($null -eq $Value) {
        return ''
    }
    return [string]$Value
}

function Get-WindowList {
    $items = New-Object System.Collections.Generic.List[object]
    $callback = [DuxWin32+EnumWindowsProc]{
        param([IntPtr]$Handle, [IntPtr]$Param)

        if (-not [DuxWin32]::IsWindowVisible($Handle)) {
            return $true
        }

        $length = [DuxWin32]::GetWindowTextLength($Handle)
        if ($length -le 0) {
            return $true
        }

        $builder = New-Object System.Text.StringBuilder ($length + 1)
        [void][DuxWin32]::GetWindowText($Handle, $builder, $builder.Capacity)
        $title = $builder.ToString()
        if ([string]::IsNullOrWhiteSpace($title)) {
            return $true
        }

        $processId = 0
        [void][DuxWin32]::GetWindowThreadProcessId($Handle, [ref]$processId)

        $processName = ''
        try {
            $process = Get-Process -Id $processId -ErrorAction Stop
            $processName = [string]$process.ProcessName
        }
        catch {
        }

        $items.Add([pscustomobject]@{
            handle = [string]$Handle
            title = $title
            process_id = [int]$processId
            process_name = $processName
        })
        return $true
    }

    [void][DuxWin32]::EnumWindows($callback, [IntPtr]::Zero)
    return $items
}

function Resolve-TargetWindow {
    param($Payload)

    $windowTitle = (Get-StringValue $Payload.window_title).Trim()
    $processName = (Get-StringValue $Payload.process_name).Trim().ToLowerInvariant()
    $appName = (Get-StringValue $Payload.app_name).Trim().ToLowerInvariant()

    $windows = @(Get-WindowList)
    if ($windowTitle -ne '') {
        $needle = $windowTitle.ToLowerInvariant()
        $matches = $windows | Where-Object { ([string]$_.title).ToLowerInvariant().Contains($needle) }
        if ($matches) {
            return $matches[0]
        }
    }

    if ($processName -ne '') {
        $matches = $windows | Where-Object { ([string]$_.process_name).ToLowerInvariant().Contains($processName) }
        if ($matches) {
            return $matches[0]
        }
    }

    if ($appName -ne '') {
        $matches = $windows | Where-Object {
            ([string]$_.title).ToLowerInvariant().Contains($appName) -or
            ([string]$_.process_name).ToLowerInvariant().Contains($appName)
        }
        if ($matches) {
            return $matches[0]
        }
    }

    return $null
}

function Focus-Window {
    param($Target)

    if ($null -eq $Target) {
        return $false
    }

    $handle = [IntPtr]::new([int64]$Target.handle)
    [void][DuxWin32]::ShowWindowAsync($handle, 9)
    Start-Sleep -Milliseconds 80
    return [DuxWin32]::SetForegroundWindow($handle)
}

try {
    $line = [Console]::In.ReadLine()
    if ([string]::IsNullOrWhiteSpace($line)) {
        Write-Output (Write-Response -Id $null -Ok $false -Result $null -Meta @{ helper = 'windows-uia-powershell' } -Error 'empty_request')
        exit 0
    }

    $request = $line | ConvertFrom-Json -Depth 20
    $id = $request.id
    $action = Get-StringValue $request.action
    $payload = $request.payload
    if ($null -eq $payload) {
        $payload = @{}
    }

    switch ($action) {
        'ui.status' {
            Write-Output (Write-Response -Id $id -Ok $true -Result @{
                platform = 'windows'
                helper = 'powershell-uia'
                ready = $true
                trusted = $true
                summary = 'Windows UI helper 已就绪'
            } -Meta @{ scaffold = $false } -Error $null)
        }
        'ax.status' {
            Write-Output (Write-Response -Id $id -Ok $true -Result @{
                platform = 'windows'
                helper = 'powershell-uia'
                ready = $true
                trusted = $true
                summary = 'Windows UI helper 已就绪'
            } -Meta @{ scaffold = $false } -Error $null)
        }
        'app.activate' {
            $target = Resolve-TargetWindow -Payload $payload
            if ($null -eq $target) {
                Write-Output (Write-Response -Id $id -Ok $false -Result $null -Meta @{ scaffold = $false } -Error 'application_not_found')
                break
            }
            $focused = Focus-Window -Target $target
            Write-Output (Write-Response -Id $id -Ok $focused -Result @{
                platform = 'windows'
                process_name = $target.process_name
                process_id = $target.process_id
                window_title = $target.title
                summary = "已激活窗口 $($target.title)"
            } -Meta @{ scaffold = $false } -Error $(if ($focused) { $null } else { 'application_activate_failed' }))
        }
        'window.focus' {
            $target = Resolve-TargetWindow -Payload $payload
            if ($null -eq $target) {
                Write-Output (Write-Response -Id $id -Ok $false -Result $null -Meta @{ scaffold = $false } -Error 'window_not_found')
                break
            }
            $focused = Focus-Window -Target $target
            Write-Output (Write-Response -Id $id -Ok $focused -Result @{
                platform = 'windows'
                process_name = $target.process_name
                process_id = $target.process_id
                window_title = $target.title
                summary = "已聚焦窗口 $($target.title)"
            } -Meta @{ scaffold = $false } -Error $(if ($focused) { $null } else { 'window_focus_failed' }))
        }
        'ui.tree' {
            $target = Resolve-TargetWindow -Payload $payload
            $windows = @(Get-WindowList)
            if ($null -ne $target) {
                $windows = $windows | Where-Object { [int]$_.process_id -eq [int]$target.process_id }
            }
            Write-Output (Write-Response -Id $id -Ok $true -Result @{
                platform = 'windows'
                windows = @($windows)
                summary = '已获取窗口列表'
            } -Meta @{ scaffold = $false } -Error $null)
        }
        'ax.tree' {
            $target = Resolve-TargetWindow -Payload $payload
            $windows = @(Get-WindowList)
            if ($null -ne $target) {
                $windows = $windows | Where-Object { [int]$_.process_id -eq [int]$target.process_id }
            }
            Write-Output (Write-Response -Id $id -Ok $true -Result @{
                platform = 'windows'
                windows = @($windows)
                summary = '已获取窗口列表'
            } -Meta @{ scaffold = $false } -Error $null)
        }
        default {
            Write-Output (Write-Response -Id $id -Ok $false -Result $null -Meta @{ scaffold = $true } -Error "unsupported_action:$action")
        }
    }
}
catch {
    Write-Output (Write-Response -Id $null -Ok $false -Result $null -Meta @{ scaffold = $false } -Error ([string]$_.Exception.Message))
    exit 0
}
