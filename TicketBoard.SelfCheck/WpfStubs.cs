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
        public List<string> Added { get; } = new();
        public Ticket AddFromCapture(string input, TicketPriority priority) { Added.Add(input); var t = new Ticket(); Tickets.Add(t); return t; }
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
