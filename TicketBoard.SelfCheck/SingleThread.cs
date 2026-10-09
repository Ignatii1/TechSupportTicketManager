using System.Collections.Concurrent;
using System.Collections.Specialized;
using System.ComponentModel;

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

    /// <summary>Выполнить проверку в текущем потоке: body идёт с этой очередью, очередь разбирается, пока body не кончится.</summary>
    public static void Run(Func<SingleThread, Task> body)
    {
        var previous = Current;
        var context = new SingleThread();
        var dispatcher = System.Windows.Application.Current!.Dispatcher;
        SetSynchronizationContext(context);
        dispatcher.Context = context;
        try
        {
            var task = body(context);
            task.ContinueWith(_ => context._queue.CompleteAdding(), TaskScheduler.Default);
            foreach (var (callback, state) in context._queue.GetConsumingEnumerable()) callback(state);
            task.GetAwaiter().GetResult();
        }
        finally
        {
            dispatcher.Context = null;
            SetSynchronizationContext(previous);
        }
    }
}
