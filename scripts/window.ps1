# Real-window driver for Fast Harness checks (Windows, pwsh 7).
#
# Usage (coordinates are client pixels of the window, physical, after SetProcessDPIAware):
#   window.ps1 start -Exe <path\fast-harness-gui.exe> -Home <fresh dir> [-Arg <args>]
#       starts the exe with FAST_HARNESS_HOME=<dir> and waits for its window
#   window.ps1 shot  -Exe <path> -Out <file.png>        PrintWindow into a PNG
#   window.ps1 click -Exe <path> -X 10 -Y 20            posted move, left down, left up
#   window.ps1 move  -Exe <path> -X 10 -Y 20            posted mouse move (hover)
#   window.ps1 wheel -Exe <path> -X 10 -Y 20 -Delta 120 posted mouse wheel (+ is up)
#   window.ps1 drag  -Exe <path> -X 10 -Y 20 -X2 10 -Y2 5   posted down, moves with MK_LBUTTON, up
#   window.ps1 key   -Exe <path> -Key Enter             posted key down/up (Enter, Escape, Tab, Back, ...)
#   window.ps1 type  -Exe <path> -Text "hello"          posted key down, WM_CHAR, key up per character (winit drops a bare WM_CHAR)
#   window.ps1 size  -Exe <path>                        prints the client size
#   window.ps1 close -Exe <path>                        posts WM_CLOSE, then kills after 3 s
# The window is the first process whose path equals -Exe, so run a private copy of the exe.
# Posted input cannot move or resize the window. Bitmap work stays in PowerShell: pwsh 7
# cannot compile Add-Type C# that uses System.Drawing.
param(
    [Parameter(Mandatory)][string]$Action,
    [Parameter(Mandatory)][string]$Exe,
    [string]$HomeDir, [string]$Arg = '', [string]$Out, [string]$Text, [string]$Key,
    [int]$X = 0, [int]$Y = 0, [int]$X2 = 0, [int]$Y2 = 0, [int]$Delta = 120
)
Add-Type -AssemblyName System.Drawing
if (-not ('FhWin2' -as [type])) {
    Add-Type @"
using System; using System.Runtime.InteropServices;
public static class FhWin2 {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern short VkKeyScan(char c);
  public static void Char(IntPtr h, char c) {
    int vk = VkKeyScan(c) & 0xFF;
    PostMessage(h, 0x0100, (IntPtr)vk, (IntPtr)1);
    PostMessage(h, 0x0102, (IntPtr)c, (IntPtr)1);
    PostMessage(h, 0x0101, (IntPtr)vk, unchecked((IntPtr)(int)0xC0000001));
  }
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  public static IntPtr L(int x, int y) { return (IntPtr)((y << 16) | (x & 0xFFFF)); }
  public static void Post(IntPtr h, uint m, long w, int x, int y) { PostMessage(h, m, (IntPtr)w, L(x, y)); }
  public static void Wheel(IntPtr h, int x, int y, int delta) {
    var p = new POINT { X = x, Y = y }; ClientToScreen(h, ref p);
    Post(h, 0x0200, 0, x, y);
    PostMessage(h, 0x020A, (IntPtr)((delta & 0xFFFF) << 16), L(p.X, p.Y));
  }
}
"@
}
[FhWin2]::SetProcessDPIAware() | Out-Null
$keys = @{ Enter = 0x0D; Escape = 0x1B; Tab = 0x09; Back = 0x08; Delete = 0x2E; Up = 0x26; Down = 0x28; Home = 0x24; End = 0x23 }

function Find-Window([string]$path, [int]$waitMs = 0) {
    $deadline = (Get-Date).AddMilliseconds($waitMs)
    do {
        $p = Get-Process | Where-Object { $_.Path -eq $path -and $_.MainWindowHandle -ne 0 } | Select-Object -First 1
        if ($p) { return $p }
        Start-Sleep -Milliseconds 200
    } while ((Get-Date) -lt $deadline)
    throw "no window for $path"
}

function Save-Shot([IntPtr]$h, [string]$file) {
    $r = New-Object FhWin2+RECT
    [FhWin2]::GetClientRect($h, [ref]$r) | Out-Null
    $bmp = New-Object System.Drawing.Bitmap $r.R, $r.B
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $dc = $g.GetHdc()
    [FhWin2]::PrintWindow($h, $dc, 3) | Out-Null
    $g.ReleaseHdc($dc); $g.Dispose()
    $bmp.Save($file, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
    "saved $file ($($r.R)x$($r.B))"
}

function Send-Drag([IntPtr]$h) {
    [FhWin2]::Post($h, 0x0200, 0, $X, $Y)
    [FhWin2]::Post($h, 0x0201, 1, $X, $Y)
    foreach ($i in 1..10) {
        Start-Sleep -Milliseconds 30
        [FhWin2]::Post($h, 0x0200, 1, [int]($X + ($X2 - $X) * $i / 10), [int]($Y + ($Y2 - $Y) * $i / 10))
    }
    Start-Sleep -Milliseconds 50
    [FhWin2]::Post($h, 0x0202, 0, $X2, $Y2)
    "drag $X,$Y -> $X2,$Y2"
}

if ($Action -eq 'start') {
    $env:FAST_HARNESS_HOME = $HomeDir
    $sp = @{ FilePath = $Exe; PassThru = $true }
    if ($Arg) { $sp.ArgumentList = $Arg }
    $null = Start-Process @sp
    $p = Find-Window $Exe 15000
    Start-Sleep -Milliseconds 1500
    return "started pid $($p.Id) home $HomeDir"
}
$p = Find-Window $Exe 3000
$h = $p.MainWindowHandle
switch ($Action) {
    'shot' { Save-Shot $h $Out }
    'click' { [FhWin2]::Post($h, 0x0200, 0, $X, $Y); [FhWin2]::Post($h, 0x0201, 1, $X, $Y); [FhWin2]::Post($h, 0x0202, 0, $X, $Y); "click $X,$Y" }
    'move' { [FhWin2]::Post($h, 0x0200, 0, $X, $Y); "move $X,$Y" }
    'wheel' { [FhWin2]::Wheel($h, $X, $Y, $Delta); "wheel $Delta at $X,$Y" }
    'drag' { Send-Drag $h }
    'key' { $vk = $keys[$Key]; [FhWin2]::PostMessage($h, 0x0100, [IntPtr]$vk, [IntPtr]1) | Out-Null; [FhWin2]::PostMessage($h, 0x0101, [IntPtr]$vk, [IntPtr]0xC0000001) | Out-Null; "key $Key" }
    'type' { foreach ($c in $Text.ToCharArray()) { [FhWin2]::Char($h, $c); Start-Sleep -Milliseconds 10 }; "typed $($Text.Length) chars" }
    'size' { $r = New-Object FhWin2+RECT; [FhWin2]::GetClientRect($h, [ref]$r) | Out-Null; "$($r.R)x$($r.B)" }
    'close' { [FhWin2]::PostMessage($h, 0x0010, [IntPtr]0, [IntPtr]0) | Out-Null; Start-Sleep 3; if (-not $p.HasExited) { $p.Kill() }; "closed $($p.Id)" }
    default { throw "unknown action $Action" }
}
