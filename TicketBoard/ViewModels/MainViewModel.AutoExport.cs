using System.IO;
using CommunityToolkit.Mvvm.ComponentModel;
using TicketBoard.Services;

namespace TicketBoard.ViewModels;

/// <summary>Автовыгрузка закрытых заявок в базу знаний (Services/ClosedExport.cs) — после каждого захода автообновления, по
/// его таймеру. Доску не трогает; итог — строкой для подсказки трея, ошибка — в лог один раз.</summary>
public sealed partial class MainViewModel
{
    /// <summary>Итог автовыгрузки для подсказки трея: «база знаний: 12:05, выгружено 3» или «не удалось»; пусто — выключена.</summary>
    [ObservableProperty] private string _autoExportState = "";

    private bool _autoExporting;
    private string _autoExportError = "";   // в лог — только новая ошибка, а не одна и та же каждые 5 минут

    /// <summary>Разобранные «не мои» заявки (номер → изменение): пока не изменились, их история не перечитывается. В памяти,
    /// до смены учётной записи.</summary>
    private Dictionary<int, DateTimeOffset> _notMine = new();

    /// <summary>Папки базы, где *.tmp в .gitignore уже есть или дописана.</summary>
    private readonly HashSet<string> _gitignored = new(StringComparer.OrdinalIgnoreCase);

    /// <summary>Один заход автовыгрузки; по таймеру автообновления, после его захода. internal и с исходом — самопроверка
    /// (AutoExportCheck) зовёт его напрямую.</summary>
    internal async Task<AutoSyncOutcome> AutoExportAsync()
    {
        if (!_settings.AutoExportClosed || _settings.AutoSyncMinutes <= 0 || _autoExporting || _intraservice is not { } client)
            return AutoSyncOutcome.Skipped;
        _autoExporting = true;
        var account = _settings.AccountKey;
        try
        {
            // кто я — тот же, что у автообновления (раз за запуск); закрытые статусы — справочник каждый раз, как там
            if (_me is not { } me)
            {
                var (user, userError) = await client.GetCurrentUserAsync();
                if (!StillExporting(account)) return AutoSyncOutcome.Dropped;
                if (user is null) return AutoExportFailed(userError);
                me = user;
                _me = user;
            }
            var (statuses, statusError) = await client.GetStatusesAsync();
            if (!StillExporting(account)) return AutoSyncOutcome.Dropped;
            if (statusError.Length > 0) return AutoExportFailed(statusError);
            var closedNames = ClosedNames();
            var closed = statuses.Where(s => s.IsFixed || s.IsFinal || closedNames.Contains(s.Name)).Select(s => s.Id).ToList();
            if (closed.Count == 0) return AutoExportFailed("ни один статус не считается закрытым — проверьте ClosedStatusNames в settings.json");

            var dir = _settings.KnowledgePath(App.DataDir);
            KeepTempFilesOutOfGit(dir);
            var mark = _settings.AutoExportSince is { } since
                ? new ExportMark(since, _settings.AutoExportSeen ?? Array.Empty<int>()) : ExportMark.Start(DateTimeOffset.Now);
            var pass = await ClosedExport.RunAsync(client, new ExportScope(closed, me), mark, _notMine, dir, _settings.TicketUrl,
                ClosedExport.Budget, CancellationToken.None);
            // выключили выгрузку или сменили учётную запись, пока шёл заход, — его итог уже ни к чему
            if (!StillExporting(account)) return AutoSyncOutcome.Dropped;
            _notMine = new(pass.NotMine);
            if (_settings.AutoExportSince is null || !pass.Next.SameAs(mark))
            {
                (_settings.AutoExportSince, _settings.AutoExportSeen) = (pass.Next.Since, pass.Next.Seen.ToArray());
                try { _settings.Save(App.DataDir); }
                catch (Exception ex) { Log?.Invoke($"Не удалось сохранить settings.json: {ex}"); }   // в памяти верно — до следующего раза
            }
            if (pass.Error.Length > 0) return AutoExportFailed(pass.Error);
            _autoExportError = "";
            AutoExportState = $"база знаний: {DateTime.Now:HH:mm}, " + (pass.Exported > 0 ? $"выгружено {pass.Exported}" : "новых нет")
                + (pass.More ? ", догоняю" : "");
            return AutoSyncOutcome.Done;
        }
        finally
        {
            _autoExporting = false;
        }
    }

    /// <summary>Итог захода ещё нужен: выгрузка включена, учётная запись та же, автообновление не выключено.</summary>
    private bool StillExporting(string account) =>
        _intraservice is not null && _settings.AccountKey == account && _settings.AutoSyncMinutes > 0 && _settings.AutoExportClosed;

    private AutoSyncOutcome AutoExportFailed(string error)
    {
        AutoExportState = $"база знаний: не удалось ({DateTime.Now:HH:mm}) — подробности в errors.log";
        if (error != _autoExportError)   // сервер лежит — одна запись в логе, а не каждые 5 минут
        {
            _autoExportError = error;
            Log?.Invoke($"Автовыгрузка: {error}");
        }
        return AutoSyncOutcome.Failed;
    }

    /// <summary>Папка базы — корень git-репозитория (его отправляет Obsidian Git): временные файлы записи (*.tmp) — в
    /// .gitignore, иначе коммит раз в минуту может поймать недописанный файл. Только дописываем, и только если строки нет.</summary>
    private void KeepTempFilesOutOfGit(string dir)
    {
        if (_gitignored.Contains(dir) || !Directory.Exists(Path.Combine(dir, ".git"))) return;
        var path = Path.Combine(dir, ".gitignore");
        try
        {
            var text = File.Exists(path) ? File.ReadAllText(path) : "";
            if (!text.Split('\n').Any(line => line.Trim() == "*.tmp"))
                File.AppendAllText(path, (text.Length > 0 && !text.EndsWith('\n') ? "\n" : "") + "*.tmp\n");
            _gitignored.Add(dir);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            Log?.Invoke($"Автовыгрузка: не удалось дописать *.tmp в {path}: {ex.Message}");
        }
    }
}
