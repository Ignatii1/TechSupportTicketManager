using System.Runtime.InteropServices;
using System.Windows.Input;
using System.Windows.Interop;

namespace TicketBoard.Services;

/// <summary>Глобальный хоткей через RegisterHotKey на невидимом message-only окне.</summary>
public sealed class HotkeyService : IDisposable
{
    private const int WM_HOTKEY = 0x0312;
    private const int HotkeyId = 1;
    private const uint MOD_ALT = 0x1, MOD_CONTROL = 0x2, MOD_SHIFT = 0x4, MOD_WIN = 0x8, MOD_NOREPEAT = 0x4000;

    [DllImport("user32.dll", SetLastError = true)] private static extern bool RegisterHotKey(IntPtr hWnd, int id, uint fsModifiers, uint vk);
    [DllImport("user32.dll", SetLastError = true)] private static extern bool UnregisterHotKey(IntPtr hWnd, int id);

    private readonly HwndSource _source;
    private bool _registered;

    public event Action? Pressed;

    public HotkeyService()
    {
        var p = new HwndSourceParameters("TicketBoard.Hotkeys")
        {
            WindowStyle = 0,
            ParentWindow = new IntPtr(-3) // HWND_MESSAGE
        };
        _source = new HwndSource(p);
        _source.AddHook(Hook);
    }

    public bool TryRegister(string hotkey, out string error)
    {
        error = "";
        Unregister();
        if (!TryParse(hotkey, out var mods, out var vk))
        {
            error = $"Не понял хоткей «{hotkey}». Пример: Ctrl+Shift+Space";
            return false;
        }
        _registered = RegisterHotKey(_source.Handle, HotkeyId, mods | MOD_NOREPEAT, vk);
        if (!_registered) error = $"Хоткей «{hotkey}» занят другой программой (код {Marshal.GetLastWin32Error()})";
        return _registered;
    }

    private static bool TryParse(string text, out uint mods, out uint vk)
    {
        mods = 0; vk = 0;
        foreach (var raw in text.Split('+', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
        {
            switch (raw.ToLowerInvariant())
            {
                case "ctrl": case "control": mods |= MOD_CONTROL; break;
                case "shift": mods |= MOD_SHIFT; break;
                case "alt": mods |= MOD_ALT; break;
                case "win": case "windows": mods |= MOD_WIN; break;
                default:
                    if (!Enum.TryParse<Key>(raw, ignoreCase: true, out var key)) return false;
                    vk = (uint)KeyInterop.VirtualKeyFromKey(key);
                    break;
            }
        }
        return vk != 0;
    }

    private IntPtr Hook(IntPtr hwnd, int msg, IntPtr wParam, IntPtr lParam, ref bool handled)
    {
        if (msg == WM_HOTKEY && wParam.ToInt32() == HotkeyId)
        {
            Pressed?.Invoke();
            handled = true;
        }
        return IntPtr.Zero;
    }

    private void Unregister()
    {
        if (_registered) UnregisterHotKey(_source.Handle, HotkeyId);
        _registered = false;
    }

    public void Dispose()
    {
        Unregister();
        _source.RemoveHook(Hook);
        _source.Dispose();
    }
}
