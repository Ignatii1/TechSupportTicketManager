using System.Diagnostics;

namespace TicketBoard.Services;

/// <summary>Правила автообновления (ViewModels/MainViewModel.AutoSync.cs), которые проверяются без окон — в TicketBoard.SelfCheck.</summary>
public static class AutoSyncRules
{
    /// <summary>Номера, которые автообновление не добавляет на доску (AutoSyncSkipIds), и стоит ли их сохранить.
    /// Первый запуск (saved — null): всё открытое моё, чего нет на доске, — прошлое, его приносит импорт; но только по
    /// целому списку — по неполному прошлое от нового не отличить, поэтому в этот раз не добавляем ничего и ничего не
    /// запоминаем. Дальше — только ещё открытые мои, и снова только по целому списку: неполный «забыл» бы удалённую.</summary>
    public static (HashSet<int> Skip, bool Persist) Skip(int[]? saved, IReadOnlyCollection<int> listed, IEnumerable<int> onBoard,
        bool complete) =>
        saved is null ? (complete ? (listed.Except(onBoard).ToHashSet(), true) : (listed.ToHashSet(), false))
        : complete ? (saved.Where(listed.Contains).ToHashSet(), true)
        : (saved.ToHashSet(), false);

    /// <summary>Весь список по страницам, fetch(номер страницы), — у импорта и автообновления. Конец — пустая страница или
    /// неполная, когда забрано всё обещанное (Total). Одного Total мало: без Paginator он равен пришедшему; одной
    /// неполной страницы тоже: сервер вправе отдавать за раз меньше, чем просили. Complete — дошли до конца без ошибки
    /// и различных номеров не меньше Total: список сортирован по изменению и, пока листали, мог сдвинуться — заявка на
    /// стыке страниц проскочила бы, а соседняя пришла дважды. Ошибка или потолок maxPages — что пришло, то и отдаём.</summary>
    public static async Task<(List<IntraserviceFound> Rows, int Total, bool Complete, string Error)> ReadAllPagesAsync(
        Func<int, Task<IntraserviceSearchResult>> fetch, int pageSize, int maxPages)
    {
        var rows = new List<IntraserviceFound>();
        var total = 0;
        for (var page = 1; page <= maxPages; page++)
        {
            var r = await fetch(page);
            if (r.Error.Length > 0) return (rows, total, false, r.Error);
            rows.AddRange(r.Found);
            total = Math.Max(total, r.Total);
            if (r.Found.Count == 0 || (rows.Count >= total && r.Found.Count < pageSize))
                return (rows, total, rows.Select(f => f.Id).Distinct().Count() >= total, "");
        }
        return (rows, total, false, "");
    }

    /// <summary>Новые комментарии заявки для бейджа на карточке: чужие (не мои — по номеру автора, а без него — по имени)
    /// и новее seen — даты самого нового уже виденного комментария. Свой ответ сдвигает seen: ответил — значит, прочитал
    /// всё до него. seen ещё нет — считаем прочитанным всё, что есть (иначе старая переписка хлынула бы как новая).
    /// Даты с обеих сторон — сервера: часы этого компьютера не участвуют. Unread — свежие сверху.</summary>
    public static (int Count, DateTimeOffset? Seen, List<IntraserviceEvent> Unread) Unread(
        IEnumerable<IntraserviceEvent> events, DateTimeOffset? seen, IntraserviceUser? me)
    {
        bool Mine(IntraserviceEvent e) => me is not null && (e.AuthorId is int id ? id == me.Id
            : me.Name.Length > 0 && string.Equals(e.Author, me.Name, StringComparison.OrdinalIgnoreCase));

        var comments = events.Where(e => e.Comment is not null && e.Date is not null).ToList();
        if (seen is null) return (0, comments.Max(e => e.Date), new());
        var myLast = comments.Where(Mine).Max(e => e.Date);
        var since = myLast > seen ? myLast : seen;
        var unread = comments.Where(e => !Mine(e) && e.Date > since).OrderByDescending(e => e.Date).ToList();
        return (unread.Count, since, unread);
    }

    /// <summary>«Прочитано до» по Changed заявки, когда переписку ещё не читали (новая карточка, первая встреча), — с запасом
    /// в секунду: Changed бывает грубее дат комментариев (секунды против миллисекунд), и комментарий, который его сдвинул,
    /// иначе потом сошёл бы за новый. Цена — чужой комментарий в ту же секунду не заметим.</summary>
    public static DateTimeOffset SeenFrom(DateTimeOffset changed) => changed.AddSeconds(1);

    /// <summary>Что положить во «Входящие»: мои открытые, которых нет на доске и которые не в Skip.</summary>
    public static List<int> ToAdd(IEnumerable<int> listed, IReadOnlySet<int> onBoard, IReadOnlySet<int> skip) =>
        listed.Where(id => !onBoard.Contains(id) && !skip.Contains(id)).ToList();

    [Conditional("DEBUG")]
    internal static void SelfCheck()
    {
        // первый запуск: открытое, чего нет на доске, — прошлое; во «Входящие» ничего не падает
        int[] listed = { 1, 2, 3, 4 };
        var board = new HashSet<int> { 1 };
        var (skip, persist) = Skip(null, listed, board, complete: true);
        Debug.Assert(persist && skip.SetEquals(new[] { 2, 3, 4 }) && ToAdd(listed, board, skip).Count == 0);

        // первый запуск на неполном списке: не добавить ничего и ничего не запомнить — иначе прошлое со следующих
        // страниц потом приехало бы как «новое»
        (skip, persist) = Skip(null, listed, board, complete: false);
        Debug.Assert(!persist && ToAdd(listed, board, skip).Count == 0);

        // потом: новая на меня — добавляется; удалённая с доски (в skip) — нет; закрытая или переданная (9) — забыта
        int[] later = { 1, 2, 3, 4, 5 };
        (skip, persist) = Skip(new[] { 2, 3, 4, 9 }, later, board, complete: true);
        Debug.Assert(persist && skip.SetEquals(new[] { 2, 3, 4 }) && ToAdd(later, board, skip).SequenceEqual(new[] { 5 }));

        // неполный список (пришли не все страницы, упёрлись в потолок) ничего не забывает и ничего не сохраняет
        (skip, persist) = Skip(new[] { 2, 9 }, new[] { 1 }, board, complete: false);
        Debug.Assert(!persist && skip.SetEquals(new[] { 2, 9 }));
        // пустой сохранённый список — это «уже запускалось», а не первый запуск
        Debug.Assert(Skip(Array.Empty<int>(), listed, board, complete: true).Skip.Count == 0);

        PagesSelfCheck();
        UnreadSelfCheck();
    }

    private static void UnreadSelfCheck()
    {
        var me = new IntraserviceUser(7, "Я Сам");
        var t0 = new DateTimeOffset(2026, 9, 26, 10, 0, 0, TimeSpan.FromHours(3));
        IntraserviceEvent Comment(int minutes, string author, int? authorId, string? text = "текст") =>
            new(t0.AddMinutes(minutes), author, "Открыта", text, true, authorId);

        // первая проверка: старая переписка прочитана, отсчёт — самый новый комментарий
        var history = new[] { Comment(-30, "Иванов", 1), Comment(-10, "Петров", 2), Comment(-5, "Сидоров", 3, text: null) };
        Debug.Assert(Unread(history, null, me) is { Count: 0 } first && first.Seen == t0.AddMinutes(-10));

        // после отсчёта: чужой комментарий — новый; смена статуса без текста — нет; порядок ответа сервера не важен
        var later = history.Append(Comment(5, "Иванов", 1)).Append(Comment(7, "Сидоров", 3, text: null)).Reverse().ToList();
        var (count, seen, unread) = Unread(later, t0, me);
        Debug.Assert(count == 1 && seen == t0 && unread[0].Date == t0.AddMinutes(5));

        // свой ответ: всё до него прочитано, после — снова новое; свой не считается никогда
        var replied = later.Append(Comment(6, "Я Сам", 7)).Append(Comment(8, "Петров", 2)).Append(Comment(9, "Я Сам", 7)).ToList();
        (count, seen, _) = Unread(replied, t0, me);
        Debug.Assert(count == 0 && seen == t0.AddMinutes(9));
        (count, _, unread) = Unread(replied.Append(Comment(12, "Петров", 2)).Append(Comment(11, "Иванов", 1)), t0, me);
        Debug.Assert(count == 2 && unread.Select(e => e.Author).SequenceEqual(new[] { "Петров", "Иванов" }));

        // отсчёт от Changed с запасом: Changed в секундах, а комментарий, который его сдвинул, — с миллисекундами
        var changed = new DateTimeOffset(2026, 9, 26, 10, 15, 0, TimeSpan.FromHours(3));
        var trigger = new IntraserviceEvent(changed.AddMilliseconds(480), "Иванов", "Открыта", "тот самый", true, 1);
        Debug.Assert(Unread(new[] { trigger }, SeenFrom(changed), me).Count == 0);
        Debug.Assert(Unread(new[] { trigger }, changed, me).Count == 1);   // без запаса он сошёл бы за новый

        // сервер не прислал номер автора — узнаём себя по имени (без учёта регистра); без даты — не в счёт
        var noIds = new[] { Comment(5, "я сам", null), Comment(6, "Иванов", null),
            new IntraserviceEvent(null, "Петров", "Открыта", "без даты", true) };
        (count, seen, _) = Unread(noIds, t0, me);
        Debug.Assert(count == 1 && seen == t0.AddMinutes(5));
    }

    /// <summary>Листание: сервер отдаёт страницы из pages (номера заявок и его Total); просим по 4 на страницу.</summary>
    private static void PagesSelfCheck()
    {
        static (List<IntraserviceFound> Rows, int Total, bool Complete, string Error, int Asked) Read(
            params (int[] Ids, int Total, string Error)[] pages)
        {
            var asked = 0;
            var r = ReadAllPagesAsync(page =>
            {
                asked++;
                var (ids, total, error) = page <= pages.Length ? pages[page - 1] : (Array.Empty<int>(), 0, "");
                return Task.FromResult(new IntraserviceSearchResult(
                    ids.Select(id => new IntraserviceFound(id, $"Заявка {id}", "Открыта", null, null, null)).ToList(), total, error));
            }, pageSize: 4, maxPages: 3).GetAwaiter().GetResult();
            return (r.Rows, r.Total, r.Complete, r.Error, asked);
        }

        // одна неполная страница — весь список, второй запрос не нужен
        var one = Read((new[] { 1, 2, 3 }, 3, ""));
        Debug.Assert(one.Complete && one.Rows.Count == 3 && one.Asked == 1);
        // пусто — тоже целый список
        Debug.Assert(Read((Array.Empty<int>(), 0, "")) is { Complete: true, Rows.Count: 0 });

        // без Paginator Total равен пришедшему: полная страница — ещё не конец
        var noPaginator = Read((new[] { 1, 2, 3, 4 }, 4, ""), (new[] { 5, 6 }, 2, ""));
        Debug.Assert(noPaginator.Complete && noPaginator.Rows.Count == 6 && noPaginator.Asked == 2);
        // сервер отдаёт меньше, чем просили (по 2 вместо 4), но Total знает — листаем, пока не заберём всё
        var capped = Read((new[] { 1, 2 }, 5, ""), (new[] { 3, 4 }, 5, ""), (new[] { 5 }, 5, ""));
        Debug.Assert(capped.Complete && capped.Rows.Count == 5 && capped.Asked == 3);
        // полная последняя страница — конец узнаём по пустой следующей
        var exact = Read((new[] { 1, 2, 3, 4 }, 4, ""));
        Debug.Assert(exact.Complete && exact.Asked == 2);

        // список сдвинулся, пока листали: номер 4 пришёл дважды, одна заявка проскочила — не целый
        var shifted = Read((new[] { 1, 2, 3, 4 }, 8, ""), (new[] { 4, 5, 6 }, 7, ""), (Array.Empty<int>(), 7, ""));
        Debug.Assert(!shifted.Complete && shifted.Rows.Count == 7 && shifted.Error == "");
        // ошибка на второй странице: первая пригодится, но список не целый
        var broken = Read((new[] { 1, 2, 3, 4 }, 6, ""), (Array.Empty<int>(), 0, "HTTP 500"));
        Debug.Assert(!broken.Complete && broken.Rows.Count == 4 && broken.Error == "HTTP 500");
        // упёрлись в потолок страниц (3 полных, а заявок больше) — не целый
        var ceiling = Read((new[] { 1, 2, 3, 4 }, 20, ""), (new[] { 5, 6, 7, 8 }, 20, ""), (new[] { 9, 10, 11, 12 }, 20, ""));
        Debug.Assert(!ceiling.Complete && ceiling.Rows.Count == 12 && ceiling.Asked == 3);
    }
}
