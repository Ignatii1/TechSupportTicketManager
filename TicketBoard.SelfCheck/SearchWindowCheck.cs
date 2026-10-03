using System.Diagnostics;
using TicketBoard.Models;
using TicketBoard.Services;
using TicketBoard.ViewModels;

namespace TicketBoard.SelfCheck;

/// <summary>Самопроверка окна «Поиск заявок»: настоящий SearchViewModel (запомненные условия и списки, поиск и страницы,
/// «устарел», просмотр, буфер, выгрузка, смена настроек) против поддельного Интрасервиса на loopback. Всё, что viewmodel
/// берёт из WPF-части (доска, карточка, буфер, папка данных), — заглушки в WpfStubs.cs. Сама разметка окна на Linux не
/// запускается — её проверяют только по именам ресурсов и привязок (см. AGENTS.md).</summary>
internal static class SearchWindowCheck
{
    public static void Run() => RunAsync().GetAwaiter().GetResult();

    private static async Task RunAsync()
    {

        var failures = 0;
        void Check(string name, bool ok)
        {
            if (ok) return;
            failures++;
            Console.Error.WriteLine($"  не прошло: {name}");
        }
        static async Task Until(Func<bool> condition, int ms = 8000)
        {
            var sw = Stopwatch.StartNew();
            while (!condition() && sw.ElapsedMilliseconds < ms) await Task.Delay(20);
        }

        var asked = new List<string>();
        var failFilters = false;
        string Row(int id, string name, bool changed = true) =>
            $"{{\"Id\":{id},\"Name\":\"{name}\",\"StatusId\":30,\"ServiceId\":844,\"Type\":\"Запрос\",\"Created\":\"2026-08-07T10:15:08\","
            + (changed ? "\"Changed\":\"2026-08-27T15:54:43\"," : "") + "\"Creator\":\"Петрова А.\",\"Executors\":\"Максимов М. С.\"}";
        const string Tail = "\"Statuses\":[{\"Id\":30,\"Name\":\"Закрыта\"}],\"Services\":[{\"Id\":844,\"Name\":\"Приложение на ТСД\"}]";

        (int, string) Respond(string target)
        {
            lock (asked) asked.Add(target);
            if (target.StartsWith("/api/user?getcurrentuserinfo=true")) return (200, """{"Id":7,"Name":"Я Сам"}""");
            if (target.StartsWith("/api/user?"))
                return (200, Uri.UnescapeDataString(target).Contains("search=максимов")
                    ? """{"Users":[{"Id":38472,"Name":"Максимов М. С."}],"Paginator":{"Count":1,"Page":1,"PageCount":1}}"""
                    : """{"Users":[],"Paginator":{"Count":0,"Page":1,"PageCount":1}}""");
            if (target.StartsWith("/api/taskstatus"))
                return (200, """[{"Id":31,"Name":"Открыта"},{"Id":29,"Name":"Выполнена","IsFixed":true},{"Id":30,"Name":"Закрыта"}]""");
            if (target.StartsWith("/api/service"))
                return (200, """{"ServiceList":{"Services":[{"Id":840,"Name":"ТСД","Path":"840|"},{"Id":844,"Name":"Приложение на ТСД","Path":"840|844|"}],"Paginator":{"Count":2,"Page":1,"PageCount":1}}}""");
            if (target.StartsWith("/api/tasktype"))
                return (200, """{"TaskTypeList":{"TaskTypes":[{"Id":1009,"Name":"Запрос на обслуживание"},{"Id":3,"Name":"Инцидент"}],"Paginator":{"Count":2,"Page":1,"PageCount":1}}}""");
            if (target.StartsWith("/api/filter"))
                return failFilters ? (404, "{}") : (200, """[{"Id":45,"IsDefault":false,"Name":"Мои заявки"}]""");
            if (target.StartsWith("/api/tasklifetime"))
                return (200, """{"TaskLifetimeList":{"TaskLifetimes":[{"Date":"2026-08-07T10:15:00","Editor":"Петрова А.","StatusId":30,"Comments":"текст"}],"Statuses":[{"Id":30,"Name":"Закрыта"}],"Paginator":{"Page":1,"PageCount":1}}}""");
            if (target.StartsWith("/api/task/"))
            {
                var id = int.Parse(target["/api/task/".Length..].Split('?')[0]);
                return (200, $"{{\"Task\":{Row(id, "Заявка " + id, changed: id != 901)}}}");
            }
            if (target.StartsWith("/api/task?"))
            {
                var q = Uri.UnescapeDataString(target);
                if (q.Contains("search=slow")) { Thread.Sleep(700); return (200, $"{{\"Tasks\":[{Row(801, "Медленная")}],{Tail},\"Paginator\":{{\"Count\":1,\"Page\":1,\"PageCount\":1}}}}"); }
                if (q.Contains("search=fast")) return (200, $"{{\"Tasks\":[{Row(802, "Быстрая")}],{Tail},\"Paginator\":{{\"Count\":1,\"Page\":1,\"PageCount\":1}}}}");
                if (q.Contains("search=nochanged")) return (200, $"{{\"Tasks\":[{Row(901, "Без даты", changed: false)}],{Tail},\"Paginator\":{{\"Count\":1,\"Page\":1,\"PageCount\":1}}}}");
                return q.Contains("page=2")
                    ? (200, $"{{\"Tasks\":[{Row(703, "Третья")}],{Tail},\"Paginator\":{{\"Count\":3,\"Page\":2,\"PageCount\":2,\"PageSize\":2}}}}")
                    : (200, $"{{\"Tasks\":[{Row(701, "Первая")},{Row(702, "Вторая")}],{Tail},\"Paginator\":{{\"Count\":3,\"Page\":1,\"PageCount\":2,\"PageSize\":2}}}}");
            }
            return (404, "{}");
        }

        var (listener, port) = FakeIntraservice.Start(Respond);
        var dataDir = Path.Combine(Path.GetTempPath(), $"tb-vm-{Guid.NewGuid():N}");
        Directory.CreateDirectory(dataDir);
        App.DataDir = dataDir;
        try
        {
            AppSettings NewSettings() => new() { IntraserviceBaseUrl = $"http://127.0.0.1:{port}", IntraserviceLogin = "u" };
            HttpIntraserviceClient NewClient(AppSettings s) => new(s.IntraserviceBaseUrl, s.IntraserviceLogin, "p");
            int Count(string prefix) { lock (asked) return asked.Count(t => t.StartsWith(prefix)); }
            string Last(string prefix) { lock (asked) return asked.Last(t => t.StartsWith(prefix)); }

            // 1. запомненные условия возвращаются в списках после загрузки справочников
            // в WPF очистка ItemsSource сбрасывает SelectedItem, а двусторонняя привязка кладёт null в модель — повторим это
            void WireLikeComboBox(SearchViewModel v)
            {
                v.StatusChoices.CollectionChanged += (_, e) => { if (e.Action == System.Collections.Specialized.NotifyCollectionChangedAction.Reset) v.SelectedStatus = null; };
                v.ServiceChoices.CollectionChanged += (_, e) => { if (e.Action == System.Collections.Specialized.NotifyCollectionChangedAction.Reset) v.SelectedService = null; };
                v.TypeChoices.CollectionChanged += (_, e) => { if (e.Action == System.Collections.Specialized.NotifyCollectionChangedAction.Reset) v.SelectedType = null; };
                v.SavedChoices.CollectionChanged += (_, e) => { if (e.Action == System.Collections.Specialized.NotifyCollectionChangedAction.Reset) v.SelectedSaved = null; };
            }
            var s1 = NewSettings();
            s1.LastSearch = new SearchFilter(Words: "vpn", Status: SearchStatus.One, StatusId: 29, ServiceId: 844, TypeId: 1009,
                SavedFilterId: 45, ChangedFrom: "01.01.2026");
            var vm = new SearchViewModel(new MainViewModel(), s1, NewClient(s1));
            WireLikeComboBox(vm);
            Check("до загрузки слова и дата на месте, списки ещё пустые", vm.Words == "vpn" && vm.ChangedFrom == "01.01.2026" && vm.SelectedService?.Id == 0);
            await vm.OpenAsync();
            Check("статус «Выполнена» выбран", vm.SelectedStatus is { Kind: SearchStatus.One, Id: 29 });
            Check("сервис, тип и сохранённый фильтр выбраны", vm.SelectedService?.Id == 844 && vm.SelectedType?.Id == 1009 && vm.SelectedSaved?.Id == 45);
            Check("вложенный сервис — со сдвигом", vm.ServiceChoices.Any(c => c.Id == 844 && c.Label.StartsWith("   ")));
            Check("справочники без ошибок — подписи под условиями пусты", vm.Notes == "");

            // 2. поиск: условия уходят серверу, имена находятся и показаны, страницы, просмотр списка
            vm.ClearCommand.Execute(null);
            vm.Mine = true;
            vm.Executor = "максимов";
            vm.Words = "принтер";
            await vm.SearchCommand.ExecuteAsync(null);
            var list = Last("/api/task?");
            Check("в запросе я и Максимов, слова, count=all, include=service", list.Contains("ExecutorIds=7,38472&") && list.Contains("search=%D0%BF") && list.Contains("count=all") && list.Contains("include=status,service"));
            Check("первая страница: 2 из 3, есть «ещё»", vm.Results.Count == 2 && vm.Total == 3 && vm.HasMore && vm.Message == "Найдено: 3 · показано 2");
            Check("кто нашёлся по имени — над списком", vm.Matched == "Исполнитель: Максимов М. С.");
            Check("у строки есть сервис (из блока Services) и тип", vm.Results[0].Service == "Приложение на ТСД · Запрос");
            Check("после поиска список не «устарел», выгрузка найденного доступна", !vm.IsStale && vm.ExportFoundCommand.CanExecute(null) && vm.StaleHint == "");
            await vm.ShowMoreCommand.ExecuteAsync(null);
            Check("«ещё»: третья заявка добавлена, больше нет", vm.Results.Count == 3 && !vm.HasMore && vm.Message == "Найдено: 3");
            vm.Words = "другое";
            Check("правка условий: список устарел, выгрузка найденного выключена, подсказка видна", vm.IsStale && !vm.ExportFoundCommand.CanExecute(null) && vm.StaleHint != "");
            vm.Limit = "10";
            await vm.SearchCommand.ExecuteAsync(null);
            Check("новый поиск снимает «устарел»; число для выгрузки поиск не трогает", !vm.IsStale);
            vm.Limit = "20";
            Check("правка «не больше, заявок» список не устаревает", !vm.IsStale);

            // 3. не все справочники загрузились: выбор человека не теряется при повторной загрузке, «устарел» не включается
            failFilters = true;
            var s3 = NewSettings();
            var vm3 = new SearchViewModel(new MainViewModel(), s3, NewClient(s3));
            WireLikeComboBox(vm3);
            var servicesBefore = Count("/api/service");
            await vm3.OpenAsync();
            Check("сообщено, что не загрузились сохранённые фильтры", vm3.Notes.Contains("сохранённые фильтры"));
            vm3.SelectedService = vm3.ServiceChoices.First(c => c.Id == 840);
            vm3.SelectedType = vm3.TypeChoices.First(c => c.Id == 3);
            vm3.Words = "vpn";
            await vm3.SearchCommand.ExecuteAsync(null);
            Check("поиск без справочника фильтров работает", vm3.Results.Count == 2 && !vm3.IsStale);
            failFilters = false;
            await vm3.OpenAsync();
            Check("повторное открытие перечитало справочники", Count("/api/service") == servicesBefore + 2 && vm3.SavedChoices.Count == 2 && vm3.Notes == "");
            Check("выбор человека остался", vm3.SelectedService?.Id == 840 && vm3.SelectedType?.Id == 3);
            Check("перезагрузка списков не сделала список устаревшим", !vm3.IsStale && vm3.ExportFoundCommand.CanExecute(null));
            var before = Count("/api/service");
            await vm3.OpenAsync();
            Check("всё загружено — открытие больше не ходит за справочниками", Count("/api/service") == before);

            // 4. сохранение настроек: тот же сервер — всё на месте, другой — забыто
            var resultsBefore = vm3.Results.Count;
            vm3.ApplySettings(NewClient(s3));
            Check("тот же сервер и логин: список и выбор остались", vm3.Results.Count == resultsBefore && vm3.SelectedService?.Id == 840);
            s3.IntraserviceBaseUrl = $"http://localhost:{port}";
            vm3.ApplySettings(NewClient(s3));
            Check("другой адрес: список, итог и подпись забыты", vm3.Results.Count == 0 && vm3.Total == 0 && vm3.Message == "" && !vm3.ExportFoundCommand.CanExecute(null));

            // 5. просмотр, буфер обмена, выгрузка выбранных и найденных
            await vm.SearchCommand.ExecuteAsync(null);
            vm.SetSelection(new[] { vm.Results[0] }, vm.Results[0]);
            await Until(() => !vm.IsPreviewBusy && vm.PreviewText.Length > 0);
            Check("просмотр: текст для агента с свойствами и перепиской", vm.PreviewText.StartsWith("---\nid: 701\n") && vm.PreviewText.Contains("\ntype: \"Запрос\"\n") && vm.PreviewText.Contains("текст"));
            Check("заголовок просмотра — номер и название", vm.PreviewTitle == "#701 Первая" && vm.PreviewHint == "");
            var cardsBefore = Count("/api/task/");
            vm.SetSelection(new[] { vm.Results[0] }, vm.Results[0]);
            await Until(() => !vm.IsPreviewBusy && vm.PreviewText.Length > 0);
            Check("тот же просмотр не ходит за заявкой дважды", Count("/api/task/") == cardsBefore);
            vm.SetSelection(vm.Results.Take(2).ToList(), null);
            Check("выбрано две: подписи кнопок", vm.CopyLabel == "Копировать (2)" && vm.ExportSelectedLabel == "Выгрузить выбранные (2)");
            await vm.CopyCommand.ExecuteAsync(null);
            Check("в буфере обе заявки подряд", ClipboardWatcher.Last is { } clip && clip.Contains("id: 701") && clip.Contains("id: 702") && vm.WorkMessage.Contains("заявок: 2"));
            var export = Path.Combine(dataDir, "out");
            vm.Folder = export;
            await vm.ExportSelectedCommand.ExecuteAsync(null);
            Check("выбранные выгружены файлами", Directory.GetFiles(Path.Combine(export, "tickets"), "*.md").Length == 2 && vm.WorkMessage.StartsWith("Готово. Выбрано — 2"));
            Check("папка запомнена в настройках", s1.KnowledgeDir == export);
            await vm.ExportFoundCommand.ExecuteAsync(null);
            Check("найденные (3) выгружены: к двум имеющимся добавилась одна", vm.WorkMessage.StartsWith("Готово. По отбору — 3") && vm.WorkMessage.Contains("новых файлов 1") && vm.WorkMessage.Contains("без изменений 2"));

            // 6. просмотр без даты изменения не берётся из кэша
            vm.ClearCommand.Execute(null);
            vm.Words = "nochanged";
            await vm.SearchCommand.ExecuteAsync(null);
            vm.SetSelection(new[] { vm.Results[0] }, vm.Results[0]);
            await Until(() => !vm.IsPreviewBusy && vm.PreviewText.Length > 0);
            var c1 = Count("/api/task/901");
            vm.SetSelection(new[] { vm.Results[0] }, vm.Results[0]);
            await Until(() => !vm.IsPreviewBusy && vm.PreviewText.Length > 0);
            Check("без Changed заявку читают заново", Count("/api/task/901") == c1 + 1);

            // 7. второй поиск, пока идёт первый: остаётся только результат второго
            vm.ClearCommand.Execute(null);
            vm.Words = "slow";
            var slow = vm.SearchCommand.ExecuteAsync(null);
            await Task.Delay(100);
            Check("кнопка «Найти» доступна, пока идёт поиск", vm.IsBusy && vm.SearchCommand.CanExecute(null));
            vm.Words = "fast";
            var fast = vm.SearchCommand.ExecuteAsync(null);
            await Task.WhenAll(slow, fast);
            Check("в списке только быстрая, поиск не «занят»", vm.Results.Count == 1 && vm.Results[0].Id == 802 && !vm.IsBusy);

            // 8a. «Сбросить» устаревляет список и тогда, когда поменялись одни выпадающие списки
            vm.ClearCommand.Execute(null);
            vm.SelectedService = vm.ServiceChoices.First(c => c.Id == 840);
            await vm.SearchCommand.ExecuteAsync(null);
            Check("поиск только по сервису: список свежий", !vm.IsStale && vm.Results.Count > 0);
            vm.ClearCommand.Execute(null);
            Check("после «Сбросить» сервис снят, текстовые поля те же — всё равно устарел", vm.SelectedService?.Id == 0 && vm.IsStale && !vm.ExportFoundCommand.CanExecute(null));

            // 8. условия сохраняются, сброс устаревит список
            Check("последний поиск запомнен (сервис 840, слов нет)", s1.LastSearch is { Words: "", ServiceId: 840 });
            vm.ClearCommand.Execute(null);
            Check("после «Сбросить» условия пусты, а список прежний — устарел", vm.Words == "" && vm.IsStale);
        }
        finally
        {
            listener.Stop();
            try { Directory.Delete(dataDir, recursive: true); } catch (IOException) { }
        }
        Debug.Assert(failures == 0, $"Окно поиска: не прошло проверок — {failures} (список выше)");
    }
}
