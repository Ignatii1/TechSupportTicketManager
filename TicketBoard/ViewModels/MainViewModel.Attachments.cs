using System.Collections.ObjectModel;
using System.Globalization;
using System.IO;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using TicketBoard.Models;
using TicketBoard.Services;

namespace TicketBoard.ViewModels;

/// <summary>Вложение в панели: номер и имя, размер (когда узнали) и что с ним сейчас — ход скачивания, ошибка, куда
/// сохранено.</summary>
public sealed partial class AttachmentItem(int ticketId, string? ticketUrl, TicketFile file) : ObservableObject
{
    public int TicketId { get; } = ticketId;
    /// <summary>Ссылка на заявку — в пометку «из интернета» скачанного файла.</summary>
    public string? TicketUrl { get; } = ticketUrl;
    public TicketFile File { get; } = file;
    /// <summary>Имя для показа; сервер не прислал имени — номером.</summary>
    public string Name => File.Name is { Length: > 0 } name ? name : $"файл №{File.Id}";
    /// <summary>«3,4 МБ»; пока не узнали — пусто.</summary>
    [ObservableProperty] private string _size = "";
    /// <summary>Строка под вложением: ход, ошибка, куда сохранено.</summary>
    [ObservableProperty] private string _status = "";
    [ObservableProperty] private bool _isBusy;
    /// <summary>Длина в байтах — для хода скачивания в процентах.</summary>
    internal long? Length { get; set; }
    /// <summary>Размер уже спросили и получили ответ — раз за запуск: не пришёл (сбой, 404) — скажет «Открыть».</summary>
    internal bool SizeAsked { get; set; }
}

/// <summary>Панель: поля карточки и вложения — тихое чтение карточки при открытии, размеры, «Открыть», «Сохранить».</summary>
public sealed partial class MainViewModel
{
    /// <summary>Сколько карточка, прочитанная (или начатая читаться), считается свежей: панель при открытии не читает её снова.
    /// Меняется только самопроверкой.</summary>
    internal static TimeSpan CardFresh { get; set; } = TimeSpan.FromMinutes(2);

    /// <summary>Куда «Открыть» скачивает вложения: %TEMP%\TicketBoard\files\&lt;заявка&gt;\ — кэш, не данные (чистит Windows).
    /// Меняется только самопроверкой.</summary>
    internal static string FilesCache { get; set; } = Path.Combine(Path.GetTempPath(), "TicketBoard", "files");

    /// <summary>Вложения выбранной заявки.</summary>
    public ObservableCollection<AttachmentItem> AttachmentItems { get; } = new();

    public bool HasAttachments => AttachmentItems.Count > 0;

    /// <summary>Вложения, уже показанные за запуск, — по заявке и файлу: вернулись к карточке — размер, ход и ошибка те же,
    /// начатое до ухода скачивание идёт дальше в той же строке.</summary>
    private readonly Dictionary<(int Ticket, TicketFile File), AttachmentItem> _attachmentItems = new();
    private CancellationTokenSource? _panelLookup;

    /// <summary>Вложения карточки — в панель, из карточки доски, без запроса.</summary>
    private void ShowAttachments(Ticket? t)
    {
        AttachmentItems.Clear();
        if (t is { IntraserviceId: int n, Files: { } files })
            foreach (var f in files)
                AttachmentItems.Add(_attachmentItems.TryGetValue((n, f), out var known) ? known : _attachmentItems[(n, f)] = new(n, t.Url, f));
        OnPropertyChanged(nameof(HasAttachments));
    }

    /// <summary>Открыли карточку в панели: после паузы (бег стрелками по карточкам запросов не шлёт) — тихо прочитать карточку,
    /// если она не свежая (CardFresh), и узнать размеры вложений, которых ещё не знаем. Выбрали другую — недоделанное для
    /// прежней отменяется (начатое чтение карточки доходит: это её же данные). pause: false — список вложений сменился у уже
    /// открытой карточки: ждать нечего.</summary>
    private async void LookupPanel(Ticket? t, bool pause = true)
    {
        _panelLookup?.Cancel();
        if (t is not { IntraserviceId: not null } || _intraservice is not { } client) return;
        var cts = _panelLookup = new CancellationTokenSource();
        try
        {
            if (pause) await Task.Delay(400, cts.Token);
            if (t.CardReadAt is not { } read || DateTimeOffset.Now - read >= CardFresh) await ReadCardQuietlyAsync(t, client);
            if (cts.IsCancellationRequested) return;   // выбрали другую — её размеры спросит её же поиск
            await ProbeSizesAsync(client, cts.Token);
        }
        catch (OperationCanceledException) { /* выбрали другую карточку — неважно */ }
    }

    /// <summary>У открытой карточки на сервере что-то сменилось (автообновление увидело новый Changed) — перечитать её сразу:
    /// с новым комментарием часто приходит и скриншот.</summary>
    private void RereadOpenCard(Ticket t)
    {
        t.CardReadAt = null;
        LookupPanel(t, pause: false);
    }

    /// <summary>Размеры вложений панели, которых ещё не знаем: по запросу на файл, по одному; тело не дочитывается — длина из
    /// Content-Length (stopAt: 1). Раз за запуск на вложение.</summary>
    private async Task ProbeSizesAsync(HttpIntraserviceClient client, CancellationToken ct)
    {
        foreach (var a in AttachmentItems.Where(a => !a.SizeAsked && a.Size.Length == 0).ToList())
        {
            ct.ThrowIfCancellationRequested();
            // ушли на другую карточку посреди запроса — исключение отмены: спросим, когда откроют снова
            var (_, length, _) = await client.DownloadFileAsync(a.File.Id, Stream.Null, null, ct, stopAt: 1, htmlExpected: Attachments.IsHtml(a.Name));
            a.SizeAsked = true;
            if (length is long l && a.Size.Length == 0) (a.Length, a.Size) = (l, Attachments.Size(l));
        }
    }

    /// <summary>«Открыть»: вложение — в папку временных и программой Windows. Программу, сценарий, ярлык (Attachments.Runs) не
    /// запускает — показывает в Проводнике: вложение пишет любой, кто может подать заявку.</summary>
    [RelayCommand(AllowConcurrentExecutions = true)]
    private async Task OpenAttachment(AttachmentItem? a)
    {
        if (a is null || a.IsBusy) return;
        if (await FetchAsync(a) is not { } path) return;
        if (Attachments.Runs(path))
        {
            FileShell.ShowInFolder(path);
            a.Status = "программа или сценарий — TicketBoard такие не запускает: файл показан в папке";
        }
        else if (FileShell.Open(path) is { Length: > 0 } error) a.Status = $"не открылся: {error}";
    }

    /// <summary>«Сохранить»: путь спрашивает окно Windows (имя — исходное); скачанный — копией, нет — скачивается прямо туда.
    /// Пометка «из интернета» — и на сохранённом.</summary>
    [RelayCommand(AllowConcurrentExecutions = true)]
    private async Task SaveAttachment(AttachmentItem? a)
    {
        if (a is null || a.IsBusy) return;
        if (_intraservice is not { } client) { a.Status = NoApi; return; }
        if (FileShell.AskSavePath(Attachments.SaveName(a.File.Id, a.File.Name)) is not { } target) return;
        var cached = CachedPath(a);
        a.IsBusy = true;
        try
        {
            var error = "";
            if (File.Exists(cached)) File.Copy(cached, target, overwrite: true);
            else error = await DownloadToAsync(client, a, target);
            if (error.Length > 0) { a.Status = $"не скачалось: {HttpIntraserviceClient.Brief(error)}"; return; }
            var marked = Attachments.MarkFromInternet(target, a.TicketUrl, client.FileUrl(a.File.Id));
            a.Status = $"сохранено: {target}" + (marked || !OperatingSystem.IsWindows() ? "" : " (без пометки «из интернета»: этот диск её не хранит)");
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException) { a.Status = $"не сохранилось: {e.Message}"; }
        finally { a.IsBusy = false; }
    }

    private const string NoApi = "API не настроен: трей → Настройки…";

    private static string CachedPath(AttachmentItem a) =>
        Attachments.DiskPath(Path.Combine(FilesCache, a.TicketId.ToString(CultureInfo.InvariantCulture)), a.File.Id, a.File.Name);

    /// <summary>Вложение в папке временных: лежит — сразу (туда кладётся только скачанное до конца, и номер файла не меняется —
    /// содержимое тоже), нет — скачивается. null — не вышло, причина — в строке вложения.</summary>
    private async Task<string?> FetchAsync(AttachmentItem a)
    {
        if (_intraservice is not { } client) { a.Status = NoApi; return null; }
        var path = CachedPath(a);
        if (File.Exists(path)) { a.Status = ""; return path; }
        a.IsBusy = true;
        try
        {
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            var error = await DownloadToAsync(client, a, path);
            a.Status = error.Length > 0 ? $"не скачалось: {HttpIntraserviceClient.Brief(error)}" : "";
            return error.Length > 0 ? null : path;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException) { a.Status = $"не записалось: {e.Message}"; return null; }
        finally { a.IsBusy = false; }
    }

    /// <summary>Скачать вложение в target: во временный рядом (обрыв не оставит половины файла под настоящим именем), пометка «из
    /// интернета» — на нём, потом подмена. Ход — в строке вложения. Ошибка сервера — текстом ("" — скачано); ошибка диска —
    /// исключением.</summary>
    private static async Task<string> DownloadToAsync(HttpIntraserviceClient client, AttachmentItem a, string target)
    {
        string? tmp = $"{target}.{Random.Shared.Next(0x10000):x4}.tmp";
        try
        {
            // Progress — в поток окна; поздний ход после конца не затирает итог: пишем, только пока занято
            var progress = new Progress<long>(n => { if (a.IsBusy) a.Status = a.Length is long l && l > 0 ? $"скачиваю {n * 100 / l}%" : $"скачиваю {Attachments.Size(n)}"; });
            (long Bytes, long? Length, string Error) r;
            await using (var stream = new FileStream(tmp, FileMode.CreateNew, FileAccess.Write, FileShare.None))
                r = await client.DownloadFileAsync(a.File.Id, stream, ((IProgress<long>)progress).Report, CancellationToken.None,
                    htmlExpected: Attachments.IsHtml(a.Name));
            if (r.Error.Length > 0) return r.Error;
            if (r.Length is long length) (a.Length, a.Size) = (length, Attachments.Size(length));
            Attachments.MarkFromInternet(tmp, a.TicketUrl, client.FileUrl(a.File.Id));
            File.Move(tmp, target, overwrite: true);
            tmp = null;
            return "";
        }
        finally
        {
            if (tmp is not null)
                try { File.Delete(tmp); } catch (Exception e) when (e is IOException or UnauthorizedAccessException) { }
        }
    }
}
