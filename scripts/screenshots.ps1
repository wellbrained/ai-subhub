<#
.SYNOPSIS
  Regenerates the README screenshots (docs\images\demo-*.png) from demo mode.

.DESCRIPTION
  Starts the app with --demo --demo-hidden once per picture, captures its window with
  PrintWindow (nothing shows on screen, the mouse is never moved), trims the invisible window
  frame and saves the PNG. Demo mode touches no real account, hub, setting or startup entry.

  demo-more-providers.png is a crop of a tall Limits now capture (Copilot, Cursor and the first
  Antigravity window). Its offsets follow the demo layout; check the picture after layout changes.

.PARAMETER Exe
  The app to run. Default: target\debug\ai-subhub.exe (build it first: cargo build -p subhub-app).

.PARAMETER Only
  Names to take (e.g. demo-popover, demo-settings-hub). Default: all.

.EXAMPLE
  cargo build -p subhub-app; scripts\screenshots.ps1
.EXAMPLE
  scripts\screenshots.ps1 -Only demo-settings-hub, demo-more-providers
#>
[CmdletBinding()]
param(
  [string]$Exe,
  [string[]]$Only,
  # Seconds to wait for the window to render its demo data.
  [int]$Wait = 9
)

$ErrorActionPreference = 'Stop'
# "a,b" arrives as one string when the script runs with powershell -File.
$Only = @($Only | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
$root = Split-Path -Parent $PSScriptRoot
if (-not $Exe) { $Exe = Join-Path $root 'target\debug\ai-subhub.exe' }
if (-not (Test-Path $Exe)) { throw "$Exe not found. Build it first: cargo build -p subhub-app" }
$dst = Join-Path $root 'docs\images'
$tmp = Join-Path ([IO.Path]::GetTempPath()) 'subhub-shots'
New-Item -ItemType Directory -Force $tmp | Out-Null

Add-Type -AssemblyName System.Drawing
if (-not ('SubHubShots' -as [type])) {
  Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices;
public class SubHubShots {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
}

# A visible window of the process with exactly this title ('' = the popover, which has none).
function Find-Window([int]$ProcId, [string]$Title) {
  $script:found = [IntPtr]::Zero
  $cb = [SubHubShots+EnumProc]{ param($h, $l)
    $p = 0; [SubHubShots]::GetWindowThreadProcessId($h, [ref]$p) | Out-Null
    if ($p -eq $ProcId -and [SubHubShots]::IsWindowVisible($h)) {
      $sb = New-Object Text.StringBuilder 256; [SubHubShots]::GetWindowText($h, $sb, 256) | Out-Null
      if ($sb.ToString() -eq $Title) { $script:found = $h; return $false }
    }
    return $true }
  [SubHubShots]::EnumWindows($cb, [IntPtr]::Zero) | Out-Null
  $script:found
}

# Runs the demo with these arguments and returns the captured window as a bitmap.
function Capture([string[]]$DemoArgs, [string]$Title) {
  $p = Start-Process $Exe -ArgumentList (@('--demo', '--demo-hidden') + $DemoArgs) -PassThru
  try {
    Start-Sleep $Wait
    $h = Find-Window $p.Id $Title
    if ($h -eq [IntPtr]::Zero) { throw "Window '$Title' not found for $($DemoArgs -join ' ')." }
    $r = New-Object SubHubShots+RECT; [SubHubShots]::GetWindowRect($h, [ref]$r) | Out-Null
    $bmp = New-Object Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
    $g = [Drawing.Graphics]::FromImage($bmp); $dc = $g.GetHdc()
    # 2 = PW_RENDERFULLCONTENT: GPU-drawn content of a window that is not on screen.
    [SubHubShots]::PrintWindow($h, $dc, 2) | Out-Null
    $g.ReleaseHdc($dc); $g.Dispose()
    $bmp
  } finally {
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
  }
}

function Save-Crop($Bmp, [int]$X, [int]$Y, [int]$W, [int]$H, [string]$Name) {
  $out = New-Object Drawing.Bitmap $W, $H
  $g = [Drawing.Graphics]::FromImage($out)
  $g.DrawImage($Bmp, (New-Object Drawing.Rectangle 0, 0, $W, $H), (New-Object Drawing.Rectangle $X, $Y, $W, $H), [Drawing.GraphicsUnit]::Pixel)
  $g.Dispose()
  $out.Save((Join-Path $dst "$Name.png"), [Drawing.Imaging.ImageFormat]::Png); $out.Dispose()
  '{0} {1}x{2}' -f $Name, $W, $H
}

# name, demo arguments, window title ('' = popover), invisible frame width (left/right/bottom)
$shots = @(
  @('demo-popover',             @('--demo-page', 'popover'), '', 7),
  @('demo-overview',            @('--demo-page', 'stats', '--demo-height', '860'), 'AI SubHub', 8),
  @('demo-usage-patterns',      @('--demo-page', 'stats', '--demo-open-row', 'windows', '--demo-height', '1000'), 'AI SubHub', 8),
  @('demo-plans',               @('--demo-page', 'stats', '--demo-open-row', 'plans', '--demo-height', '900'), 'AI SubHub', 8),
  @('demo-statistics-usage',    @('--demo-page', 'stats', '--demo-open-row', 'usage', '--demo-height', '1000'), 'AI SubHub', 8),
  @('demo-statistics-limits',   @('--demo-page', 'stats', '--demo-open-row', 'limits', '--demo-height', '890'), 'AI SubHub', 8),
  @('demo-settings-general',    @('--demo-page', 'general', '--demo-height', '700'), 'AI SubHub', 8),
  @('demo-settings-accounts',   @('--demo-page', 'accounts', '--demo-height', '750'), 'AI SubHub', 8),
  @('demo-settings-appearance', @('--demo-page', 'appearance', '--demo-height', '700'), 'AI SubHub', 8),
  @('demo-settings-hub',        @('--demo-page', 'hub', '--demo-height', '700'), 'AI SubHub', 8),
  @('demo-taskbar',             @('--demo-page', 'taskbar'), 'AI SubHub showcase', 8)
)

foreach ($s in $shots) {
  if ($Only -and $Only -notcontains $s[0]) { continue }
  $bmp = Capture $s[1] $s[2]
  $c = $s[3]
  Save-Crop $bmp $c 0 ($bmp.Width - 2 * $c) ($bmp.Height - $c) $s[0]
  $bmp.Dispose()
}

if (-not $Only -or $Only -contains 'demo-more-providers') {
  # The Limits board as tall as the screen allows; the new providers sit below Claude and Codex.
  $bmp = Capture @('--demo-page', 'stats', '--demo-open-row', 'limits', '--demo-height', '2000') 'AI SubHub'
  $h = [Math]::Min(712, $bmp.Height - 730 - 8)
  # x 222 = the 8 px invisible frame + the sidebar.
  Save-Crop $bmp 222 730 992 $h 'demo-more-providers'
  $bmp.Dispose()
}
