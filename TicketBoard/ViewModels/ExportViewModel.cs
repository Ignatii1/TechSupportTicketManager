using System.Diagnostics;
using System.Globalization;
using System.IO;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using TicketBoard.Services;

namespace TicketBoard.ViewModels;

/// <summary>Окно «Выгрузка для базы знаний»: отбор (мои, слова, статус, период, сколько), папка, ход и итог. Сама
/// выгрузка — Services/KnowledgeExport.cs, в фоне; отбор и папка запоминаются в settings.json, так что следующая
/// выгрузка — одной кнопкой (а изменившиеся заявки она перепишет, остальные пропустит).</summary>
public sealed partial class ExportViewModel : ObservableObject
{
    private readonly AppSettings _settings;
    private HttpIntraserviceClient? _intraservice;   // null — API не настроен
    private CancellationTokenSource? _run;
    private bool _stopping;   // нажали «Остановить» — ход больше не показываем, чтобы не затереть «Останавливаю…»

    [ObservableProperty] private bool _mine;
    [ObservableProperty] private string _words = "";
    [ObservableProperty] private ExportStatus _status;
    [ObservableProperty] private string _days = "";
    [ObservableProperty] private string _limit = "";
    [ObservableProperty] private string _folder = "";
    /// <summary>Ход выгрузки или её итог — выделяется и копируется.</summary>
    [ObservableProperty] private string _message = "";
    [ObservableProperty, NotifyCanExecuteChangedFor(nameof(ExportCommand)), NotifyCanExecuteChangedFor(nameof(StopCommand))]
    private bool _isBusy;

    public ExportViewModel(AppSettings settings, HttpIntraserviceClient? intraservice)
    {
        _settings = settings;
        _intraservice = intraservice;
        var f = settings.LastExport ?? new();
        (Mine, Words, Status) = (f.Mine, f.Words ?? "", f.Status);
        Days = f.Days.ToString(CultureInfo.InvariantCulture);
        Limit = f.Limit.ToString(CultureInfo.InvariantCulture);
        Folder = settings.KnowledgePath(App.DataDir);
    }

    /// <summary>Настройки сохранены — новый клиент API. Идущая выгрузка доходит старым: она уже на полпути.</summary>
    public void ApplySettings(HttpIntraserviceClient? intraservice) => _intraservice = intraservice;

    private bool CanExport() => !IsBusy;
    private bool CanStop() => IsBusy;

    [RelayCommand(CanExecute = nameof(CanExport))]
    private async Task Export()
    {
        if (_intraservice is not { } client) { Message = "API не настроен: трей → Настройки…"; return; }
        if (!int.TryParse(Days.Trim(), NumberStyles.None, CultureInfo.InvariantCulture, out var days) || days > KnowledgeExport.MaxDays)
        { Message = $"«За последние, дней» — от 0 (за всё время) до {KnowledgeExport.MaxDays}"; return; }
        if (!int.TryParse(Limit.Trim(), NumberStyles.None, CultureInfo.InvariantCulture, out var limit) || limit is < 1 or > KnowledgeExport.MaxLimit)
        { Message = $"«Не больше, заявок» — от 1 до {KnowledgeExport.MaxLimit}"; return; }
        var dir = Folder.Trim();
        if (!Path.IsPathFullyQualified(dir)) { Message = "Папка — полный путь, например D:\\Obsidian\\База\\Заявки"; return; }
        var filter = new ExportFilter(Mine, Words.Trim(), Status, days, limit);   // без «моих» и слов откажет сама выгрузка

        // отбор и папку — запомнить; папка по умолчанию хранится пустой строкой: переедет вместе с exe
        _settings.LastExport = filter;
        _settings.KnowledgeDir = string.Equals(Path.GetFullPath(dir), Path.GetFullPath(Path.Combine(App.DataDir, "knowledge")),
            StringComparison.OrdinalIgnoreCase) ? "" : dir;
        try { _settings.Save(App.DataDir); }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException) { /* не запомнится — выгрузке не мешает */ }

        IsBusy = true;
        _stopping = false;
        Message = "Начинаю…";
        var run = _run = new CancellationTokenSource();
        // ход приходит из фона; после итога и после «Остановить» запоздавший «Переписка: N из M» не затрёт сообщение
        var progress = new Progress<string>(m => { if (IsBusy && !_stopping) Message = m; });
        var closed = _settings.ClosedNames();
        Func<int, string> url = _settings.TicketUrl;
        try
        {
            Directory.CreateDirectory(dir);
            var r = await Task.Run(() => KnowledgeExport.RunAsync(client, filter, dir, closed, url, progress, run.Token));
            IsBusy = false;
            Message = Summary(r);
        }
        catch (Exception ex)
        {
            IsBusy = false;
            Message = ex is IOException or UnauthorizedAccessException
                ? $"Не удалось записать в папку {dir}:\n{ex.Message}"
                : $"Выгрузка сорвалась: {ex.Message}";
        }
        finally
        {
            IsBusy = false;
            _run = null;
            run.Dispose();
        }
    }

    [RelayCommand(CanExecute = nameof(CanStop))]
    private void Stop()
    {
        _stopping = true;
        _run?.Cancel();
        Message = "Останавливаю — дописываю начатое…";
    }

    [RelayCommand]
    private void OpenFolder()
    {
        var dir = Folder.Trim();
        if (!Path.IsPathFullyQualified(dir)) return;
        try
        {
            Directory.CreateDirectory(dir);
            Process.Start(new ProcessStartInfo(dir) { UseShellExecute = true });
        }
        catch (Exception ex) { Message = $"Не открыть папку: {ex.Message}"; }
    }

    private static string Summary(ExportResult r) =>
        r.Found == 0 ? (r.Error.Length > 0 ? r.Error : "Под отбор не попало ни одной заявки — проверьте слова, статус и период.")
        : $"Готово. По отбору — {r.Found}: новых файлов {r.Created}, обновлено {r.Updated}, без изменений {r.Unchanged}."
          + (r.Failed > 0 ? $"\nНе прочитались: {r.Failed} — выгрузите ещё раз, дочитаются. Первая ошибка — {r.FirstError}" : "")
          + (r.Error.Length > 0 ? $"\n{r.Error}" : "");
}
