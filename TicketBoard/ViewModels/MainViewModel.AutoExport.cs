using System.Diagnostics;
using System.IO;
using CommunityToolkit.Mvvm.ComponentModel;
using TicketBoard.Services;

namespace TicketBoard.ViewModels;

/// <summary>Автовыгрузка в базу знаний (Services/ClosedExport.cs) — мои закрытые после каждого захода автообновления, по его
/// таймеру; зеркало (все закрытые или все заявки) — раз в час, а отстало — тоже каждым. Доску не трогает; итог — строкой для
/// подсказки трея, ошибка — в лог один раз.</summary>
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

    /// <summary>Зеркало отдыхает: когда начался его последний заход, догнавший без ошибок, и для какого охвата. null — не
    /// отдыхает (порция кончилась — «догоняю», заход не удался, отметку забыли, программу перезапустили).</summary>
    private (ExportReach Reach, DateTimeOffset At)? _exportRest;

    /// <summary>Останов текущего захода (свой у каждого): ForgetExport и ApplySettings отменяют — заход бросает работу (и
    /// ожидание очереди за долгой выгрузкой из окна поиска) и итог не пишет, даже если выгрузку тут же включили снова.</summary>
    private CancellationTokenSource _exportStop = new();

    /// <summary>Ошибки .gitignore (каждая — с путём к нему), что лог уже знает: каждая — раз за запуск, а пробовать — каждым
    /// заходом. Трогает их только заход (по одному).</summary>
    private readonly HashSet<string> _gitignoreLogged = new(StringComparer.OrdinalIgnoreCase);

    /// <summary>Один заход автовыгрузки; по таймеру автообновления, после его захода. internal и с исходом — самопроверка
    /// (AutoExportCheck) зовёт его напрямую.</summary>
    internal async Task<AutoSyncOutcome> AutoExportAsync()
    {
        if (!ExportCanRun || _autoExporting || _intraservice is not { } client)
            return AutoSyncOutcome.Skipped;
        _autoExporting = true;
        var (dir, folder, account, stop) = (_settings.KnowledgePath(App.DataDir), FolderSetting, _settings.AccountKey, CancellationToken.None);
        var reach = _settings.AutoExportReach;
        try
        {
            // папку выгрузки сменили — отметка была про прежнюю: заявки до неё лежат там, а не в новой, — заново, с недели
            // назад. Папка — как в настройках (пусто — «knowledge» рядом с exe): перенесли программу — папка та же
            if (_settings.AutoExportDir is { } markDir && !AppSettings.SameFolder(markDir, folder, App.DataDir)) ForgetExport();
            // охват шире, чем у отметки (мои → все закрытые → все): узкий заход пропускал то, что нужно широкому, — тоже заново
            if (_settings.AutoExportSince is not null && !ClosedExport.MarkServes(_settings.AutoExportMarkReach, reach)) ForgetExport();
            // зеркало после догнавшего захода отдыхает час; мои — каждым заходом автообновления
            var started = DateTimeOffset.Now;   // отдых — от начала захода: от конца он пропускал бы ближайший тик таймера
            if (!ClosedExport.Due(reach, _exportRest, started)) return AutoSyncOutcome.Skipped;
            var previous = _exportStop;   // прежний заход кончился (заходы не перекрываются) — его останов больше не нужен
            _exportStop = new();
            previous.Dispose();
            stop = _exportStop.Token;
            // отметка — первым делом: в первый заход (включили, сменили папку или учётную запись) «неделя назад» записывается
            // сразу, а не после первого удачного: иначе, пока заходы не удаются (папки нет), окно ползло бы вперёд
            var mark = _settings.AutoExportSince is { } since
                ? new ExportMark(since, _settings.AutoExportSeen ?? Array.Empty<int>()) : KeepMark(ExportMark.Start(DateTimeOffset.Now), folder, reach);
            // папка — до любых запросов, одним шагом с .gitignore (строка *.tmp — до первого файла), в пуле: недоступный сетевой
            // диск может висеть секунды. Папки нет и не создать (диск без VPN) — сразу «не удалось», сервер не спрошен
            await Task.Run(() =>
            {
                Directory.CreateDirectory(dir);
                KeepTempFilesOutOfGit(dir);
            });
            if (!StillExporting(account, folder, stop)) return AutoSyncOutcome.Dropped;
            // кто я — тот же, что у автообновления (раз за запуск)
            var (me, userError) = await MeAsync(client);
            if (!StillExporting(account, folder, stop)) return AutoSyncOutcome.Dropped;
            if (me is null) return AutoExportFailed(userError);
            // закрытые статусы — справочник каждый раз, как у автообновления; «всем заявкам» он ни к чему
            var closedNames = ClosedNames();
            IReadOnlyCollection<int> closed = Array.Empty<int>();
            if (reach != ExportReach.All)
            {
                var (statuses, statusError) = await client.GetStatusesAsync();
                if (!StillExporting(account, folder, stop)) return AutoSyncOutcome.Dropped;
                if (statusError.Length > 0) return AutoExportFailed(statusError);
                // пустой справочник и «ни одного закрытого» — разные беды: первая на сервере, вторую чинит сам пользователь
                if (statuses.Count == 0) return AutoExportFailed("сервер не вернул ни одного статуса заявок");
                closed = statuses.Where(s => s.ClosedBy(closedNames)).Select(s => s.Id).ToList();
                if (closed.Count == 0) return AutoExportFailed("ни один статус не считается закрытым — проверьте ClosedStatusNames в settings.json");
            }

            var (notMine, failures, ticketUrl, attachments) = (_notMine, _failures, (Func<int, string>)_settings.TicketUrl, _settings.KnowledgeAttachments);
            var clock = Stopwatch.StartNew();
            // заход — не в потоке окна: выгрузка пишет файлы и оглавления (в большом месяце — тысячи файлов) синхронно
            var pass = await Task.Run(() => ClosedExport.RunAsync(client, new ExportScope(closed, me, closedNames, attachments, reach), mark, notMine,
                failures, dir, ticketUrl, ClosedExport.Budget, DateTimeOffset.Now, stop));
            // выключили выгрузку, сменили учётную запись или папку, пока шёл заход, — его итог уже ни к чему
            if (!StillExporting(account, folder, stop)) return AutoSyncOutcome.Dropped;
            // зеркало догнало без ошибок — следующий через час; порция кончилась или сбой (сервер лежал, заявка сломана) — следующим
            // заходом автообновления, как у моих: иначе одна сломанная заявка замедлила бы догон до раза в час на часы
            _exportRest = reach != ExportReach.Mine && !pass.More && pass.Error.Length == 0 ? (reach, started) : null;
            _notMine = new(pass.NotMine);
            _failures = pass.Failures;
            foreach (var f in pass.GivenUp)
                Log?.Invoke($"Автовыгрузка: заявка #{f.Id} пропущена — не далась {f.Times} заходов подряд, с {f.First:dd.MM HH:mm}. "
                    + $"Если она ваша — выгрузите её из окна «Поиск заявок»; изменится — автовыгрузка попробует снова. Последняя ошибка: {f.Error}");
            // отметка — с охватом этого захода: сменили охват посреди него — следующий решит, годится ли она (MarkServes)
            if (!pass.Next.SameAs(mark)) KeepMark(pass.Next, folder, reach);
            if (pass.Error.Length > 0) return AutoExportFailed(pass.Error);
            _autoExportError = "";
            AutoExportState = $"база знаний: {DateTime.Now:HH:mm}, "
                + (pass.Exported > 0 ? $"выгружено {pass.Exported}{ClosedExport.Took(clock.Elapsed - pass.Waited)}" : "новых нет")
                + (pass.GivenUp.Count > 0 ? $", пропущено {pass.GivenUp.Count} (errors.log)" : "") + (pass.More ? ", догоняю" : "");
            return AutoSyncOutcome.Done;
        }
        catch (Exception ex)
        {
            // по таймеру: ни окон, ни падений (папку выгрузки не создать — диска нет, на её месте файл) — подсказка трея и лог
            return StillExporting(account, folder, stop) ? AutoExportFailed(ex.ToString()) : AutoSyncOutcome.Dropped;
        }
        finally
        {
            _autoExporting = false;
        }
    }

    /// <summary>Отметку — в настройки и settings.json, вместе с папкой и охватом, к которым она относится.</summary>
    private ExportMark KeepMark(ExportMark mark, string folder, ExportReach reach)
    {
        (_settings.AutoExportSince, _settings.AutoExportSeen, _settings.AutoExportDir, _settings.AutoExportMarkReach) =
            (mark.Since, mark.Seen.ToArray(), folder, reach);
        try { _settings.Save(App.DataDir); }
        catch (Exception ex) { Log?.Invoke($"Не удалось сохранить settings.json: {ex}"); }   // в памяти верно — до следующего раза
        return mark;
    }

    /// <summary>Забыть, докуда дошла выгрузка (сменили учётную запись, папку или охват, выключили выгрузку): следующий заход
    /// начнёт как первый — с недели назад и сразу, без отдыха. Отметку на диск записывает тот, кто зовёт.</summary>
    private void ForgetExport()
    {
        (_settings.AutoExportSince, _settings.AutoExportSeen, _settings.AutoExportDir, _settings.AutoExportMarkReach) = (null, null, null, null);
        _exportRest = null;
        _notMine = new();
        _failures = ExportFailures.None;
        AutoExportState = "";
        StopExportPass();
    }

    /// <summary>Заход, что ещё идёт или ждёт очереди писать, — бросает (его итог уже не нужен: забыли отметку, выгрузку
    /// выключили, автообновление поставили на 0). Остановленный так и остаётся остановленным: включили снова, пока он не
    /// вернулся, — итог всё равно не пишется (StillExporting). Новый заход — со своим остановом.</summary>
    private void StopExportPass() => _exportStop.Cancel();

    /// <summary>Итог захода ещё нужен: его не останавливали (ни отметку не забывали, ни выгрузку не выключали), выгрузка
    /// включена, учётная запись и папка те же, автообновление не выключено.</summary>
    private bool StillExporting(string account, string folder, CancellationToken stop) =>
        !stop.IsCancellationRequested && ExportCanRun && _settings.AccountKey == account && AppSettings.SameFolder(FolderSetting, folder, App.DataDir);

    /// <summary>Автовыгрузка может идти: включена, автообновление не на нуле (её заход идёт после его захода), сервер задан.</summary>
    private bool ExportCanRun => _settings.AutoExportClosed && _settings.AutoSyncMinutes > 0 && _intraservice is not null;

    /// <summary>Папка выгрузки, как она в настройках: пусто — «knowledge» рядом с exe (едет вместе с программой).</summary>
    private string FolderSetting => _settings.KnowledgeDir.Trim();


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

    /// <summary>*.tmp — в .gitignore папки базы, если она в git-репозитории (правило — KnowledgeExport.IgnoreTempsInGit;
    /// проверяется каждым заходом: .gitignore могли удалить). Не вышло — в лог по разу на каждую причину, а пробуем каждым
    /// заходом: занятый на миг файл пройдёт сам, и замок, что то держит файл, то нет, не пишет в лог строку за строкой;
    /// сломался потом по другой причине — в лог снова.</summary>
    private void KeepTempFilesOutOfGit(string dir)
    {
        if (KnowledgeExport.IgnoreTempsInGit(dir) is { Length: > 0 } error && _gitignoreLogged.Add(error))
            Log?.Invoke($"Автовыгрузка: {error}. Пробую каждым заходом; так и не выйдет — допишите строку *.tmp в этот .gitignore сами.");
    }
}
