using System.Diagnostics;

namespace TicketBoard.Services;

/// <summary>Правила автообновления (ViewModels/MainViewModel.AutoSync.cs), которые проверяются без окон — в TicketBoard.SelfCheck.</summary>
public static class AutoSyncRules
{
    /// <summary>Номера, которые автообновление не добавляет на доску (AutoSyncSkipIds). Первый запуск (saved — null):
    /// всё открытое моё, чего нет на доске, — прошлое, его приносит импорт. Дальше — только ещё открытые мои, и только
    /// если список пришёл целиком: неполный ответ «забыл» бы удалённую, и она вернулась бы на доску.</summary>
    public static HashSet<int> Skip(int[]? saved, IReadOnlyCollection<int> listed, IEnumerable<int> onBoard, bool complete) =>
        saved is null ? listed.Except(onBoard).ToHashSet()
        : complete ? saved.Where(listed.Contains).ToHashSet()
        : saved.ToHashSet();

    /// <summary>Что положить во «Входящие»: мои открытые, которых нет на доске и которые не в Skip.</summary>
    public static List<int> ToAdd(IEnumerable<int> listed, IReadOnlySet<int> onBoard, IReadOnlySet<int> skip) =>
        listed.Where(id => !onBoard.Contains(id) && !skip.Contains(id)).ToList();

    [Conditional("DEBUG")]
    internal static void SelfCheck()
    {
        // первый запуск: открытое, чего нет на доске, — прошлое; во «Входящие» ничего не падает
        int[] listed = { 1, 2, 3, 4 };
        var board = new HashSet<int> { 1 };
        var skip = Skip(null, listed, board, complete: true);
        Debug.Assert(skip.SetEquals(new[] { 2, 3, 4 }) && ToAdd(listed, board, skip).Count == 0);

        // потом: новая на меня — добавляется; удалённая с доски (в skip) — нет; закрытая или переданная (9) — забыта
        int[] later = { 1, 2, 3, 4, 5 };
        skip = Skip(new[] { 2, 3, 4, 9 }, later, board, complete: true);
        Debug.Assert(skip.SetEquals(new[] { 2, 3, 4 }) && ToAdd(later, board, skip).SequenceEqual(new[] { 5 }));

        // неполный список (пришли не все страницы) ничего не забывает
        Debug.Assert(Skip(new[] { 2, 9 }, new[] { 1 }, board, complete: false).SetEquals(new[] { 2, 9 }));
        // пустой сохранённый список — это «уже запускалось», а не первый запуск
        Debug.Assert(Skip(Array.Empty<int>(), listed, board, complete: true).Count == 0);
    }
}
