using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using Microsoft.Win32;

namespace TicketBoard.Services;

/// <summary>Что вложениям панели нужно от Windows: открыть файл его программой, показать в Проводнике, спросить, куда
/// сохранить. Решения (что открывать, куда качать) — в MainViewModel.Attachments.cs; здесь только вызовы Windows, в
/// самопроверке — заглушка, которая их записывает.</summary>
internal static class FileShell
{
    /// <summary>Открыть программой, которую Windows назначила этому окончанию; "" — открылся, иначе причина.</summary>
    public static string Open(string path)
    {
        try
        {
            using var _ = Process.Start(new ProcessStartInfo(path) { UseShellExecute = true });
            return "";
        }
        catch (Exception e) when (e is Win32Exception or InvalidOperationException or FileNotFoundException)
        {
            return e.Message;
        }
    }

    /// <summary>Windows считает такое окончание опасным (AssocIsDangerous: свой список и флаг «всегда небезопасно» у типа в
    /// реестре — знает и о типах, что поставили программы) — запасная сеть к Attachments.Runs. Не ответила — опасно.</summary>
    public static bool IsDangerous(string path)
    {
        try { return AssocIsDangerous(Path.GetExtension(path)); }
        catch (Exception e) when (e is DllNotFoundException or EntryPointNotFoundException) { return true; }
    }

    [DllImport("shlwapi.dll", CharSet = CharSet.Unicode)] private static extern bool AssocIsDangerous(string assoc);

    /// <summary>Проводник с выделенным файлом.</summary>
    public static void ShowInFolder(string path)
    {
        try { using var _ = Process.Start("explorer.exe", $"/select,\"{path}\""); }
        catch (Win32Exception) { /* Проводника нет — показывать нечем */ }
    }

    /// <summary>Окно «Сохранить как» с этим именем; false — отменили. Окончание исходного файла — фильтром, чтобы окно не
    /// дописало своё.</summary>
    public static bool TryAskSavePath(string name, out string path)
    {
        var ext = Path.GetExtension(name);
        var dialog = new SaveFileDialog
        {
            Title = "Сохранить вложение",
            FileName = name,
            OverwritePrompt = true,
            AddExtension = false,
            Filter = ext.Length > 0 ? $"Файл {ext}|*{ext}|Все файлы|*.*" : "Все файлы|*.*",
        };
        path = dialog.ShowDialog() == true ? dialog.FileName : "";
        return path.Length > 0;
    }
}
