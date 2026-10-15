using CommunityToolkit.Mvvm.Input;
using TicketBoard.Models;
using TicketBoard.Services;

namespace TicketBoard.ViewModels;

/// <summary>«Спросить базу знаний»: запрос агенту над базой — в буфер обмена.</summary>
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
}
