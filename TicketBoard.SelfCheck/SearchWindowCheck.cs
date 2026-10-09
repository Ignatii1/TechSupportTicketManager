using System.Diagnostics;
using System.Globalization;
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
    public static void Run() => SingleThread.Run(RunAsync);

    private static async Task RunAsync(SingleThread pump)
    {
        var checks = new CheckSet("окно поиска");
        void Check(string name, bool ok, Func<string>? details = null) => checks.Check(name, ok, details);

        var asked = new List<string>();
        var failFilters = false;
        var failServices = false;
        var servicesFromTickets = false;   // справочник сервисов учётной записи не отдают (403), назначенных нет — только из заявок
        var servicesAssigned = false;      // справочник пуст (200 без сервисов) — только те, на которые назначен
        var servicesNone = false;          // сервисов нет нигде, и без ошибок: ни в справочнике, ни назначенных, ни в заявках
        var refuseNoCount = false;         // список заявок без счёта (count=false) — 400
        var servicesBlip = false;          // справочник сервисов на миг недоступен (503) — это не отказ
        var services500 = false;           // справочник отвечает 500 — так бывает и с запретом: нужны запасные списки
        var failTypes = false;
        var slowStatuses = false;
        var slowCards = false;
        var failStatuses = false;
        string Row(int id, string name, bool changed = true) =>
            $"{{\"Id\":{id},\"Name\":\"{name}\",\"StatusId\":30,\"ServiceId\":844,\"Type\":\"Запрос\",\"Created\":\"2026-08-07T10:15:08\","
            + (changed ? "\"Changed\":\"2026-08-27T15:54:43\"," : "") + "\"Creator\":\"Петрова А.\",\"Executors\":\"Максимов М. С.\"}";
        const string Tail = "\"Statuses\":[{\"Id\":30,\"Name\":\"Закрыта\"}],\"Services\":[{\"Id\":844,\"Name\":\"Приложение на ТСД\"}]";

        (int, string) Respond(string target)
        {
            lock (asked) asked.Add(target);
            if (target.StartsWith("/api/user?getcurrentuserinfo=true")) return (200, """{"Id":7,"Name":"Я Сам"}""");
            if (target.StartsWith("/api/user?"))
            {
                var q = Uri.UnescapeDataString(target);
                if (q.Contains("search=медленный"))   // ответ о сотруднике приходит не сразу: за это время условия успевают править
                {
                    Thread.Sleep(300);
                    return (200, """{"Users":[{"Id":9,"Name":"Медленный М."}],"Paginator":{"Count":1,"Page":1,"PageCount":1}}""");
                }
                return (200, q.Contains("search=максимов")
                    ? """{"Users":[{"Id":38472,"Name":"Максимов М. С."}],"Paginator":{"Count":1,"Page":1,"PageCount":1}}"""
                    : """{"Users":[],"Paginator":{"Count":0,"Page":1,"PageCount":1}}""");
            }
            if (target.StartsWith("/api/taskstatus"))
            {
                if (slowStatuses) Thread.Sleep(400);
                if (failStatuses) return (503, "{}");
                return (200, """[{"Id":31,"Name":"Открыта"},{"Id":29,"Name":"Выполнена","IsFixed":true},{"Id":30,"Name":"Закрыта"}]""");
            }
            if (target.StartsWith("/api/service") && servicesBlip) return (503, "{}");
            if (target.StartsWith("/api/service") && services500 && !target.Contains("for=filtertasks")) return (500, """{"Message":"An error has occurred."}""");
            if (target.StartsWith("/api/service") && servicesNone)
                return (200, """{"ServiceList":{"Services":[],"Paginator":{"Count":0,"Page":1,"PageCount":0}}}""");
            if (target.StartsWith("/api/service") && servicesAssigned)
                return target.Contains("for=filtertasks")
                    ? (200, """{"ServiceList":{"Services":[{"Id":840,"Name":"ТСД","Path":"840|"},{"Id":844,"Name":"Приложение на ТСД","Path":"840|844|"}],"Paginator":{"Count":2,"Page":1,"PageCount":1}}}""")
                    : (200, """{"ServiceList":{"Services":[],"Paginator":{"Count":0,"Page":1,"PageCount":0}}}""");
            if (target.StartsWith("/api/service") && servicesFromTickets)
                return target.Contains("for=filtertasks") ? (200, """{"ServiceList":{"Services":[],"Paginator":{"Count":0,"Page":1,"PageCount":0}}}""")
                    : (403, """{"Message":"Нет прав на просмотр списка сервисов"}""");
            if (target.StartsWith("/api/service"))
                return failServices ? (404, "{}") : (200, """{"ServiceList":{"Services":[{"Id":840,"Name":"ТСД","Path":"840|"},{"Id":844,"Name":"Приложение на ТСД","Path":"840|844|"}],"Paginator":{"Count":2,"Page":1,"PageCount":1}}}""");
            if (target.StartsWith("/api/tasktype"))
                return failTypes ? (404, "{}") : (200, """{"TaskTypeList":{"TaskTypes":[{"Id":1009,"Name":"Запрос на обслуживание"},{"Id":3,"Name":"Инцидент"}],"Paginator":{"Count":2,"Page":1,"PageCount":1}}}""");
            if (target.StartsWith("/api/filter"))
                return failFilters ? (404, "{}") : (200, """[{"Id":45,"IsDefault":false,"Name":"Мои заявки"}]""");
            if (target.StartsWith("/api/tasklifetime"))
                return (200, """{"TaskLifetimeList":{"TaskLifetimes":[{"Date":"2026-08-07T10:15:00","Editor":"Петрова А.","StatusId":30,"Comments":"текст"}],"Statuses":[{"Id":30,"Name":"Закрыта"}],"Paginator":{"Page":1,"PageCount":1}}}""");
            if (target.StartsWith("/api/task/"))
            {
                if (slowCards) Thread.Sleep(300);
                var id = int.Parse(target["/api/task/".Length..].Split('?')[0]);
                return (200, $"{{\"Task\":{Row(id, "Заявка " + id, changed: id != 901)}}}");
            }
            if (target.StartsWith("/api/task?"))
            {
                var q = Uri.UnescapeDataString(target);
                // сервисы из заявок: блок Services страницы последних заявок (servicesNone — блока нет)
                if (q.Contains("fields=Id,Name,ServiceId") && refuseNoCount && q.Contains("count=false"))
                    return (400, """{"errors":{"count":["The value 'false' is not valid."]},"status":400}""");
                if (q.Contains("fields=Id,Name,ServiceId"))
                    return failServices ? (503, "{}") : servicesNone ? (200, """{"Tasks":[],"Paginator":{"Page":1,"HasNextPage":false}}""")
                        : (200, """{"Tasks":[{"Id":701,"Name":"П","ServiceId":844},{"Id":702,"Name":"В","ServiceId":850}],"Services":[{"Id":850,"Name":"Принтеры"},{"Id":844,"Name":"Приложение на ТСД","Path":"840|844|"}],"Paginator":{"Page":1,"HasNextPage":false}}""");
                // счёт по умолчанию досчитал до потолка: «1 000 или больше»
                if (q.Contains("search=thousand")) return (200, $"{{\"Tasks\":[{Row(861, "Тысячная")}],{Tail},\"Paginator\":{{\"Count\":1000,\"Page\":1,\"PageCount\":1000,\"PageSize\":1}}}}");
                if (q.Contains("search=slow")) { Thread.Sleep(700); return (200, $"{{\"Tasks\":[{Row(801, "Медленная")}],{Tail},\"Paginator\":{{\"Count\":1,\"Page\":1,\"PageCount\":1}}}}"); }
                if (q.Contains("search=fast")) return (200, $"{{\"Tasks\":[{Row(802, "Быстрая")}],{Tail},\"Paginator\":{{\"Count\":1,\"Page\":1,\"PageCount\":1}}}}");
                // найдено 2500 (счёт сервера), на странице одна: выгрузка «всех» тут — повод для вопроса
                if (q.Contains("search=huge")) return (200, $"{{\"Tasks\":[{Row(851, "Огромная")}],{Tail},\"Paginator\":{{\"Count\":2500,\"Page\":1,\"PageCount\":2500,\"PageSize\":1}}}}");
                if (q.Contains("search=nochanged")) return (200, $"{{\"Tasks\":[{Row(901, "Без даты", changed: false)}],{Tail},\"Paginator\":{{\"Count\":1,\"Page\":1,\"PageCount\":1}}}}");
                return q.Contains("page=2")
                    ? (200, $"{{\"Tasks\":[{Row(703, "Третья")}],{Tail},\"Paginator\":{{\"Count\":3,\"Page\":2,\"PageCount\":2,\"PageSize\":2}}}}")
                    : (200, $"{{\"Tasks\":[{Row(701, "Первая")},{Row(702, "Вторая")}],{Tail},\"Paginator\":{{\"Count\":3,\"Page\":1,\"PageCount\":2,\"PageSize\":2}}}}");
            }
            return (404, "{}");
        }

        var (listener, port) = FakeIntraservice.Start(Respond);
        using (var data = new TempDataDir("tb-vm"))
        {
        var dataDir = data.Path;
        try
        {
            AppSettings NewSettings() => new() { IntraserviceBaseUrl = $"http://127.0.0.1:{port}", IntraserviceLogin = "u" };
            HttpIntraserviceClient NewClient(AppSettings s) => new(s.IntraserviceBaseUrl, s.IntraserviceLogin, "p");
            // доска — настоящая, пустая, без сервера: окну поиска от неё нужны только «что уже на доске» и «+ На доску»
            var boards = 0;
            MainViewModel Board()
            {
                var dir = Path.Combine(dataDir, $"board{++boards}");
                Directory.CreateDirectory(dir);
                var s = new AppSettings();
                var board = new MainViewModel(new TicketStore(dir), s, new IntraserviceLinkParser(s), null);
                foreach (var c in board.Columns) pump.WatchItems(c.Items);   // «+ На доску» — только в потоке окна
                return board;
            }
            // привязанные списки окна меняются только в его потоке — иначе WPF бросил бы исключение
            SearchViewModel Watched(SearchViewModel v)
            {
                pump.WatchItems(v.Results);
                pump.WatchItems(v.StatusChoices);
                pump.WatchItems(v.ServiceChoices);
                pump.WatchItems(v.TypeChoices);
                pump.WatchItems(v.SavedChoices);
                return v;
            }
            int Count(string prefix) { lock (asked) return asked.Count(t => t.StartsWith(prefix)); }
            // страница заявок ради сервисов (запасной список): узнаётся по полям строки
            int ServicePages() { lock (asked) return asked.Count(t => t.Contains("fields=Id,Name,ServiceId")); }
            string ServicePage() { lock (asked) return asked.Last(t => t.Contains("fields=Id,Name,ServiceId")); }
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
            var vm = Watched(new SearchViewModel(Board(), s1, NewClient(s1)));
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
            Check("в запросе я и Максимов, слова, include=service, без count (count=all живой сервер не принимает)", list.Contains("ExecutorIds=7,38472&") && list.Contains("search=%D0%BF") && !list.Contains("count=") && list.Contains("include=status,service"));
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
            var vm3 = Watched(new SearchViewModel(Board(), s3, NewClient(s3)));
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
            var filtersBefore = Count("/api/filter");
            await vm3.OpenAsync();
            Check("повторное открытие спросило только про то, что не загрузилось", Count("/api/service") == servicesBefore + 1
                && Count("/api/filter") == filtersBefore + 1 && vm3.SavedChoices.Count == 2 && vm3.Notes == "");
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
            Check("другой адрес: сервис и тип прежнего сервера сняты", vm3.SelectedService?.Id == 0 && vm3.SelectedType?.Id == 0 && vm3.ServiceChoices.Count == 1);
            await vm3.OpenAsync();
            Check("справочники нового сервера прочитаны, выбор — «любой»", vm3.ServiceChoices.Count == 3 && vm3.SelectedService?.Id == 0 && vm3.SelectedType?.Id == 0);

            // 5. просмотр, буфер обмена, выгрузка выбранных и найденных
            await vm.SearchCommand.ExecuteAsync(null);
            vm.SetSelection(new[] { vm.Results[0] }, vm.Results[0]);
            Check("просмотр дождался текста", await SingleThread.Until(() => !vm.IsPreviewBusy && vm.PreviewText.Length > 0));
            Check("просмотр: текст для агента с свойствами и перепиской", vm.PreviewText.StartsWith("---\nid: 701\n") && vm.PreviewText.Contains("\ntype: \"Запрос\"\n") && vm.PreviewText.Contains("текст"));
            Check("заголовок просмотра — номер и название", vm.PreviewTitle == "#701 Первая" && vm.PreviewHint == "");
            var cardsBefore = Count("/api/task/");
            vm.SetSelection(new[] { vm.Results[0] }, vm.Results[0]);
            Check("просмотр дождался текста", await SingleThread.Until(() => !vm.IsPreviewBusy && vm.PreviewText.Length > 0));
            Check("тот же просмотр не ходит за заявкой дважды", Count("/api/task/") == cardsBefore);
            vm.SetSelection(vm.Results.Take(2).ToList(), null);
            Check("выбрано две: подписи кнопок", vm.CopyLabel == "Копировать (2)" && vm.ExportSelectedLabel == "Выгрузить выбранные (2)");
            await vm.CopyCommand.ExecuteAsync(null);
            Check("в буфере обе заявки подряд", ClipboardWatcher.Last is { } clip && clip.Contains("id: 701") && clip.Contains("id: 702") && vm.WorkMessage.Contains("заявок: 2"));
            var export = Path.Combine(dataDir, "out");
            vm.Folder = export;
            await vm.ExportSelectedCommand.ExecuteAsync(null);
            Check("выбранные выгружены файлами — в папки месяцев создания", Directory.GetFiles(Path.Combine(export, "tickets"), "*.md", SearchOption.AllDirectories)
                .Count(f => Path.GetFileName(f) != "_index.md") == 2 && Directory.Exists(Path.Combine(export, "tickets", "2026-08")) && vm.WorkMessage.StartsWith("Готово. Выбрано — 2"));
            Check("папка запомнена в настройках", s1.KnowledgeDir == export);
            await vm.ExportFoundCommand.ExecuteAsync(null);
            Check("найденные (3) выгружены: к двум имеющимся добавилась одна", vm.WorkMessage.StartsWith("Готово. По отбору — 3") && vm.WorkMessage.Contains("новых файлов 1") && vm.WorkMessage.Contains("без изменений 2"));

            // 6. просмотр без даты изменения не берётся из кэша
            vm.ClearCommand.Execute(null);
            vm.Words = "nochanged";
            await vm.SearchCommand.ExecuteAsync(null);
            vm.SetSelection(new[] { vm.Results[0] }, vm.Results[0]);
            Check("просмотр дождался текста", await SingleThread.Until(() => !vm.IsPreviewBusy && vm.PreviewText.Length > 0));
            var c1 = Count("/api/task/901");
            vm.SetSelection(new[] { vm.Results[0] }, vm.Results[0]);
            Check("просмотр дождался текста", await SingleThread.Until(() => !vm.IsPreviewBusy && vm.PreviewText.Length > 0));
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

            // 9. правят условия, пока разбираются с именем сотрудника: список — по прежним, «найденные» выключено, подсказка видна
            var sA = NewSettings();
            var vmA = Watched(new SearchViewModel(Board(), sA, NewClient(sA)));
            WireLikeComboBox(vmA);
            await vmA.OpenAsync();
            vmA.ClearCommand.Execute(null);
            vmA.Executor = "медленный";
            var running = vmA.SearchCommand.ExecuteAsync(null);
            await Task.Delay(100);
            vmA.Words = "правка";
            await running;
            Check("список получен, но условия уже другие — устарел", vmA.Results.Count == 2 && vmA.IsStale && !vmA.ExportFoundCommand.CanExecute(null) && vmA.StaleHint != "");

            // 10. справочники сервисов и типов не загрузились: запомненные номера не стираются; сервис — строкой «Сервис №…» и в
            // поиске (номер серверу понятен и без справочника, иначе поиск молча стал бы шире), тип — нет; вернулись — выбор на месте
            failServices = true;
            failTypes = true;
            var sB = NewSettings();
            sB.LastSearch = new SearchFilter(Words: "vpn", ServiceId: 844, TypeId: 1009);
            var vmB = Watched(new SearchViewModel(Board(), sB, NewClient(sB)));
            WireLikeComboBox(vmB);
            await vmB.OpenAsync();
            Check("сервис — строкой «список не загрузился», типы — «любой»; сказано, что не загрузилось: о сервисах — под их списком, с обеими причинами",
                vmB.ServiceChoices.Count == 2 && vmB.SelectedService is { Id: 844 } kept && kept.Label.Contains("список не загрузился") && vmB.TypeChoices.Count == 1
                && vmB.ServiceNote.StartsWith("Список сервисов не загрузился: ") && vmB.ServiceNote.Contains("сервисы из заявок — ошибка сервера (HTTP 503)")
                && vmB.Notes.Contains("типы") && !vmB.Notes.Contains("сервисы"),
                () => vmB.ServiceNote + "\n" + vmB.Notes + "\n" + string.Join(", ", vmB.ServiceChoices.Select(c => $"{c.Id} {c.Label}")));
            await vmB.SearchCommand.ExecuteAsync(null);
            Check("поиск ушёл с запомненным сервисом (без вложенных — о них сказано) и без типа", Last("/api/task?").Contains("ServiceIds=844&")
                && !Last("/api/task?").Contains("TypeIds=") && vmB.Matched.Contains("вложенные сервисы в поиск не вошли"), () => Last("/api/task?") + "\n" + vmB.Matched);
            Check("а запомненные номера не стёрлись", sB.LastSearch is { ServiceId: 844, TypeId: 1009 });
            failServices = false;
            failTypes = false;
            await vmB.OpenAsync();
            Check("справочники пришли — прежний выбор вернулся, подписи пусты", vmB.SelectedService?.Id == 844 && vmB.SelectedType?.Id == 1009
                && vmB.ServiceNote == "" && vmB.Notes == "");

            // 10а. справочник сервисов учётной записи не отдают (403), назначенных на неё нет: сервисы — из последних заявок, и так
            // и сказано под списком; запомненный сервис выбран, поиск по нему уходит серверу
            servicesFromTickets = true;
            try
            {
                var sT = NewSettings();
                sT.LastSearch = new SearchFilter(ServiceId: 850);
                var vmT = Watched(new SearchViewModel(Board(), sT, NewClient(sT)));
                WireLikeComboBox(vmT);
                var ticketPagesBefore = ServicePages();
                await vmT.OpenAsync();
                Check("сервисы из заявок: «любой» и два, по имени", vmT.ServiceChoices.Select(c => c.Id).SequenceEqual(new[] { 0, 844, 850 }),
                    () => string.Join(", ", vmT.ServiceChoices.Select(c => $"{c.Id} {c.Label}")));
                Check("под списком — откуда он и почему неполный", vmT.ServiceNote.Contains("нет доступа (HTTP 403)") && vmT.ServiceNote.Contains("последних 2 заявок")
                    && vmT.Notes == "", () => vmT.ServiceNote);
                Check("из заявок — одна страница, без счёта, по изменению, с архивными", ServicePage()
                    == "/api/task?archive=true&inactive=true&include=status,service&count=false&sort=Changed%20desc&pagesize=1000&page=1&fields=Id,Name,ServiceId"
                    && ServicePages() == ticketPagesBefore + 1, ServicePage);
                Check("запомненный сервис выбран", vmT.SelectedService?.Id == 850);
                await vmT.SearchCommand.ExecuteAsync(null);
                Check("поиск по нему ушёл серверу", Last("/api/task?").Contains("ServiceIds=850&"), () => Last("/api/task?"));
                Check("и сказано, что вложенные — только из неполного списка", vmT.Matched.Contains("Список сервисов неполный"), () => vmT.Matched);
                Check("в неполном списке — без отступов (родителя в нём нет)", vmT.ServiceChoices.First(c => c.Id == 844).Label == "Приложение на ТСД");
                vmT.WithChildren = false;
                await vmT.SearchCommand.ExecuteAsync(null);
                Check("без вложенных — об этом ни слова", !vmT.Matched.Contains("Список сервисов неполный"), () => vmT.Matched);

                // count=false сервер не принял (400) — та же страница со счётом, список сервисов всё равно есть
                refuseNoCount = true;
                var sNc = NewSettings();
                var vmNc = Watched(new SearchViewModel(Board(), sNc, NewClient(sNc)));
                var pagesBefore = ServicePages();
                await vmNc.OpenAsync();
                Check("count=false отклонён — со счётом, сервисы есть", vmNc.ServiceChoices.Count == 3 && ServicePages() == pagesBefore + 2
                    && !ServicePage().Contains("count="), () => ServicePage() + " | " + vmNc.ServiceNote);
            }
            finally { servicesFromTickets = false; refuseNoCount = false; }

            // 10а2. запомненный сервис, которого в неполном списке нет, — отдельной строкой и в поиске, а не «любой»
            servicesFromTickets = true;
            try
            {
                var sRem = NewSettings();
                sRem.LastSearch = new SearchFilter(ServiceId: 900);
                var vmRem = Watched(new SearchViewModel(Board(), sRem, NewClient(sRem)));
                WireLikeComboBox(vmRem);
                await vmRem.OpenAsync();
                Check("запомненный №900 — своей строкой и выбран", vmRem.SelectedService is { Id: 900 } m900 && m900.Label.Contains("нет в этом списке"),
                    () => string.Join(", ", vmRem.ServiceChoices.Select(c => $"{c.Id} {c.Label}")));
                await vmRem.SearchCommand.ExecuteAsync(null);
                Check("поиск — по нему", Last("/api/task?").Contains("ServiceIds=900&"), () => Last("/api/task?"));
            }
            finally { servicesFromTickets = false; }

            // 10а3. справочник на миг недоступен (503): это не отказ — запасных списков не просим, следующее открытие спросит снова
            servicesBlip = true;
            var sBl = NewSettings();
            var vmBl = Watched(new SearchViewModel(Board(), sBl, NewClient(sBl)));
            var blipPages = ServicePages();
            int Assigned() { lock (asked) return asked.Count(t => t.Contains("for=filtertasks")); }
            var blipAssigned = Assigned();
            await vmBl.OpenAsync();
            Check("503 — без запасных списков, причина под списком", ServicePages() == blipPages && Assigned() == blipAssigned
                && vmBl.ServiceChoices.Count == 1 && vmBl.ServiceNote.StartsWith("Список сервисов не загрузился: ошибка сервера (HTTP 503)"), () => vmBl.ServiceNote);
            servicesBlip = false;
            await vmBl.OpenAsync();
            Check("сервер ожил — весь справочник, подписи нет", vmBl.ServiceChoices.Count == 3 && vmBl.ServiceNote == "", () => vmBl.ServiceNote);
            // а 500 — не сбой, который пройдёт: запасные списки просятся
            services500 = true;
            var s500 = NewSettings();
            var vm500 = Watched(new SearchViewModel(Board(), s500, NewClient(s500)));
            var assigned500 = Assigned();
            await vm500.OpenAsync();
            services500 = false;
            Check("500 на справочнике — назначенные сервисы", Assigned() == assigned500 + 1 && vm500.ServiceChoices.Count == 3
                && vm500.ServiceNote.Contains("(HTTP 500)") && vm500.ServiceNote.Contains("на которые вы назначены"), () => vm500.ServiceNote);

            // 10б. справочник пришёл пустым (200, без сервисов) — это не «сервисов нет»: берутся назначенные, и так и сказано
            servicesAssigned = true;
            try
            {
                var sAs = NewSettings();
                var vmAs = Watched(new SearchViewModel(Board(), sAs, NewClient(sAs)));
                await vmAs.OpenAsync();
                Check("пустой справочник — назначенные сервисы", vmAs.ServiceChoices.Select(c => c.Id).SequenceEqual(new[] { 0, 840, 844 }),
                    () => string.Join(", ", vmAs.ServiceChoices.Select(c => $"{c.Id} {c.Label}")));
                Check("под списком — что это назначенные и почему", vmAs.ServiceNote.Contains("пустой ответ") && vmAs.ServiceNote.Contains("на которые вы назначены"),
                    () => vmAs.ServiceNote);
            }
            finally { servicesAssigned = false; }

            // 10в. сервисов нет нигде, и без ошибок — это ответ: под списком сказано, а повторное открытие за ними не ходит
            servicesNone = true;
            try
            {
                var sNo = NewSettings();
                var vmNo = Watched(new SearchViewModel(Board(), sNo, NewClient(sNo)));
                await vmNo.OpenAsync();
                var servicesAsked = Count("/api/service");
                await vmNo.OpenAsync();
                Check("нигде нет — «любой» и объяснение, повторно не спрашивается", vmNo.ServiceChoices.Count == 1
                    && vmNo.ServiceNote.StartsWith("Сервер не вернул ни одного сервиса") && Count("/api/service") == servicesAsked, () => vmNo.ServiceNote);
            }
            finally { servicesNone = false; }

            // 10г. поддельный сервер, как живой, не принимает count=all: HTTP 400 с ответом проверки параметров ASP.NET Core
            using (var raw = new HttpClient())
            {
                using var refused = await raw.GetAsync($"http://127.0.0.1:{port}/api/task?count=all&page=1");
                var body = await refused.Content.ReadAsStringAsync();
                Check("count=all — HTTP 400, как у живого", (int)refused.StatusCode == 400 && body.Contains("\"count\":[\"The value 'all' is not valid.\"]"), () => body);
                using var accepted = await raw.GetAsync($"http://127.0.0.1:{port}/api/task?count=false&page=1");
                Check("count=false принимается", (int)accepted.StatusCode == 200);
            }

            // 11. настройки сохранили (тот же сервер, клиент новый), пока читались справочники, а поиск уже ждёт их
            slowStatuses = true;
            var sC = NewSettings();
            var vmC = Watched(new SearchViewModel(Board(), sC, NewClient(sC)));
            WireLikeComboBox(vmC);
            var opening = vmC.OpenAsync();
            await Task.Delay(100);
            var waiting = vmC.SearchCommand.ExecuteAsync(null);   // «Закрытые» — без статусов её не собрать
            await Task.Delay(50);
            vmC.ApplySettings(NewClient(sC));
            await Task.WhenAll(opening, waiting);
            slowStatuses = false;
            Check("прочитанное тем же сервером принято, поиск по «закрытым» состоялся", vmC.StatusChoices.Count > 3 && vmC.Results.Count == 2 && vmC.Message == "Найдено: 3 · показано 2");

            // 12. «+ На доску»: карточка строится из известных номера, ссылки и названия — без разбора текста
            var board = Board();
            var sE = NewSettings();
            var vmE = Watched(new SearchViewModel(board, sE, NewClient(sE)));
            vmE.Words = "vpn";
            await vmE.SearchCommand.ExecuteAsync(null);
            vmE.AddToBoardCommand.Execute(vmE.Results[0]);
            Check("добавлена карточка с номером, ссылкой и названием", board.AllTickets.SingleOrDefault() is { IntraserviceId: 701, Title: "Первая" } a
                && a.Url == vmE.Results[0].Url && a.Url.EndsWith("/Task/View/701") && vmE.Results[0].OnBoard && board.Columns[0].Items.Contains(a));
            vmE.AddToBoardCommand.Execute(vmE.Results[0]);
            Check("повторно не добавляется", board.AllTickets.Count() == 1);

            // 14. «Не больше, заявок» относится к выгрузке, а не к поиску: нечисло поиску не мешает, выгрузке — мешает, подпись
            // кнопки показывает потолок, запомненное — прежнее число, а не мусор
            var sG = NewSettings();
            var vmG = Watched(new SearchViewModel(Board(), sG, NewClient(sG)));
            vmG.Words = "vpn";
            vmG.Limit = "много";
            await vmG.SearchCommand.ExecuteAsync(null);
            Check("поиск при «мусоре» в поле потолка работает", vmG.Results.Count == 2 && vmG.Message == "Найдено: 3 · показано 2");
            Check("запомнено прежнее число, а не мусор", sG.LastSearch is { Limit: 500 });
            await vmG.ExportFoundCommand.ExecuteAsync(null);
            Check("выгрузка с «мусором» отказывается и называет, что можно (число или 0)", vmG.WorkMessage.Contains("число заявок или 0") && !vmG.IsWorking);
            vmG.Limit = "2";
            Check("подпись показывает потолок: две из трёх", vmG.ExportFoundLabel == "Выгрузить найденные (2 из 3)");
            vmG.Limit = "500";
            Check("потолок больше найденного — подпись с числом найденного", vmG.ExportFoundLabel == "Выгрузить найденные (3)");

            // 14а. 0 — все: подпись с числом найденного, в настройках — 0; до порога вопроса нет, список шёл по созданию; много (2500) —
            // вопрос с числом, «нет» — на сервер за заявками не ходим; порог ниже (3) — «да» выгружает
            var sZ = NewSettings();
            var vmZ = Watched(new SearchViewModel(Board(), sZ, NewClient(sZ)));
            var questions = new List<(string Heading, string Text)>();
            var answer = false;
            vmZ.Confirm = (heading, text) => { questions.Add((heading, text)); return answer; };
            vmZ.Words = "vpn";
            vmZ.Limit = "0";
            await vmZ.SearchCommand.ExecuteAsync(null);
            Check("0 — все: подпись с числом найденного, в настройках — 0", vmZ.ExportFoundLabel == "Выгрузить найденные (3)" && sZ.LastSearch is { Limit: 0 });
            vmZ.Folder = Path.Combine(dataDir, "everything");
            await vmZ.ExportFoundCommand.ExecuteAsync(null);
            // первая страница — со счётом по умолчанию (count=all живой сервер не принимает), следующая — без счёта (count=false)
            string FirstExportPage() { lock (asked) return asked.Last(t => t.StartsWith("/api/task?") && t.Contains("sort=Created") && t.EndsWith("&page=1")); }
            Check("три заявки — без вопроса; выгружены все, список шёл по созданию, первая страница со счётом, вторая — без",
                questions.Count == 0 && vmZ.WorkMessage.StartsWith("Готово. По отбору — 3") && Last("/api/task?").Contains("sort=Created%20asc,%20Id%20asc")
                && !FirstExportPage().Contains("count=") && Last("/api/task?").Contains("&count=false&") && Last("/api/task?").EndsWith("&page=2")
                && !vmZ.WorkMessage.Contains("взяты первые"), () => vmZ.WorkMessage + "\n" + FirstExportPage() + "\n" + Last("/api/task?"));
            vmZ.Words = "huge";
            await vmZ.SearchCommand.ExecuteAsync(null);
            var big = 2500.ToString("N0", CultureInfo.CurrentCulture);
            Check("нашлось 2500: подпись «все» с числом, поиск показал одну страницу", vmZ.Total == 2500 && vmZ.ExportFoundLabel == $"Выгрузить найденные ({big})" && vmZ.Results.Count == 1);
            var cardsBeforeAsk = Count("/api/task/");
            await vmZ.ExportFoundCommand.ExecuteAsync(null);
            Check("2500: сначала вопрос с числом; «нет» — выгрузка не начата, на сервер за заявками не ходили",
                questions.Count == 1 && questions[0].Heading.Contains(big) && questions[0].Text.Contains(big) && questions[0].Text.Contains("продолжится")
                && vmZ.WorkMessage == "Выгрузка отменена." && !vmZ.IsWorking && Count("/api/task/") == cardsBeforeAsk);
            vmZ.Limit = "100";
            Check("потолок 100 из 2500 — подпись «100 из 2 500»", vmZ.ExportFoundLabel == $"Выгрузить найденные (100 из {big})");
            SearchViewModel.ConfirmFrom = 3;
            try
            {
                vmZ.Words = "vpn";
                vmZ.Limit = "0";
                await vmZ.SearchCommand.ExecuteAsync(null);
                await vmZ.ExportFoundCommand.ExecuteAsync(null);
                Check("порог 3: три заявки — вопрос, «нет» — отмена", questions.Count == 2 && vmZ.WorkMessage == "Выгрузка отменена.");
                answer = true;
                await vmZ.ExportFoundCommand.ExecuteAsync(null);
                Check("«да» — выгрузка идёт: всё уже выгружено прежде, без изменений 3", questions.Count == 3
                    && vmZ.WorkMessage.StartsWith("Готово. По отбору — 3") && vmZ.WorkMessage.Contains("без изменений 3"), () => vmZ.WorkMessage);
            }
            finally { SearchViewModel.ConfirmFrom = 2000; }

            // 14г. сервер досчитал до потолка (1000 — «столько или больше»): число с «+», подпись кнопки и вопрос — без выдуманного числа
            vmZ.Words = "thousand";
            vmZ.Limit = "0";
            answer = false;
            await vmZ.SearchCommand.ExecuteAsync(null);
            var thousand = 1000.ToString("N0", CultureInfo.CurrentCulture);
            Check("найдено «1 000+»", vmZ.Total == 1000 && vmZ.Message == $"Найдено: {thousand}+ · показано 1" && vmZ.ExportFoundLabel == $"Выгрузить найденные ({thousand}+)",
                () => vmZ.Message + " | " + vmZ.ExportFoundLabel);
            vmZ.Limit = "500";
            Check("«500 из 1 000+»", vmZ.ExportFoundLabel == $"Выгрузить найденные (500 из {thousand}+)", () => vmZ.ExportFoundLabel);
            vmZ.Limit = "1000";
            Check("ровно потолок — «1 000 из 1 000+», как и в вопросе", vmZ.ExportFoundLabel == $"Выгрузить найденные ({thousand} из {thousand}+)"
                && SearchViewModel.ConfirmTitle(1000, 1000) == $"Выгрузить заявок: {thousand}?", () => vmZ.ExportFoundLabel);
            vmZ.Limit = "5000";
            Check("потолок больше «1 000+» — «до 5 000»", vmZ.ExportFoundLabel == $"Выгрузить найденные (до {5000.ToString("N0", CultureInfo.CurrentCulture)})", () => vmZ.ExportFoundLabel);
            var asks = questions.Count;
            vmZ.Limit = "0";
            await vmZ.ExportFoundCommand.ExecuteAsync(null);
            Check("все при «1 000+» — вопрос без числа (хоть найдено и меньше порога 2000), «нет» — отмена", questions.Count == asks + 1
                && questions[^1].Heading == "Выгрузить все найденные заявки?" && questions[^1].Text.Contains($"не меньше {thousand}")
                && questions[^1].Text.Contains("Будут выгружены все.") && !questions[^1].Text.Contains("всего порядка") && vmZ.WorkMessage == "Выгрузка отменена.",
                () => questions[^1].Heading + "\n" + questions[^1].Text);
            vmZ.Limit = "3000";
            await vmZ.ExportFoundCommand.ExecuteAsync(null);
            Check("до 3000 при «1 000+» — «Выгрузить до 3 000 заявок?»", questions.Count == asks + 2
                && questions[^1].Heading == $"Выгрузить до {3000.ToString("N0", CultureInfo.CurrentCulture)} заявок?" && vmZ.WorkMessage == "Выгрузка отменена.",
                () => questions[^1].Heading);
            // показано всё, что окно показывает, а счёт упёрся в потолок: подсказка «уточните условия» остаётся (потолок 3 на трёх)
            var ceilingWas = HttpIntraserviceClient.CountCeiling;
            HttpIntraserviceClient.CountCeiling = 3;
            try
            {
                vmZ.Words = "vpn";
                await vmZ.SearchCommand.ExecuteAsync(null);
                await vmZ.ShowMoreCommand.ExecuteAsync(null);
                Check("все показаны, а счёт — потолок: «3+ · показано 3»", vmZ.Results.Count == 3 && vmZ.Message == "Найдено: 3+ · показано 3", () => vmZ.Message);
            }
            finally { HttpIntraserviceClient.CountCeiling = ceilingWas; }

            // 14б. итог: законченная выгрузка — «Готово», остановленная или прерванная — «Не закончено» с тем, что успели, и причиной
            var done = SearchViewModel.Summary(new ExportResult(1250, 1000, 200, 50, 0, "", "Заняло 2 ч 05 мин."), "По отбору");
            Check("итог законченной: «Готово» и числа с разделителем", done.StartsWith($"Готово. По отбору — {1250.ToString("N0", CultureInfo.CurrentCulture)}: новых файлов {1000.ToString("N0", CultureInfo.CurrentCulture)}") && done.EndsWith("Заняло 2 ч 05 мин."));
            var part = SearchViewModel.Summary(new ExportResult(400, 390, 0, 5, 5, "#12: нет доступа", "Остановлено — что успели, сохранено.", Complete: false), "По отбору");
            Check("итог остановленной: «Не закончено», сколько обработано, что не прочиталось, причина",
                part.StartsWith("Не закончено. По отбору, обработано — 400: новых файлов 390") && part.Contains("Не прочитались: 5") && part.Contains("#12: нет доступа") && part.EndsWith("Остановлено — что успели, сохранено."));
            Check("ничего не обработано — только причина", SearchViewModel.Summary(new ExportResult(0, 0, 0, 0, 0, "", "Остановлено.", Complete: false), "По отбору") == "Остановлено.");

            // 14в. поздний отчёт о ходе (Progress шлёт его в поток окна — он может прийти уже после итога) не затирает итог выгрузки
            var sLate = NewSettings();
            var vmL2 = Watched(new SearchViewModel(Board(), sLate, NewClient(sLate)));
            var lateRun = vmL2.BeginWork("начинаю");
            IProgress<string> lateProgress = vmL2.ProgressInto(lateRun);
            lateProgress.Report("ход до итога");
            Check("отчёт о ходе до итога виден", await SingleThread.Until(() => vmL2.WorkMessage == "ход до итога", 2000));
            vmL2.FinishWork(lateRun, "итог");
            lateProgress.Report("поздний отчёт");
            await Task.Delay(150);
            Check("отчёт о ходе до итога виден, поздний после итога — нет", vmL2.WorkMessage == "итог" && !vmL2.IsWorking, () => vmL2.WorkMessage);

            // 15. поиск с доски не затирает запомненные условия, а свой — запоминает
            var sH = NewSettings();
            sH.LastSearch = new SearchFilter(Mine: true, Status: SearchStatus.Closed, ServiceId: 844);
            var vmH = Watched(new SearchViewModel(Board(), sH, NewClient(sH)));
            await vmH.StartWithAsync("принтер");
            Check("быстрый поиск идёт по слову, привычные условия в настройках целы", vmH.Results.Count == 2 && sH.LastSearch is { Mine: true, ServiceId: 844, Words: "" });
            vmH.Words = "другое слово";
            await vmH.SearchCommand.ExecuteAsync(null);
            Check("следующий поиск уже запоминается", sH.LastSearch is { Words: "другое слово" });

            // 15a. окно после «быстрого» поиска с доски, открытое из трея заново, возвращает привычные условия; показанное — не трогает
            var sQ = NewSettings();
            sQ.LastSearch = new SearchFilter(Mine: true, Status: SearchStatus.Closed, ServiceId: 844);
            var vmQ = Watched(new SearchViewModel(Board(), sQ, NewClient(sQ)));
            WireLikeComboBox(vmQ);
            await vmQ.StartWithAsync("принтер");
            Check("после быстрого поиска в форме — слово, условий нет", vmQ.Words == "принтер" && !vmQ.Mine && vmQ.Results.Count == 2);
            await vmQ.OpenAsync(restoreQuick: false);
            Check("окно уже показано — результаты быстрого поиска на месте", vmQ.Words == "принтер" && vmQ.Results.Count == 2);
            await vmQ.OpenAsync(restoreQuick: true);
            Check("спрятанное окно открыли заново — привычные условия, список убран", vmQ.Words == "" && vmQ.Mine && vmQ.SelectedStatus is { Kind: SearchStatus.Closed }
                && vmQ.SelectedService?.Id == 844 && vmQ.Results.Count == 0 && vmQ.Message == "");
            await vmQ.StartWithAsync("принтер");
            vmQ.Words = "своё";
            await vmQ.SearchCommand.ExecuteAsync(null);
            await vmQ.OpenAsync(restoreQuick: true);
            Check("свой поиск после быстрого запомнен — при открытии его условия остаются", vmQ.Words == "своё" && sQ.LastSearch is { Words: "своё" });

            // 15b. «быстрый» поиск при ненастроенном API не оставляет флаг: следующий, уже настоящий, запоминается
            var sJ = NewSettings();
            var vmJ = Watched(new SearchViewModel(Board(), sJ, null));
            await vmJ.StartWithAsync("слово");
            Check("API не настроен — сказано", vmJ.Message.Contains("API не настроен"));
            vmJ.ApplySettings(NewClient(sJ));
            vmJ.Words = "vpn";
            await vmJ.SearchCommand.ExecuteAsync(null);
            Check("поиск после настройки API запомнен", sJ.LastSearch is { Words: "vpn" } && vmJ.Results.Count == 2);

            // 16. статусов не было (сеть), «закрытые» без них не собрать — следующий поиск пробует снова
            var sI = NewSettings();
            failStatuses = true;
            var vmI = Watched(new SearchViewModel(Board(), sI, NewClient(sI)));   // по умолчанию — «Закрытые»
            await vmI.SearchCommand.ExecuteAsync(null);
            Check("без статусов «закрытые» не собрать — сказано", vmI.Message.Contains("не загрузился") && vmI.Results.Count == 0);
            failStatuses = false;
            await vmI.SearchCommand.ExecuteAsync(null);
            Check("вернулась сеть — тот же «Найти» уже ищет", vmI.Results.Count == 2);

            // 17. дата на краю диапазона: ошибка в строке состояния, а не исключение из команды
            var sK = NewSettings();
            var vmK = Watched(new SearchViewModel(Board(), sK, NewClient(sK)));
            vmK.ChangedTo = "31.12.9999";
            await vmK.SearchCommand.ExecuteAsync(null);
            Check("дата 9999 года — понятное сообщение", vmK.Message.Contains("не понятна") && vmK.Results.Count == 0 && !vmK.IsBusy);

            // 18. предупреждение о дате — по всему загруженному списку, а не по последней странице
            vmK.ClearCommand.Execute(null);
            vmK.ChangedFrom = "01.01.2027";   // заявки сервера — 2026 года: все вне периода
            await vmK.SearchCommand.ExecuteAsync(null);
            Check("первая страница: две из двух вне периода", vmK.Warning.Contains("(2 из 2 загруженных)"));
            await vmK.ShowMoreCommand.ExecuteAsync(null);
            Check("с «ещё» — три из трёх, а не одна из одной", vmK.Warning.Contains("(3 из 3 загруженных)"));

            // 19. прочитанный текст заявки живёт недолго: окно открыто часами, заявке успевают дописать
            var sL = NewSettings();
            var vmL = Watched(new SearchViewModel(Board(), sL, NewClient(sL)));
            vmL.Words = "vpn";
            await vmL.SearchCommand.ExecuteAsync(null);
            SearchViewModel.TextTtl = TimeSpan.FromMilliseconds(60);
            try
            {
                vmL.SetSelection(new[] { vmL.Results[0] }, vmL.Results[0]);
                Check("просмотр дождался текста", await SingleThread.Until(() => !vmL.IsPreviewBusy && vmL.PreviewText.Length > 0));
                var cards1 = Count("/api/task/701");
                await Task.Delay(150);
                vmL.SetSelection(new[] { vmL.Results[1] }, vmL.Results[1]);
                Check("просмотр дождался #702", await SingleThread.Until(() => !vmL.IsPreviewBusy && vmL.PreviewText.Contains("id: 702")));
                vmL.SetSelection(new[] { vmL.Results[0] }, vmL.Results[0]);
                Check("просмотр дождался #701", await SingleThread.Until(() => !vmL.IsPreviewBusy && vmL.PreviewText.Contains("id: 701")));
                Check("по истечении срока заявку читают заново", Count("/api/task/701") == cards1 + 1);
            }
            finally { SearchViewModel.TextTtl = TimeSpan.FromMinutes(3); }

            // 20. выгрузка идёт, а ищут снова: в итоге — сколько было найдено к её началу
            slowCards = true;
            var sM = NewSettings();
            var vmM = Watched(new SearchViewModel(Board(), sM, NewClient(sM)));
            vmM.Words = "vpn";
            await vmM.SearchCommand.ExecuteAsync(null);
            vmM.Limit = "2";
            vmM.Folder = Path.Combine(dataDir, "third");
            var exportingM = vmM.ExportFoundCommand.ExecuteAsync(null);
            await Task.Delay(100);
            vmM.Words = "fast";
            await vmM.SearchCommand.ExecuteAsync(null);   // другой список: найдена одна заявка
            await exportingM;
            slowCards = false;
            Check("итог выгрузки — про прежний список (найдено 3, взяты 2)", vmM.WorkMessage.Contains("Всего найдено 3, взяты первые 2"));

            // 21. сменили сервер после «быстрого» поиска: номера прежнего сервера не вернутся из запомненных условий
            var sR = NewSettings();
            sR.LastSearch = new SearchFilter(Mine: true, Status: SearchStatus.Closed, ServiceId: 844, TypeId: 1009, SavedFilterId: 45);
            var vmR = Watched(new SearchViewModel(Board(), sR, NewClient(sR)));
            WireLikeComboBox(vmR);
            await vmR.StartWithAsync("принтер");
            sR.IntraserviceBaseUrl = $"http://localhost:{port}";
            vmR.ApplySettings(NewClient(sR));
            Check("в запомненных условиях номера прежнего сервера сброшены, остальное цело",
                sR.LastSearch is { ServiceId: 0, TypeId: 0, SavedFilterId: 0, Mine: true, Status: SearchStatus.Closed });
            await vmR.OpenAsync(restoreQuick: true);
            Check("и в форме их нет: привычные условия без чужих номеров", vmR.Mine && vmR.SelectedService?.Id == 0 && vmR.SelectedType?.Id == 0 && vmR.SelectedSaved?.Id == 0);

            // 22. по списку проехали стрелкой: сервер спрашивают только про строку, на которой остановились
            var sS = NewSettings();
            var vmS = Watched(new SearchViewModel(Board(), sS, NewClient(sS)));
            vmS.Words = "vpn";
            await vmS.SearchCommand.ExecuteAsync(null);
            var c701 = Count("/api/task/701");
            var c702 = Count("/api/task/702");
            vmS.SetSelection(new[] { vmS.Results[0] }, vmS.Results[0]);
            await Task.Delay(40);
            vmS.SetSelection(new[] { vmS.Results[1] }, vmS.Results[1]);
            Check("просмотр дождался #702", await SingleThread.Until(() => !vmS.IsPreviewBusy && vmS.PreviewText.Contains("id: 702")));
            Check("прочитана только последняя строка, мимо которой не проехали", Count("/api/task/701") == c701 && Count("/api/task/702") == c702 + 1);

            // 13. сменили сервер, пока шла выгрузка: она останавливается — чужие заявки в папку не пишем
            slowCards = true;
            var sF = NewSettings();
            var vmF = Watched(new SearchViewModel(Board(), sF, NewClient(sF)));
            vmF.Words = "vpn";
            await vmF.SearchCommand.ExecuteAsync(null);
            vmF.SetSelection(vmF.Results.ToList(), null);
            vmF.Folder = Path.Combine(dataDir, "stopped");
            var exporting = vmF.ExportSelectedCommand.ExecuteAsync(null);
            await Task.Delay(100);
            sF.IntraserviceBaseUrl = $"http://localhost:{port}";
            vmF.ApplySettings(NewClient(sF));
            await exporting;
            slowCards = false;
            Check("выгрузка со старого сервера остановлена", vmF.WorkMessage.Contains("Остановлено") && !vmF.IsWorking);

            // 8. условия сохраняются, сброс устаревит список
            Check("последний поиск запомнен (сервис 840, слов нет)", s1.LastSearch is { Words: "", ServiceId: 840 });
            vm.ClearCommand.Execute(null);
            Check("после «Сбросить» условия пусты, а список прежний — устарел", vm.Words == "" && vm.IsStale);
        }
        finally { listener.Stop(); }
        }
        Check("списки окна и доски менялись только в потоке окна", pump.OffThread == 0, () => $"изменений из чужого потока: {pump.OffThread}");
        checks.AssertAll();
    }
}
