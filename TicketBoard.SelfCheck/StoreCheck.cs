using System.Text.Json;
using TicketBoard.Models;
using TicketBoard.Services;

namespace TicketBoard.SelfCheck;

/// <summary>Самопроверка хранилища заявок (tickets.json): запись через временный файл, битый файл уходит в сторону, а не
/// затирается, бэкап раз в день и не больше 30 — и формат файла. Имена полей в нём — имена свойств Ticket: переименовали
/// свойство — заявки пользователя его потеряют. Поэтому файл в нынешних именах обязан читаться, а набор полей, которые
/// пишутся, — совпадать с образцом: новое поле добавляют в образец, сознательно.</summary>
internal static class StoreCheck
{
    /// <summary>Файл в нынешних именах, все поля заполнены.</summary>
    private const string Sample = """
        [
          {
            "Id": "9b2f6c1e-0000-4000-8000-000000000001",
            "CreatedAt": "2026-09-01T10:00:00+03:00",
            "Title": "Принтер не печатает",
            "Url": "https://hd/Task/View/702180",
            "Description": "с утра",
            "Priority": "High",
            "Status": "Waiting",
            "StatusChangedAt": "2026-09-02T11:00:00+03:00",
            "CompletedAt": "2026-09-03T12:00:00+03:00",
            "IntraserviceId": 702180,
            "ExternalStatus": "Закрыта",
            "LastSyncAt": "2026-09-04T13:00:00+03:00",
            "KeptOpenStatus": "Закрыта",
            "Creator": "Петрова А.",
            "CreatorPhone": "+7 900 000-00-00",
            "CreatorEmail": "petrova@example.ru",
            "Executors": "Я Сам, Сидоров С.",
            "ExecutorGroup": "Вторая линия",
            "CommentsCheckedFor": "2026-09-05T14:00:00+03:00",
            "CommentsSeenAt": "2026-09-05T14:00:01+03:00",
            "UnreadComments": 3,
            "AssignedToMe": false,
            "Notes": [ { "Id": "9b2f6c1e-0000-4000-8000-000000000002", "CreatedAt": "2026-09-06T15:00:00+03:00", "Text": "позвонить" } ]
          }
        ]
        """;

    public static void Run()
    {
        var checks = new CheckSet("хранилище");
        void Check(string name, bool ok, Func<string>? details = null) => checks.Check(name, ok, details);
        static DateTimeOffset At(string s) => DateTimeOffset.Parse(s);

        using (var data = new TempDataDir("tb-store"))
        {
            // 1. запись и чтение
            var store = new TicketStore(Path.Combine(data.Path, "store"));
            Check("файла нет — пустая доска, файл не создаётся", store.Load().Count == 0 && !File.Exists(store.FilePath));
            var card = new Ticket
            {
                Title = "Принтер", IntraserviceId = 702180, Status = TicketStatus.InProgress, Priority = TicketPriority.High,
                ExternalStatus = "В работе", Creator = "Петрова А.", AssignedToMe = true, UnreadComments = 2,
            };
            card.Notes.Add(new Note { Text = "позвонить" });
            store.Save(new[] { card, new Ticket { Title = "Без номера" } });
            var json = File.ReadAllText(store.FilePath);
            Check("запись и чтение — всё на месте", store.Load() is [var b1, var b2]
                && b1 is { Title: "Принтер", IntraserviceId: 702180, Status: TicketStatus.InProgress, Priority: TicketPriority.High,
                    ExternalStatus: "В работе", Creator: "Петрова А.", AssignedToMe: true, UnreadComments: 2 }
                && b1.Id == card.Id && b1.Notes is [{ Text: "позвонить" }] && b2 is { Title: "Без номера", IntraserviceId: null }, () => json);
            Check("…без временного файла; статус и приоритет словами; пустое и вычисляемое не пишется",
                !File.Exists(store.FilePath + ".tmp") && json.Contains("\"Status\": \"InProgress\"") && json.Contains("\"Priority\": \"High\"")
                && !json.Contains("DisplayNumber") && !json.Contains("ServerChanged") && !json.Contains("\"CompletedAt\""), () => json);
            Check("первая запись — без бэкапа: копировать нечего", !Directory.Exists(store.BackupDir));

            // 2. бэкап — раз в день, прежнего файла
            store.Save(new[] { card });
            var backups = Directory.Exists(store.BackupDir) ? Directory.GetFiles(store.BackupDir) : Array.Empty<string>();
            Check("вторая запись — бэкап прежнего файла", backups is [var morning]
                && Path.GetFileName(morning).StartsWith("tickets-") && File.ReadAllText(morning) == json, () => string.Join(", ", backups));
            store.Save(Array.Empty<Ticket>());
            Check("третья за день — утренний бэкап не тронут", backups is [var same] && File.ReadAllText(same) == json && store.Load().Count == 0);

            // 3. бэкапов — не больше 30: уходят самые старые
            var keep = new TicketStore(Path.Combine(data.Path, "keep"));
            keep.Save(new[] { card });
            Directory.CreateDirectory(keep.BackupDir);
            var first = new DateTime(2025, 1, 1);
            for (var d = 1; d <= 35; d++) File.WriteAllText(Path.Combine(keep.BackupDir, $"tickets-{first.AddDays(d):yyyy-MM-dd}.json"), "[]");
            keep.Save(new[] { card });
            var left = Directory.GetFiles(keep.BackupDir, "tickets-*.json").Select(f => Path.GetFileName(f)!).Order().ToList();
            Check("бэкапов 30: сегодняшний и самые свежие из старых", left.Count == 30
                && left[0] == $"tickets-{first.AddDays(7):yyyy-MM-dd}.json" && left[^1] == $"tickets-{DateTime.Now:yyyy-MM-dd}.json",
                () => $"{left.Count}: {left.FirstOrDefault()} … {left.LastOrDefault()}");

            // 4. битый файл — в сторону, как был; доска пустая; следующая запись — новый файл рядом
            const string Broken = "[ { \"Title\": \"оборвано";
            File.WriteAllText(store.FilePath, Broken);
            var loaded = store.Load();
            var aside = Directory.GetFiles(Path.GetDirectoryName(store.FilePath)!, "tickets.json.corrupt-*");
            Check("битый файл — пустая доска, а файл отложен как был", loaded.Count == 0 && !File.Exists(store.FilePath)
                && aside is [var put] && File.ReadAllText(put) == Broken, () => string.Join(", ", aside));
            store.Save(new[] { card });
            Check("…следующая запись — новый файл, отложенный цел", store.Load() is [{ Title: "Принтер" }]
                && Directory.GetFiles(Path.GetDirectoryName(store.FilePath)!, "tickets.json.corrupt-*").Length == 1);

            // 5. формат: файл в нынешних именах читается целиком, и пишутся ровно эти поля
            var old = new TicketStore(Path.Combine(data.Path, "old"));
            File.WriteAllText(old.FilePath, Sample);
            var t = old.Load() is [var one] ? one : null;
            Check("файл в нынешних именах читается целиком", t is
                {
                    Title: "Принтер не печатает", Url: "https://hd/Task/View/702180", Description: "с утра", Priority: TicketPriority.High,
                    Status: TicketStatus.Waiting, IntraserviceId: 702180, ExternalStatus: "Закрыта", KeptOpenStatus: "Закрыта",
                    Creator: "Петрова А.", CreatorPhone: "+7 900 000-00-00", CreatorEmail: "petrova@example.ru", Executors: "Я Сам, Сидоров С.",
                    ExecutorGroup: "Вторая линия", UnreadComments: 3, AssignedToMe: false,
                }
                && t.Id == Guid.Parse("9b2f6c1e-0000-4000-8000-000000000001") && t.CreatedAt == At("2026-09-01T10:00:00+03:00")
                && t.StatusChangedAt == At("2026-09-02T11:00:00+03:00") && t.CompletedAt == At("2026-09-03T12:00:00+03:00")
                && t.LastSyncAt == At("2026-09-04T13:00:00+03:00") && t.CommentsCheckedFor == At("2026-09-05T14:00:00+03:00")
                && t.CommentsSeenAt == At("2026-09-05T14:00:01+03:00")
                && t.Notes is [{ Text: "позвонить" } note] && note.Id == Guid.Parse("9b2f6c1e-0000-4000-8000-000000000002")
                && note.CreatedAt == At("2026-09-06T15:00:00+03:00"), () => File.ReadAllText(old.FilePath));
            static string[] Keys(string file)
            {
                using var doc = JsonDocument.Parse(file);
                return doc.RootElement[0].EnumerateObject().Select(p => p.Name).Order().ToArray();
            }
            if (t is not null)
            {
                old.Save(new[] { t });
                var written = Keys(File.ReadAllText(old.FilePath));
                var sample = Keys(Sample);
                Check("пишутся ровно поля образца (новое поле — в образец; переименованное — потеря данных пользователя)",
                    written.SequenceEqual(sample),
                    () => $"лишние: {string.Join(", ", written.Except(sample))}; пропали: {string.Join(", ", sample.Except(written))}");
            }
        }
        checks.AssertAll();
    }
}
