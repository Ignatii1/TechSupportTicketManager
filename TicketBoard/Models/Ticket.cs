using System.Collections.ObjectModel;
using System.Collections.Specialized;
using System.Text.Json.Serialization;
using CommunityToolkit.Mvvm.ComponentModel;

namespace TicketBoard.Models;

/// <summary>Локальный статус заявки — колонка на доске. Не путать с ExternalStatus (статус в Интрасервисе).</summary>
public enum TicketStatus { Inbox, InProgress, Waiting, Done }

public enum TicketPriority { Low, Mid, High }

/// <summary>Состояние индикатора возраста карточки.</summary>
public enum AgeState { Neutral, Warn, Overdue }

/// <summary>Правила из settings.json, нужные модели. Заполняются при старте.</summary>
public static class TicketRules
{
    public static int OverdueDays { get; set; } = 3;
    public static int OverloadLimit { get; set; } = 5;
}

public sealed record Note
{
    public Guid Id { get; init; } = Guid.NewGuid();
    public DateTimeOffset CreatedAt { get; init; } = DateTimeOffset.Now;
    public string Text { get; init; } = "";
}

public sealed partial class Ticket : ObservableObject
{
    public Guid Id { get; init; } = Guid.NewGuid();
    public DateTimeOffset CreatedAt { get; init; } = DateTimeOffset.Now;

    [ObservableProperty] private string _title = "";
    [ObservableProperty] private string _url = "";
    [ObservableProperty] private string _description = "";
    [ObservableProperty] private TicketPriority _priority = TicketPriority.Mid;

    [ObservableProperty, NotifyPropertyChangedFor(nameof(AgeState))]
    private TicketStatus _status = TicketStatus.Inbox;
    [ObservableProperty, NotifyPropertyChangedFor(nameof(DaysInStatus), nameof(AgeState), nameof(AgeLabel), nameof(AgeText))]
    private DateTimeOffset _statusChangedAt = DateTimeOffset.Now;
    [ObservableProperty] private DateTimeOffset? _completedAt;

    // ---- Интрасервис. Сейчас заполняется парсером ссылки, позже — API. ----
    [ObservableProperty, NotifyPropertyChangedFor(nameof(DisplayNumber))]
    private int? _intraserviceId;
    /// <summary>Статус заявки на стороне Интрасервиса (придёт из API).</summary>
    [ObservableProperty] private string? _externalStatus;
    [ObservableProperty] private DateTimeOffset? _lastSyncAt;

    /// <summary>Закрытый статус, с которым на вопрос «перенести в «Готово»?» ответили «Оставить»: с ним карточку больше
    /// не предлагаем и не считаем в заголовке. Статус сменился (переоткрыли, закрыли иначе) — снова спросим.</summary>
    [ObservableProperty] private string? _keptOpenStatus;

    partial void OnExternalStatusChanged(string? value)
    {
        if (value != KeptOpenStatus) KeptOpenStatus = null;
    }

    private ObservableCollection<Note> _notes = new();
    public ObservableCollection<Note> Notes
    {
        get => _notes;
        set
        {
            _notes.CollectionChanged -= OnNotesChanged;
            _notes = value ?? new();
            _notes.CollectionChanged += OnNotesChanged;
            OnPropertyChanged(nameof(NotesCount));
        }
    }

    public Ticket() => _notes.CollectionChanged += OnNotesChanged;

    private void OnNotesChanged(object? s, NotifyCollectionChangedEventArgs e) => OnPropertyChanged(nameof(NotesCount));

    // ---- вычисляемое, в JSON не пишется ----

    [JsonIgnore] public int NotesCount => Notes.Count;
    [JsonIgnore] public int DaysInStatus => Math.Max(0, (int)(DateTimeOffset.Now.Date - StatusChangedAt.Date).TotalDays);
    [JsonIgnore] public string DisplayNumber => IntraserviceId is int n ? $"#{n}" : "без номера";

    /// <summary>Готово никогда не краснеет; порог − 1 день — жёлтый, порог и больше — красный.</summary>
    [JsonIgnore] public AgeState AgeState =>
        Status == TicketStatus.Done ? AgeState.Neutral
        : DaysInStatus >= TicketRules.OverdueDays ? AgeState.Overdue
        : DaysInStatus >= TicketRules.OverdueDays - 1 ? AgeState.Warn
        : AgeState.Neutral;

    /// <summary>Бейдж на карточке: «сегодня» / «N д».</summary>
    [JsonIgnore] public string AgeLabel => DaysInStatus == 0 ? "сегодня" : $"{DaysInStatus} д";

    /// <summary>Строка в панели: «с сегодня» / «2 дня» / «5 дней — просрочена».</summary>
    [JsonIgnore] public string AgeText => DaysInStatus == 0 ? "с сегодня"
        : Plural(DaysInStatus, "день", "дня", "дней") + (AgeState == AgeState.Overdue ? " — просрочена" : "");

    private static string Plural(int n, string one, string few, string many)
    {
        int m = n % 10, h = n % 100;
        var word = h is > 10 and < 20 ? many : m == 1 ? one : m is > 1 and < 5 ? few : many;
        return $"{n} {word}";
    }

    public void MoveTo(TicketStatus status)
    {
        if (Status == status) return;
        Status = status;
        StatusChangedAt = DateTimeOffset.Now;
        CompletedAt = status == TicketStatus.Done ? DateTimeOffset.Now : null;
        MarkAppear();
    }

    // ---- анимация появления карточки: только в памяти, в JSON не пишется (приватное поле) ----
    private long _appearAt;

    /// <summary>Карточка только что появилась в колонке (перенос, новая) — показать с анимацией.</summary>
    public void MarkAppear() => _appearAt = Environment.TickCount64;

    /// <summary>Один раз: true, если отметка свежая. Старая (карточка была скрыта фильтром, окно в трее) — сгорает.</summary>
    public bool TakeAppear()
    {
        var fresh = _appearAt != 0 && Environment.TickCount64 - _appearAt < 500;
        _appearAt = 0;
        return fresh;
    }

    /// <summary>Пересчитать возраст (вызывается по таймеру и при смене дня).</summary>
    public void RefreshAge()
    {
        OnPropertyChanged(nameof(DaysInStatus));
        OnPropertyChanged(nameof(AgeState));
        OnPropertyChanged(nameof(AgeLabel));
        OnPropertyChanged(nameof(AgeText));
    }
}
