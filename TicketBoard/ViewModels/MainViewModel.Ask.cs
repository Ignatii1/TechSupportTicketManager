using System.IO;
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
        var url = string.IsNullOrWhiteSpace(t.Url) ? _settings.TicketUrl(n) : t.Url;
        var (text, row, error) = await KnowledgeExport.BuildOneAsync(client, n, url, CancellationToken.None);
        string message;
        if (row is null) message = error;
        else
        {
            // место в базе — с диска (папка может быть сетевой): не в потоке окна; не вышло — как будто файла нет
            var dir = _settings.KnowledgePath(App.DataDir);
            var (file, inBase) = await Task.Run(() =>
            {
                try { return KnowledgeExport.FileInBase(dir, row); }
                catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or ArgumentException or NotSupportedException)
                { return ($"{KnowledgeExport.TicketsFolder}/{KnowledgeExport.ShardOf(row.Created)}/{row.Id}.md", false); }
            });
            var request = KnowledgeAsk.Request(n, row.Name, url, file, inBase, text);
            message = ClipboardWatcher.TrySetText(request)
                ? $"запрос агенту в буфере ({request.Length:N0} знаков) — вставьте в сессию с базой знаний"
                : "буфер обмена занят другой программой — нажмите ещё раз";
        }
        if (SelectedTicket == t) SyncMessage = message;
    }
}
