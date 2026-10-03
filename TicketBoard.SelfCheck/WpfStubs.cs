// Заглушки того, что SearchViewModel берёт из WPF-части приложения (доска, карточка, буфер обмена, папка данных): в консоли
// самопроверки её нет. Добавил во viewmodel новое обращение к приложению — добавь сюда такую же заглушку.
namespace TicketBoard
{
    public static class App { public static string DataDir { get; set; } = Path.GetTempPath(); }
}
namespace TicketBoard.Models
{
    public enum TicketPriority { Low, Mid, High }
    public sealed class Ticket { public int? IntraserviceId { get; set; } }
}
namespace TicketBoard.ViewModels
{
    using TicketBoard.Models;
    public sealed class MainViewModel
    {
        public List<Ticket> Tickets { get; } = new();
        public IEnumerable<Ticket> AllTickets => Tickets;
        public List<(int Id, string Url, string Title)> Added { get; } = new();
        public Ticket AddKnown(int id, string url, string title, TicketPriority priority)
        {
            Added.Add((id, url, title));
            var t = new Ticket { IntraserviceId = id };
            Tickets.Add(t);
            return t;
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
