using CommunityToolkit.Mvvm.Input;
using TicketBoard.Models;
using TicketBoard.Services;

namespace TicketBoard.ViewModels;

/// <summary>Черновик комментария заявки: текст, вид (внутренний) и итог последней отправки. В памяти, у каждой заявки свой:
/// текст одной заявки не уйдёт в другую. На диск не пишется (ponytail: закрыли приложение — недописанное пропало).</summary>
public sealed record CommentDraft(string Text, bool Internal, string Result)
{
    /// <summary>Пустой. Внутренний — безопаснее: случайно ушедший текст увидят сотрудники, а не заявитель.</summary>
    public static readonly CommentDraft Empty = new("", true, "");
}

/// <summary>Запись в Интрасервис (этап 8): комментарий из панели — только когда запись разрешена в настройках, после сверки
/// переписки и предпросмотра. Больше в Интрасервис ничто не пишет: ни автообновление, ни F5, ни выгрузка, ни Claude.</summary>
public sealed partial class MainViewModel
{
    private readonly Dictionary<int, CommentDraft> _drafts = new();
    /// <summary>Заявки, чей комментарий сейчас отправляется: второе «Отправить…» той же заявки ждёт первое, другие — нет.</summary>
    private readonly HashSet<int> _sending = new();
    /// <summary>Учётная запись, к которой относятся черновики: на другом сервере (или у другого логина) у заявки с тем же номером
    /// своя переписка — черновики прежней туда не годятся.</summary>
    private string? _draftsAccount;

    /// <summary>Поле комментария в панели: запись разрешена, API настроен, у заявки есть номер.</summary>
    public bool CanComment => _settings.AllowIntraserviceWrite && _intraservice is not null && SelectedTicket?.IntraserviceId is not null;

    /// <summary>Текст комментария выбранной заявки — её черновик.</summary>
    public string CommentText
    {
        get => DraftOf(SelectedTicket).Text;
        set => EditDraft(d => d with { Text = value });
    }

    /// <summary>Внутренний (видят только сотрудники) — у черновика выбранной заявки.</summary>
    public bool CommentInternal
    {
        get => DraftOf(SelectedTicket).Internal;
        set => EditDraft(d => d with { Internal = value });
    }

    /// <summary>Итог последней отправки выбранной заявки: «отправлено ДД.ММ в ЧЧ:ММ», «не отправлен: …», «неизвестно, дошёл ли: …».</summary>
    public string CommentResult => DraftOf(SelectedTicket).Result;

    private CommentDraft DraftOf(Ticket? t) => t?.IntraserviceId is int n ? DraftOf(n) : CommentDraft.Empty;

    private CommentDraft DraftOf(int n) => _drafts.GetValueOrDefault(n, CommentDraft.Empty);

    private void EditDraft(Func<CommentDraft, CommentDraft> edit)
    {
        if (SelectedTicket?.IntraserviceId is int n) SetDraft(n, edit(DraftOf(n)));
    }

    /// <summary>Черновик заявки n (пустой — убрать); выбрана она — панель перечитывает поле, вид и итог.</summary>
    private void SetDraft(int n, CommentDraft draft)
    {
        if (draft == CommentDraft.Empty) _drafts.Remove(n);
        else _drafts[n] = draft;
        if (SelectedTicket?.IntraserviceId == n) NotifyDraft();
    }

    private void Result(int n, string text) => SetDraft(n, DraftOf(n) with { Result = text });

    /// <summary>Выбрали другую заявку или поменялись настройки — поле, вид и итог перечитываются (у каждой заявки свои).</summary>
    private void NotifyDraft()
    {
        OnPropertyChanged(nameof(CanComment));
        OnPropertyChanged(nameof(CommentText));
        OnPropertyChanged(nameof(CommentInternal));
        OnPropertyChanged(nameof(CommentResult));
    }

    /// <summary>Сменилась учётная запись (адрес, логин) — черновики забыты; просто сохранили настройки — нет.</summary>
    private void ForgetDrafts()
    {
        if (_settings.AccountKey == _draftsAccount) return;
        _draftsAccount = _settings.AccountKey;
        _drafts.Clear();
    }

    /// <summary>«Отправить…». Заявка заново с сервера: карточка (её Changed уйдёт в запрос — изменят заявку после, сервер
    /// ответит 409), потом переписка; есть в ней записи, которых нет на экране, — не отправляем, а показываем их. Потом
    /// предпросмотр, потом запись — один раз, без повторов. Итог — в черновик заявки: виден, когда она выбрана; отправлено —
    /// черновик пуст. Одновременно — у разных заявок можно (кнопка одна на все), у одной — нет.</summary>
    [RelayCommand(AllowConcurrentExecutions = true)]
    private async Task SendComment()
    {
        if (!CanComment || SelectedTicket is not Ticket t || t.IntraserviceId is not int n || _intraservice is not { } client) return;
        if (!_sending.Add(n)) return;   // эта заявка уже отправляется — второй раз не шлём
        try { await SendCommentAsync(t, n, client); }
        finally { _sending.Remove(n); }
    }

    private async Task SendCommentAsync(Ticket t, int n, HttpIntraserviceClient client)
    {
        var account = _settings.AccountKey;
        if (DraftOf(n).Text.Trim().Length == 0) { Result(n, "напишите комментарий"); return; }

        Result(n, "проверяю заявку…");
        var card = await client.GetTaskAsync(n);
        if (card.Task is not { } task) { Result(n, $"не отправлен: {card.Error}"); return; }
        var life = await client.GetLifetimeAsync(n);
        if (life.Error.Length > 0) { Result(n, $"не отправлен: {life.Error}"); return; }
        if (SelectedTicket != t) { Result(n, "не отправлен: выбрали другую заявку — откройте эту и нажмите «Отправить…» снова"); return; }
        // поправили, пока сверялась переписка, — в предпросмотр и в запрос идёт то, что в поле сейчас
        var draft = DraftOf(n);
        var text = draft.Text.Trim();
        if (text.Length == 0) { Result(n, "напишите комментарий"); return; }
        var rows = ToRows(life.Events);
        var shown = _commentsShownFor == t;   // не на экране — ещё грузится или не загрузилась
        if (!shown || !Comments.SequenceEqual(rows))
        {
            // человек отвечает на то, что видел, — а там уже другое (или он не видел ничего): сначала пусть прочитает. Новое —
            // одни смены статуса без комментария, а они спрятаны, — сказать, где их видно
            // новые — записи, которых не было (плашка статуса у соседней могла сдвинуться — это не новая запись)
            var before = Comments.Select(c => c with { StatusChange = null }).ToHashSet();
            var news = rows.Where(r => !before.Contains(r with { StatusChange = null })).ToList();
            var hidden = !ShowAllEvents && news.Count > 0 && news.All(r => r.Text is null);
            ShowFresh(t, n, rows, life.HasMore);
            Result(n, !shown
                ? "не отправлен: переписки ещё не было на экране — теперь она есть; прочитайте и нажмите «Отправить…» снова"
                : news.Count == 0   // новых нет — запись убрали или поправили
                    ? "не отправлен: пока вы писали, переписка в заявке изменилась — она на экране; прочитайте и нажмите «Отправить…» снова"
                : hidden ? "не отправлен: пока вы писали, у заявки сменился статус — эти записи показывает кнопка-глаз; посмотрите и нажмите «Отправить…» снова"
                : "не отправлен: пока вы писали, в заявке появились новые записи — они на экране; прочитайте и нажмите «Отправить…» снова");
            return;
        }

        // вид — первой строкой; название — с сервера (у карточки доски своё, его могли поменять), одной строкой: его пишет
        // заявитель, и его перевод строки не подделает строку вида
        var kind = draft.Internal ? "Внутренний — увидят только сотрудники." : "Публичный — увидит инициатор.";
        if (!Views.AskWindow.Ask($"Отправить комментарий в заявку #{n}?", $"{kind}\n«{KnowledgeExport.OneLine(task.Name)}»\n\n{text}", "Отправить"))
        {
            Result(n, "");
            return;
        }
        // пока был открыт предпросмотр, запись выключили или сменили сервер или логин — выключатель главнее «Отправить»; просто
        // сохранили настройки (новый клиент той же учётной записи) — не повод
        if (!_settings.AllowIntraserviceWrite || _intraservice is null || _settings.AccountKey != account)
        {
            Result(n, "не отправлен: пока был открыт предпросмотр, поменялись настройки — нажмите «Отправить…» снова");
            return;
        }

        Result(n, "отправляю…");
        var r = await client.CommentAsync(n, text, draft.Internal, task.ChangedText);
        switch (r.Outcome)
        {
            case WriteOutcome.Done:
                // пока шла запись, дописали — это уже следующий комментарий: его не стираем
                var now = DateTime.Now;
                var sent = $"отправлено {now:dd.MM} в {now:HH:mm}";   // с датой: в трее приложение живёт сутками
                SetDraft(n, DraftOf(n).Text == draft.Text ? CommentDraft.Empty with { Result = sent } : DraftOf(n) with { Result = sent });
                break;
            case WriteOutcome.Refused:
                Result(n, $"не отправлен: {r.Error}" + (r.Conflict ? "\nПереписка перечитана — прочитайте и нажмите «Отправить…» снова." : ""));
                break;
            default:
                Log?.Invoke($"Комментарий в заявку #{n}: неизвестно, дошёл ли: {r.Error}");   // текст комментария в журнал не пишем
                Result(n, $"неизвестно, дошёл ли: {r.Error}\nПереписка перечитана — если вашего комментария в ней нет, отправьте снова.");
                break;
        }
        // заявка на сервере могла измениться (записали мы или кто-то до нас) — переписку заново, мимо кэша
        if (r.Outcome != WriteOutcome.Refused || r.Conflict)
        {
            _commentCache.Remove(n);
            if (SelectedTicket == t) LoadComments(t, force: true);
        }
    }
}
