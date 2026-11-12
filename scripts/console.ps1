# Real-terminal driver for Fast Harness TUI checks (Windows, pwsh 7).
#
# The TUI runs in a classic console window (started through conhost.exe, so the
# Windows Terminal default does not take it). Input goes in with the console API
# (WriteConsoleInput, the records crossterm reads), the screen text comes out with
# ReadConsoleOutputCharacter, and a PNG with PrintWindow of the console window.
# Each call attaches to the TUI's console for its own run only.
#
# Usage:
#   console.ps1 start -Exe <path\fast-harness.exe> -Cwd <dir> [-Arg <args>]
#       starts `conhost.exe <exe> <args>` in <dir>; prints the TUI's pid
#   console.ps1 type  -Exe <path> -Text "hello"     key down/up per character
#   console.ps1 key   -Exe <path> -Key Enter        Enter, Escape, Tab, Back, Up, Down, Space, F2..F4,
#                                                   or a chord Alt+<letter|digit>, Ctrl+<letter|digit>
#                                                   (Alt+Y, Ctrl+O)
#   console.ps1 text  -Exe <path> [-Out <file.txt>] the visible screen as text
#   console.ps1 shot  -Exe <path> -Out <file.png>   PrintWindow of the console window
#   console.ps1 close -Exe <path>                   kills the TUI (its console closes)
# The TUI is the first process whose path equals -Exe.
param(
    [Parameter(Mandatory)][string]$Action,
    [Parameter(Mandatory)][string]$Exe,
    [string]$Cwd = '.', [string]$Arg = '', [string]$Out, [string]$Text, [string]$Key
)
Add-Type -AssemblyName System.Drawing
if (-not ('FhCon' -as [type])) {
    Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices;
public static class FhCon {
  [DllImport("kernel32.dll")] public static extern bool FreeConsole();
  [DllImport("kernel32.dll")] public static extern bool AttachConsole(uint pid);
  [DllImport("kernel32.dll")] public static extern IntPtr GetConsoleWindow();
  [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern IntPtr CreateFileW(string n, uint a, uint s, IntPtr sa, uint c, uint f, IntPtr t);
  [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
  [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern bool WriteConsoleInputW(IntPtr h, INPUT_RECORD[] r, uint n, out uint w);
  [DllImport("kernel32.dll")] static extern bool GetConsoleScreenBufferInfo(IntPtr h, out INFO i);
  [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern bool ReadConsoleOutputCharacterW(IntPtr h, StringBuilder b, uint n, COORD at, out uint read);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct COORD { public short X, Y; }
  [StructLayout(LayoutKind.Sequential)] public struct SMALL { public short L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct INFO { public COORD Size, Cursor; public ushort Attr; public SMALL Window; public COORD Max; }
  [StructLayout(LayoutKind.Explicit, CharSet = CharSet.Unicode)] public struct INPUT_RECORD {
    [FieldOffset(0)] public ushort EventType;
    [FieldOffset(4)] public int KeyDown; [FieldOffset(8)] public ushort Repeat;
    [FieldOffset(10)] public ushort Vk; [FieldOffset(12)] public ushort Scan;
    [FieldOffset(14)] public char Char; [FieldOffset(16)] public uint Ctrl;
  }
  const uint RW = 0xC0000000, SHARE = 3, OPEN = 3;
  static IntPtr Open(string n) { return CreateFileW(n, RW, SHARE, IntPtr.Zero, OPEN, 0, IntPtr.Zero); }
  public static bool Attach(uint pid) { FreeConsole(); return AttachConsole(pid); }
  public static uint Key(ushort vk, char c) { return Key(vk, c, 0); }
  public static uint Key(ushort vk, char c, uint mods) {
    var r = new INPUT_RECORD[2];
    for (int i = 0; i < 2; i++) { r[i].EventType = 1; r[i].KeyDown = i == 0 ? 1 : 0; r[i].Repeat = 1; r[i].Vk = vk; r[i].Char = c; r[i].Ctrl = mods; }
    IntPtr h = Open("CONIN$"); uint w; WriteConsoleInputW(h, r, 2, out w); CloseHandle(h); return w;
  }
  public static string Screen() {
    IntPtr h = Open("CONOUT$"); INFO i; GetConsoleScreenBufferInfo(h, out i);
    var sb = new StringBuilder();
    for (short y = i.Window.T; y <= i.Window.B; y++) {
      int width = i.Window.R - i.Window.L + 1; var line = new StringBuilder(width); uint read;
      ReadConsoleOutputCharacterW(h, line, (uint)width, new COORD { X = i.Window.L, Y = y }, out read);
      sb.AppendLine(line.ToString(0, (int)read).TrimEnd());
    }
    CloseHandle(h); return sb.ToString();
  }
}
"@
}
$vks = @{ Enter = 0x0D; Escape = 0x1B; Tab = 0x09; Back = 0x08; Up = 0x26; Down = 0x28; F2 = 0x71; F3 = 0x72; F4 = 0x73; Space = 0x20 }

# A chord `Alt+Y` or `Ctrl+O` (one letter or digit): the record a real console
# gives, with LEFT_ALT_PRESSED (2) or LEFT_CTRL_PRESSED (8) and, for Ctrl, the
# control character.
function Send-Chord([string]$mod, [string]$name) {
    $ch = [char]::ToLowerInvariant($name[0])
    $vk = [uint16][char]::ToUpperInvariant($ch)
    if ($mod -eq 'Alt') { return [FhCon]::Key($vk, $ch, 2) }
    $c = if ($ch -match '[a-z]') { [char]([int]$ch - 96) } else { $ch }
    [FhCon]::Key($vk, $c, 8)
}

function Find-Tui([int]$waitMs = 0) {
    $deadline = (Get-Date).AddMilliseconds($waitMs)
    do {
        $p = Get-Process | Where-Object { $_.Path -eq $Exe } | Select-Object -First 1
        if ($p) { return $p }
        Start-Sleep -Milliseconds 200
    } while ((Get-Date) -lt $deadline)
    throw "no process for $Exe"
}

if ($Action -eq 'start') {
    $line = "`"$Exe`" $Arg".Trim()
    # `start` gives the new console its own standard handles; Start-Process would pass this
    # shell's pipes on, and the TUI would see no terminal.
    $null = Start-Process cmd.exe -ArgumentList "/c start `"fh-tui`" /D `"$Cwd`" conhost.exe $line" -WindowStyle Hidden
    $p = Find-Tui 15000
    Start-Sleep -Milliseconds 2000
    return "started pid $($p.Id)"
}
$p = Find-Tui 3000
if ($Action -eq 'close') { $p.Kill(); return "closed $($p.Id)" }
if (-not [FhCon]::Attach([uint32]$p.Id)) { throw "cannot attach to the console of $($p.Id)" }
$res = switch ($Action) {
    'type' { foreach ($c in $Text.ToCharArray()) { [FhCon]::Key(0, $c) | Out-Null; Start-Sleep -Milliseconds 5 }; "typed $($Text.Length) chars" }
    'key' {
        if ($Key -match '^(Alt|Ctrl)\+([A-Za-z0-9])$') { Send-Chord $Matches[1] $Matches[2] | Out-Null; "key $Key"; break }
        $vk = $vks[$Key]; if ($null -eq $vk) { throw "unknown key $Key" }
        $c = switch ($Key) { 'Enter' { [char]13 } 'Escape' { [char]27 } 'Space' { [char]32 } default { [char]0 } }
        [FhCon]::Key($vk, $c) | Out-Null; "key $Key"
    }
    'text' { $s = [FhCon]::Screen(); if ($Out) { [IO.File]::WriteAllText($Out, $s) }; $s }
    'shot' {
        $h = [FhCon]::GetConsoleWindow(); $r = New-Object FhCon+RECT
        [FhCon]::GetClientRect($h, [ref]$r) | Out-Null
        $bmp = New-Object System.Drawing.Bitmap $r.R, $r.B
        $g = [System.Drawing.Graphics]::FromImage($bmp); $dc = $g.GetHdc()
        [FhCon]::PrintWindow($h, $dc, 2) | Out-Null
        $g.ReleaseHdc($dc); $g.Dispose()
        $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
        "saved $Out ($($r.R)x$($r.B))"
    }
    default { throw "unknown action $Action" }
}
# Detach first: output written while attached lands on the TUI's screen.
[FhCon]::FreeConsole() | Out-Null
$res
