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
    }
}
