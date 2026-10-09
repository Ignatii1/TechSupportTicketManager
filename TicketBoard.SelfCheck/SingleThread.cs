using System.Collections.Concurrent;
using System.Collections.Specialized;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.ExceptionServices;

namespace TicketBoard.SelfCheck;

/// <summary>Один поток для всех продолжений await — как UI-поток WPF: доска и окно поиска собирают итоги параллельных
/// запросов в обычные списки без блокировок и меняют привязанные коллекции, рассчитывая, что продолжения идут в их поток
/// (в пуле потоков проверка гонялась бы с ними, а WPF бросил бы исключение). Отложенное через Dispatcher (WpfStubs.cs)
/// встаёт в ту же очередь — после текущей работы, как Background у WPF.</summary>
internal sealed class SingleThread : SynchronizationContext
{
    private readonly BlockingCollection<(SendOrPostCallback Callback, object? State)> _queue = new();
    private readonly int _thread = Environment.CurrentManagedThreadId;
    private int _offThread;

    /// <summary>Сколько изменений пришло не из потока очереди (см. Watch и WatchItems).</summary>
    public int OffThread => Volatile.Read(ref _offThread);

    /// <summary>Считать изменения свойств не из потока очереди — у доски таких быть не должно: она рассчитывает на один поток.</summary>
    public void Watch(INotifyPropertyChanged source) => source.PropertyChanged += (_, _) => Note();

    /// <summary>Считать изменения коллекции не из потока очереди — привязанную так менять WPF не даёт.</summary>
    public void WatchItems(INotifyCollectionChanged source) => source.CollectionChanged += (_, _) => Note();

    private void Note()
    {
        if (Environment.CurrentManagedThreadId != _thread) Interlocked.Increment(ref _offThread);
    }

    public override void Post(SendOrPostCallback d, object? state)
    {
        try { _queue.Add((d, state)); }
        catch (InvalidOperationException) { /* проверка кончилась — то, что не доделано в фоне, уже никому не нужно */ }
    }

    public override void Send(SendOrPostCallback d, object? state) => throw new NotSupportedException();

    public override SynchronizationContext CreateCopy() => this;

    /// <summary>Выполнить проверку в текущем потоке: body идёт с этой очередью, очередь разбирается, пока body не кончится.
    /// Исключение из колбэка очереди (async void доски, отложенное через Dispatcher — в WPF это падение приложения) не
    /// обрывает body на полпути: он доходит до конца — со своим finally и списком непрошедших проверок. Исключение
    /// печатается сразу (Debug.Assert в конце проверки может уронить процесс раньше) и бросается следом.</summary>
    public static void Run(Func<SingleThread, Task> body)
    {
        var previous = Current;
        var context = new SingleThread();
        var dispatcher = System.Windows.Application.Current!.Dispatcher;
        lock (System.Windows.Threading.DispatcherTimer.Created) System.Windows.Threading.DispatcherTimer.Created.Clear();
        SetSynchronizationContext(context);
        dispatcher.Context = context;
        ExceptionDispatchInfo? thrown = null;
        try
        {
            var task = body(context);
            task.ContinueWith(_ => context._queue.CompleteAdding(), TaskScheduler.Default);
            foreach (var (callback, state) in context._queue.GetConsumingEnumerable())
                try { callback(state); }
                catch (Exception e)
                {
                    Console.Error.WriteLine($"  исключение в колбэке очереди (в WPF — падение приложения):\n    {e.ToString().Replace("\n", "\n    ")}");
                    thrown ??= ExceptionDispatchInfo.Capture(e);
                }
            task.GetAwaiter().GetResult();
            thrown?.Throw();
        }
        finally
        {
            dispatcher.Context = null;
            SetSynchronizationContext(previous);
            lock (System.Windows.Threading.DispatcherTimer.Created) System.Windows.Threading.DispatcherTimer.Created.Clear();
        }
    }

    /// <summary>Ждать условия, не держа поток очереди: false — не дождались за ms.</summary>
    public static async Task<bool> Until(Func<bool> condition, int ms = 8000)
    {
        var sw = Stopwatch.StartNew();
        while (!condition() && sw.ElapsedMilliseconds < ms) await Task.Delay(20);
        return condition();
    }
}
