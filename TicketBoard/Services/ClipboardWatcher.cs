using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Windows.Interop;

namespace TicketBoard.Services;

/// <summary>Сообщает об изменении буфера обмена: AddClipboardFormatListener на невидимом message-only окне,
/// как у HotkeyService. Событие — в UI-потоке. Что лежит в буфере, решает подписчик (App → ClaudeRelay).</summary>
public sealed class ClipboardWatcher : IDisposable
{
    private const int WM_CLIPBOARDUPDATE = 0x031D;

    [DllImport("user32.dll", SetLastError = true)] private static extern bool AddClipboardFormatListener(IntPtr hwnd);
    [DllImport("user32.dll", SetLastError = true)] private static extern bool RemoveClipboardFormatListener(IntPtr hwnd);

    private readonly HwndSource _source;

    public event Action? Changed;

    public ClipboardWatcher()
    {
        _source = new HwndSource(new HwndSourceParameters("TicketBoard.Clipboard")
        {
            WindowStyle = 0,
            ParentWindow = new IntPtr(-3) // HWND_MESSAGE
        });
        _source.AddHook(Hook);
        if (!AddClipboardFormatListener(_source.Handle))
        {
            var error = new Win32Exception(Marshal.GetLastWin32Error());
            _source.Dispose();
            throw error;
        }
    }

    private IntPtr Hook(IntPtr hwnd, int msg, IntPtr wParam, IntPtr lParam, ref bool handled)
    {
        if (msg == WM_CLIPBOARDUPDATE)
        {
            Changed?.Invoke();
            handled = true;
        }
        return IntPtr.Zero;
    }

    public void Dispose()
    {
        RemoveClipboardFormatListener(_source.Handle);
        _source.Dispose();
    }
}
