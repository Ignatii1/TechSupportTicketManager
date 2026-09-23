using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;

namespace TicketBoard.Services;

/// <summary>Сообщает об изменении буфера обмена: AddClipboardFormatListener на невидимом message-only окне,
/// как у HotkeyService. Событие — в UI-потоке. Что лежит в буфере, решает подписчик (App → ClaudeRelay).</summary>
public sealed class ClipboardWatcher : IDisposable
{
    private const int WM_CLIPBOARDUPDATE = 0x031D;
    private const uint CF_UNICODETEXT = 13, CF_HDROP = 15;

    [DllImport("user32.dll", SetLastError = true)] private static extern bool AddClipboardFormatListener(IntPtr hwnd);
    [DllImport("user32.dll", SetLastError = true)] private static extern bool RemoveClipboardFormatListener(IntPtr hwnd);
    [DllImport("user32.dll")] private static extern uint GetClipboardSequenceNumber();
    [DllImport("user32.dll")] private static extern bool IsClipboardFormatAvailable(uint format);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern uint RegisterClipboardFormat(string name);

    /// <summary>Форматы, при которых текст из буфера не читаем: просьба менеджеров паролей не заглядывать в их данные;
    /// файлы; Office — он рисует текст по запросу, и на большой таблице это секунды в UI-потоке. Кнопка «Copy»
    /// в браузере кладёт только простой текст.</summary>
    private static readonly uint[] NotOurs = new[] { "ExcludeClipboardContentFromMonitorProcessing", "Rich Text Format",
        "XML Spreadsheet", "Biff8", "Biff12" }.Select(RegisterClipboardFormat).Where(f => f != 0).Append(CF_HDROP).ToArray();

    /// <summary>Номер содержимого буфера: меняется при каждой записи в него.</summary>
    public static uint SequenceNumber => GetClipboardSequenceNumber();

    /// <summary>В буфере простой текст и ничего из NotOurs. Проверка по списку форматов: буфер не открывается,
    /// содержимое не рисуется.</summary>
    public static bool HasPlainText => IsClipboardFormatAvailable(CF_UNICODETEXT) && !NotOurs.Any(IsClipboardFormatAvailable);

    /// <summary>Текст из буфера. Буфер бывает ещё занят тем, кто в него пишет, — несколько коротких попыток.</summary>
    public static string? TryGetText()
    {
        for (var attempt = 0; attempt < 5; attempt++)
        {
            try { return Clipboard.ContainsText() ? Clipboard.GetText() : null; }
            catch (ExternalException) { Thread.Sleep(40); }
        }
        return null;
    }

    /// <summary>Положить текст в буфер; false — буфер так и не освободился.</summary>
    public static bool TrySetText(string text)
    {
        for (var attempt = 0; attempt < 5; attempt++)
        {
            try { Clipboard.SetText(text); return true; }
            catch (ExternalException) { Thread.Sleep(40); }
        }
        return false;
    }

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
