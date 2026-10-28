using CommunityToolkit.Mvvm.Input;
using TicketBoard.Models;
using TicketBoard.Services;

namespace TicketBoard.ViewModels;

/// <summary>База знаний из панели: «Спросить базу знаний» (запрос агенту — в буфер) и «В базу знаний» (заявку — в базу сейчас).</summary>
public sealed partial class MainViewModel
{
    /// <summary>В буфер — запрос агенту (KnowledgeAsk): номер, ссылка, где заявка в базе знаний, задача и полный текст заявки,
    /// свежий с сервера, в формате базы (в базе её может ещё не быть, а агенту нужна вся переписка). Вставляется в сессию
    /// агента над базой, в Claude Desktop или в чат — текст один. Ход и итог — строкой под статусом, как у ⟳.</summary>
    [RelayCommand]
    private async Task AskKnowledge()
    {
        if (SelectedTicket is not Ticket t) return;
        if (t.IntraserviceId is not int n) { SyncMessage = "у заявки нет номера"; return; }
        if (_intraservice is not { } client) { SyncMessage = "API не настроен: трей → Настройки…"; return; }
        SyncMessage = "готовлю запрос агенту…";
        var clipboard = ClipboardWatcher.SequenceNumber;   // скопирует пользователь своё, пока готовим, — не затирать
        var url = string.IsNullOrWhiteSpace(t.Url) ? _settings.TicketUrl(n) : t.Url;
        var card = await client.GetTaskAsync(n);
        string message;
        if (card.Task is not { } task) message = card.Error;
        else
        {
            var row = KnowledgeExport.RowOf(task);
            // кто я — для роли в тексте, как в файле базы; не узнали — без роли: запрос нужен и так
            var (me, _) = await MeAsync(client);
            var (text, error) = await KnowledgeExport.BuildAsync(client, row, task, url, CancellationToken.None,
                context: new ExportContext(me, ClosedNames()));
            if (error.Length > 0) message = error;
            else
            {
                // место в базе — с диска (папка может быть сетевой): не в потоке окна
                var dir = _settings.KnowledgePath(App.DataDir);
                var (file, where) = await Task.Run(() => KnowledgeExport.FileInBase(dir, row));
                var request = KnowledgeAsk.Request(n, row.Name, url, file, where, text);
                message = ClipboardWatcher.SequenceNumber != clipboard
                    ? "пока готовил запрос, в буфер скопировали другое — нажмите ещё раз"
                    : ClipboardWatcher.TrySetText(request)
                    ? $"запрос агенту в буфере ({request.Length:N0} знаков) — вставьте в сессию с базой знаний"
                    : "буфер обмена занят другой программой — нажмите ещё раз";
            }
        }
        if (SelectedTicket == t) SyncMessage = message;
    }

    /// <summary>«В базу знаний»: заявку — в базу сейчас, с перепиской целиком и вложениями (по настройке «Вложения в базу
    /// знаний», как у окна «Поиск заявок») — тем же движком, что выгрузка и автовыгрузка: оглавления, таблица, .gitignore.
    /// Агент над базой видит файл, а «Спросить базу знаний» отправляет его к файлу и у заявки, которую автовыгрузка ещё не
    /// взяла. Итог — строкой под кнопками, как у ⟳.</summary>
    [RelayCommand]
    private async Task ExportToBase()
    {
        if (SelectedTicket is not Ticket t) return;
        if (t.IntraserviceId is not int n) { SyncMessage = "у заявки нет номера"; return; }
        if (_intraservice is not { } client) { SyncMessage = "API не настроен: трей → Настройки…"; return; }
        SyncMessage = "выгружаю в базу знаний…";
        string message;
        try { message = await ExportOneAsync(client, n); }
        catch (Exception ex) { message = $"не выгружена: {ex.Message}"; }   // папку базы не создать: сетевой диск без VPN, на её месте файл
        if (SelectedTicket == t) SyncMessage = message;
    }

    /// <summary>Пишет другая выгрузка — что сказать вместо ожидания.</summary>
    internal const string WriterBusy = "сейчас пишет другая выгрузка (окно «Поиск заявок» или автовыгрузка) — нажмите, когда она закончится";

    /// <summary>Одна заявка — в базу: карточка с сервера (месяц файла — по дате создания, версия — по изменению), кто я — для
    /// роли и «моих» вложений (не узнали — не выгружаем: файл без роли считался бы верным), движок. Пишет другая выгрузка — в
    /// очередь не встаём: окно поиска пишет часами, кнопка стояла бы недоступной у всех карточек, а отменить ожидание нечем
    /// (заход автовыгрузки — секунды: нажать ещё раз). ponytail: карточку движок читает ещё раз — он берёт строку списка, а не
    /// карточку; одна лишняя карточка на нажатие.</summary>
    private async Task<string> ExportOneAsync(HttpIntraserviceClient client, int n)
    {
        if (KnowledgeExport.Writer.CurrentCount == 0) return WriterBusy;
        var card = await client.GetTaskAsync(n);
        if (card.Task is not { } task) return card.Error;
        var (me, userError) = await MeAsync(client);
        if (me is null) return userError;
        var row = KnowledgeExport.RowOf(task);
        var dir = _settings.KnowledgePath(App.DataDir);
        var context = new ExportContext(me, ClosedNames(), _settings.KnowledgeAttachments);
        // не в потоке окна: движок пишет файл, оглавление месяца и таблицу синхронно. Без хода (progress): ради одной заявки
        // он не обходит всю папку вложений, чтобы сказать её объём
        var export = await Task.Run(() => KnowledgeExport.ExportRowsAsync(client, new[] { row }, dir, _settings.TicketUrl, null,
            CancellationToken.None, context: context));
        if (!export.Current.Contains(n))
            return export.Errors.TryGetValue(n, out var error) ? error : $"не выгружена: {export.Error}";
        var (file, _) = await Task.Run(() => KnowledgeExport.FileInBase(dir, row));
        // что ещё сказала выгрузка (вложения, таблица не записалась, .gitignore не дописать) — строками ниже
        return (export.Unchanged > 0 ? $"в базе знаний уже свежая: {file}" : $"в базе знаний: {file}")
            + (export.Error.Length > 0 ? "\n" + export.Error : "");
    }
}
