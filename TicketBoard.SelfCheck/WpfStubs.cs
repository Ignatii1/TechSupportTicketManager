// Заглушки того, что viewmodel'и доски и окна поиска берут из WPF и из остальной части приложения (папка данных, диалоги,
// буфер обмена, таймеры, отложенное через Dispatcher, представления коллекций, drag&drop): в консоли самопроверки их нет. Добавил во viewmodel новое
// обращение к WPF или к приложению — добавь сюда такую же заглушку.
namespace TicketBoard
{
    public static class App { public static string DataDir { get; set; } = Path.GetTempPath(); }
}
namespace TicketBoard.Views
{
    /// <summary>Диалоги доски: ответ на вопрос — Answer (по умолчанию «да»), всё показанное — в Shown.</summary>
    public static class AskWindow
    {
        public static Func<string, string, bool> Answer { get; set; } = (_, _) => true;
        public static List<(string Heading, string Text)> Shown { get; } = new();

        public static bool Ask(string heading, string text, string yes, string no = "Отмена", bool danger = false)
        {
            lock (Shown) Shown.Add((heading, text));
            return Answer(heading, text);
        }

        public static void Tell(string heading, string text)
        {
            lock (Shown) Shown.Add((heading, text));
        }
    }
}
namespace TicketBoard.Services
{
    internal static class ClipboardWatcher
    {
        public static string? Last;
        public static bool TrySetText(string text) { Last = text; return true; }
    }
}
namespace System.ComponentModel
{
    public interface ICollectionView : System.Collections.IEnumerable
    {
        Predicate<object>? Filter { get; set; }
        void Refresh();
    }
}
namespace System.Windows.Data
{
    /// <summary>Представление коллекции: фильтр применяется при перечислении — как у WPF после Refresh.</summary>
    public static class CollectionViewSource
    {
        public static System.ComponentModel.ICollectionView GetDefaultView(object source) => new View((System.Collections.IEnumerable)source);

        private sealed class View(System.Collections.IEnumerable source) : System.ComponentModel.ICollectionView
        {
            public Predicate<object>? Filter { get; set; }
            public void Refresh() { }
            public System.Collections.IEnumerator GetEnumerator()
            {
                foreach (var item in source)
                    if (Filter?.Invoke(item) != false) yield return item;
            }
        }
    }
}
namespace System.Windows.Threading
{
    public enum DispatcherPriority { Background }

    /// <summary>Отложенное — в очередь потока проверки (Context — её SingleThread): после текущей работы, как Background у
    /// WPF. Проверка без своей очереди — сразу.</summary>
    public sealed class Dispatcher
    {
        public SynchronizationContext? Context { get; set; }

        public void BeginInvoke(Action action, DispatcherPriority priority)
        {
            if (Context is { } queue) queue.Post(_ => action(), null);
            else action();
        }
    }

    /// <summary>Таймер, который сам не тикает: проверка находит таймеры доски среди Created (по Interval), смотрит IsEnabled
    /// и зовёт Fire — как тик WPF, в своём потоке.</summary>
    public sealed class DispatcherTimer
    {
        public static List<DispatcherTimer> Created { get; } = new();
        public DispatcherTimer() { lock (Created) Created.Add(this); }
        public TimeSpan Interval { get; set; }
        public bool IsEnabled { get; private set; }
        public event EventHandler? Tick;
        public void Start() => IsEnabled = true;
        public void Stop() => IsEnabled = false;
        public void Fire() => Tick?.Invoke(this, EventArgs.Empty);
    }
}
namespace System.Windows
{
    public sealed class Application
    {
        public static Application? Current { get; set; } = new();
        public Threading.Dispatcher Dispatcher { get; } = new();
    }
}
namespace GongSolutions.Wpf.DragDrop
{
    public interface IDropInfo { object? Data { get; } }

    public interface IDropTarget
    {
        void DragEnter(IDropInfo dropInfo);
        void DragOver(IDropInfo dropInfo);
        void DragLeave(IDropInfo dropInfo);
        void Drop(IDropInfo dropInfo);
    }

    /// <summary>Только чтобы собралось: настоящий gong при Drop переносит карточку между коллекциями колонок, этот — нет.
    /// Проверки переносят карточки командами доски (MoveTo), а не перетаскиванием.</summary>
    public class DefaultDropHandler : IDropTarget
    {
        public void DragEnter(IDropInfo dropInfo) { }
        public void DragOver(IDropInfo dropInfo) { }
        public void DragLeave(IDropInfo dropInfo) { }
        public void Drop(IDropInfo dropInfo) { }
    }
}
