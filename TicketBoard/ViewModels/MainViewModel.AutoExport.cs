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

    /// <summary>Заявки, на которых заходы спотыкаются, — чтобы одна сломанная не держала выгрузку вечно (ExportFailures).</summary>
    private ExportFailures _failures = ExportFailures.None;

    /// <summary>Поколение выгрузки: растёт, когда отметку забывают (ForgetExport), — заход, начатый раньше, итог не пишет
    /// (выключили и тут же включили снова, пока он шёл).</summary>
    private int _exportEpoch;

    /// <summary>Папки базы, где *.tmp в .gitignore уже есть или дописана; и те, о чьей неудаче лог уже знает (раз за запуск,
    /// а пробовать — каждым заходом: файл бывает занят на миг). Трогает их только заход (в пуле, по одному).</summary>
    private readonly HashSet<string> _gitignored = new(StringComparer.OrdinalIgnoreCase), _gitignoreLogged = new(StringComparer.OrdinalIgnoreCase);

    /// <summary>Один заход автовыгрузки; по таймеру автообновления, после его захода. internal и с исходом — самопроверка
    /// (AutoExportCheck) зовёт его напрямую.</summary>
    internal async Task<AutoSyncOutcome> AutoExportAsync()
    {
        if (!_settings.AutoExportClosed || _settings.AutoSyncMinutes <= 0 || _autoExporting || _intraservice is not { } client)
            return AutoSyncOutcome.Skipped;
        _autoExporting = true;
        // папку выгрузки сменили — отметка была про прежнюю: заявки до неё лежат там, а не в новой, — заново, с недели назад
        var dir = _settings.KnowledgePath(App.DataDir);
        if (_settings.AutoExportDir is { } markDir && !SamePath(markDir, dir)) ForgetExport();
        var (account, epoch) = (_settings.AccountKey, _exportEpoch);
        try
        {
            // кто я — тот же, что у автообновления (раз за запуск); закрытые статусы — справочник каждый раз, как там
            if (_me is not { } me)
            {
                var (user, userError) = await client.GetCurrentUserAsync();
                if (!StillExporting(account, epoch, dir)) return AutoSyncOutcome.Dropped;
                if (user is null) return AutoExportFailed(userError);
                me = user;
                _me = user;
            }
            var (statuses, statusError) = await client.GetStatusesAsync();
            if (!StillExporting(account, epoch, dir)) return AutoSyncOutcome.Dropped;
            if (statusError.Length > 0) return AutoExportFailed(statusError);
            // пустой справочник и «ни одного закрытого» — разные беды: первая на сервере, вторую чинит сам пользователь
            if (statuses.Count == 0) return AutoExportFailed("сервер не вернул ни одного статуса заявок");
            var closedNames = ClosedNames();
            var closed = statuses.Where(s => s.ClosedBy(closedNames)).Select(s => s.Id).ToList();
            if (closed.Count == 0) return AutoExportFailed("ни один статус не считается закрытым — проверьте ClosedStatusNames в settings.json");

            var mark = _settings.AutoExportSince is { } since
                ? new ExportMark(since, _settings.AutoExportSeen ?? Array.Empty<int>()) : ExportMark.Start(DateTimeOffset.Now);
            var (notMine, failures, ticketUrl) = (_notMine, _failures, (Func<int, string>)_settings.TicketUrl);
            // заход — не в потоке окна: выгрузка пишет файлы и оглавления (в большом месяце — тысячи файлов) синхронно, а
            // проверка папки на сетевом диске, которого нет, может висеть секунды
            var pass = await Task.Run(() =>
            {
                KeepTempFilesOutOfGit(dir);
                return ClosedExport.RunAsync(client, new ExportScope(closed, me), mark, notMine, failures, dir,
                    ticketUrl, ClosedExport.Budget, DateTimeOffset.Now, CancellationToken.None);
            });
            // выключили выгрузку, сменили учётную запись или папку, пока шёл заход, — его итог уже ни к чему
            if (!StillExporting(account, epoch, dir)) return AutoSyncOutcome.Dropped;
            _notMine = new(pass.NotMine);
            _failures = pass.Failures;
            foreach (var f in pass.GivenUp)
                Log?.Invoke($"Автовыгрузка: заявка #{f.Id} пропущена — не далась {f.Times} заходов подряд, с {f.First:dd.MM HH:mm}. "
                    + $"Если она ваша — выгрузите её из окна «Поиск заявок»; изменится — автовыгрузка попробует снова. Последняя ошибка: {f.Error}");
            if (_settings.AutoExportSince is null || !pass.Next.SameAs(mark))
            {
                (_settings.AutoExportSince, _settings.AutoExportSeen, _settings.AutoExportDir) = (pass.Next.Since, pass.Next.Seen.ToArray(), dir);
                try { _settings.Save(App.DataDir); }
                catch (Exception ex) { Log?.Invoke($"Не удалось сохранить settings.json: {ex}"); }   // в памяти верно — до следующего раза
            }
            if (pass.Error.Length > 0) return AutoExportFailed(pass.Error);
            _autoExportError = "";
            AutoExportState = $"база знаний: {DateTime.Now:HH:mm}, " + (pass.Exported > 0 ? $"выгружено {pass.Exported}" : "новых нет")
                + (pass.GivenUp.Count > 0 ? $", пропущено {pass.GivenUp.Count} (errors.log)" : "") + (pass.More ? ", догоняю" : "");
            return AutoSyncOutcome.Done;
        }
        catch (Exception ex)
        {
            // по таймеру: ни окон, ни падений (папку выгрузки не создать — диска нет, на её месте файл) — подсказка трея и лог
            return StillExporting(account, epoch, dir) ? AutoExportFailed(ex.ToString()) : AutoSyncOutcome.Dropped;
        }
        finally
        {
            _autoExporting = false;
        }
    }

    /// <summary>Забыть, докуда дошла выгрузка (сменили учётную запись, выключили выгрузку): следующий заход начнёт как
    /// первый — с недели назад. Отметку на диск записывает тот, кто зовёт.</summary>
    private void ForgetExport()
    {
        (_settings.AutoExportSince, _settings.AutoExportSeen, _settings.AutoExportDir) = (null, null, null);
        _notMine = new();
        _failures = ExportFailures.None;
        AutoExportState = "";
        _exportEpoch++;
    }

    /// <summary>Итог захода ещё нужен: выгрузка включена и с тех пор не забывалась, учётная запись и папка те же,
    /// автообновление не выключено.</summary>
    private bool StillExporting(string account, int epoch, string dir) =>
        _intraservice is not null && _settings.AccountKey == account && _settings.AutoSyncMinutes > 0 && _settings.AutoExportClosed
        && _exportEpoch == epoch && SamePath(_settings.KnowledgePath(App.DataDir), dir);

    /// <summary>Та же папка: без учёта регистра и косой черты в конце (как её вписали в настройках).</summary>
    private static bool SamePath(string a, string b) =>
        string.Equals(a.Trim().TrimEnd('\\', '/'), b.Trim().TrimEnd('\\', '/'), StringComparison.OrdinalIgnoreCase);

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

    /// <summary>Папка базы — в git-репозитории (его отправляет Obsidian Git; она сама корень или его подпапка): временные
    /// файлы записи (*.tmp) — в .gitignore самой папки (действует на неё и вложенные), иначе коммит раз в минуту может
    /// поймать недописанный файл. Только дописываем, и только если строки нет.</summary>
    private void KeepTempFilesOutOfGit(string dir)
    {
        if (_gitignored.Contains(dir) || !InGitRepository(dir)) return;
        var path = Path.Combine(dir, ".gitignore");
        try
        {
            Directory.CreateDirectory(dir);   // папки ещё нет (новая подпапка хранилища) — строка нужна до первого файла
            var text = File.Exists(path) ? File.ReadAllText(path) : "";
            if (!text.Split('\n').Any(line => line.Trim() == "*.tmp"))
                File.AppendAllText(path, (text.Length > 0 && !text.EndsWith('\n') ? "\n" : "") + "*.tmp\n");
            _gitignored.Add(dir);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            // в лог — раз за запуск, а пробуем каждым заходом: занятый на миг файл пройдёт сам
            if (_gitignoreLogged.Add(dir))
                Log?.Invoke($"Автовыгрузка: не удалось дописать *.tmp в {path} (не пройдёт — допишите сами): {ex.Message}");
        }
    }

    /// <summary>Папка или одна из папок над ней — репозиторий: в ней .git, папка или файл (рабочее дерево, подмодуль).</summary>
    private static bool InGitRepository(string dir)
    {
        for (var d = new DirectoryInfo(dir); d is not null; d = d.Parent)
            if (Directory.Exists(Path.Combine(d.FullName, ".git")) || File.Exists(Path.Combine(d.FullName, ".git")))
                return true;
        return false;
    }
}
