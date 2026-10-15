using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Text;
using System.Text.RegularExpressions;

namespace TicketBoard.Services;

public static partial class KnowledgeExport
{
    /// <summary>Самопроверка: имена файлов, экранирование, формат — и живая выгрузка настоящим клиентом против поддельного
    /// Интрасервиса (FakeIntraservice) в три захода: первый, повторный без изменений, после переименования заявки. Даты
    /// списка — от сегодняшнего дня: с «последними 365 днями» неподвижные даты однажды выпали бы из периода. Выгрузка всех
    /// заявок (страницы, остановка и продолжение, повторы, выключатель) — в SelfCheckMass (.SelfCheck.Mass.cs).</summary>
    [Conditional("DEBUG")]
    internal static void SelfCheck()
    {
        // имя файла: номер и название без знаков, запрещённых Windows и ломающих ссылки Obsidian
        Debug.Assert(FileName(702180, "Принтер: «HP» / замятие? [срочно] #1") == "702180 — Принтер «HP» замятие срочно 1.md");
        // длина названия задаётся: путь глубокой папки хранилища иначе не влез бы в MAX_PATH
        Debug.Assert(FileName(5, "длинное название", 5) == "5 — длинн.md" && FileName(5, "название", 0) == "5.md" && FileName(5, "я😀хвост", 2) == "5 — я.md");
        var longRow = new IntraserviceFound(501, new string('я', 200), "Закрыта", null, new DateTimeOffset(2026, 8, 1, 0, 0, 0, TimeSpan.Zero));
        foreach (var depth in new[] { 100, 200, 235 })   // длина пути к tickets; на 235 от названия остаётся один номер
        {
            var folder = new string('d', depth);
            var (shard, rel) = PlaceOf(folder, longRow);
            Debug.Assert(shard == "2026-08" && Path.Combine(folder, rel).Length <= 250 && rel.EndsWith(".md") && rel.Contains("501"));
        }
        Debug.Assert(PlaceOf(new string('d', 100), longRow).Rel.Length - "2026-08/".Length - "501 — .md".Length == 80);   // короткий путь — название целиком (до 80)
        Debug.Assert(FileName(5, " ?? ") == "5.md" && FileName(5, new string('я', 100)) == $"5 — {new string('я', 80)}.md");
        // обрезка не разрезает эмодзи (пару-заменитель): имя читается из UTF-8 обратно тем же
        var emoji = FileName(5, new string('я', 79) + "😀 хвост");
        Debug.Assert(emoji == $"5 — {new string('я', 79)}.md" && Encoding.UTF8.GetString(Encoding.UTF8.GetBytes(emoji)) == emoji);
        Debug.Assert(FileName(5, new string('я', 78) + "😀 хвост") == $"5 — {new string('я', 78)}😀.md");
        // текст как есть, но не разметка: заголовок, тег Obsidian, черта; номер «#702180» тегом не бывает — не трогаем
        Debug.Assert(Escape("# не заголовок\nтекст #тег и #702180\n---\r\n  ## тоже") ==
            "\\# не заголовок\nтекст \\#тег и #702180\n\\---\n  \\## тоже");
        // ограды кода и комментарий html без закрытия съели бы следующие записи переписки
        Debug.Assert(Escape("лог:\n```\nошибка\n  ~~~ x\nкод `inline` оставляем\n<!-- скрыто") ==
            "лог:\n\\```\nошибка\n  \\~~~ x\nкод `inline` оставляем\n&lt;!-- скрыто");
        // текст заявки пишут люди со стороны (тема письма — название, имя — из письма), а автовыгрузка кладёт файл в хранилище
        // Obsidian без участия человека: команда Templater «<%» (с триггером на новый файл выполнилась бы, едва файл появится),
        // встроенный DataviewJS «`$=» и ограда ```dataviewjs в цитате или пункте списка (выполняются при показе), html
        // (<code>$<span></span>= …</code> — тот же DataviewJS без «$=» в тексте) — только текст; «%%» — комментарий Obsidian:
        // непарный скрыл бы все следующие записи (батник: %%f); незакрытые <!--, <?…, <![CDATA[ — тоже. «1 < 2» и «<Модуль>» —
        // не html, как есть; автоссылки <https://…> и <почта@домен> — ссылки, как были, но не после «<%» (Templater)
        var hostileText = "<%* tp.x %> и `$= dv.y` и `\n $= z`\n> ```dataviewjs\n> > ~~~js\n- ```dataviewjs\n12) ```\n"
            + "<code>$<span></span>= dv.z</code> 1 < 2 <Модуль>\nfor %%f in (*.log) do echo %%%f\n"
            + "<CODE>$<SPAN></SPAN>= dv.w</CODE> <!DOCTYPE x> <![CDATA[ y <?xml v?>\n"
            + "<https://portal/kb/1?id=5> <ivanov@corp.ru> <%*a=1//@x.ru> <a$=b@x.ru>";
        Debug.Assert(Escape(hostileText) ==
            "&lt;%* tp.x %> и `&#36;= dv.y` и `\n &#36;= z`\n> \\```dataviewjs\n> > \\~~~js\n- \\```dataviewjs\n12) \\```\n"
            + "&lt;code>$&lt;span>&lt;/span>= dv.z&lt;/code> 1 < 2 <Модуль>\nfor %&#37;f in (*.log) do echo %&#37;%f\n"
            + "&lt;CODE>$&lt;SPAN>&lt;/SPAN>= dv.w&lt;/CODE> &lt;!DOCTYPE x> &lt;![CDATA[ y &lt;?xml v?>\n"
            + "<https://portal/kb/1?id=5> <ivanov@corp.ru> &lt;%*a=1//@x.ru> &lt;a&#36;=b@x.ru>", Escape(hostileText));
        Debug.Assert(Escape(Escape(hostileText)) == Escape(hostileText), "дважды — то же: сущность не начало команды");
        var hostile = Format(new IntraserviceFound(7, "<%* tp.a %> `$= b` <i>", "<% s %>", "<%c%> `$=c` %%", null, Description: "<%* d %>",
                Executors: "<%e%>", ExecutorGroup: "<%g%>", Extra: new("<%sv%>", "<%t%>", "<%k%>", null)),
            new[] { new IntraserviceEvent(null, "<%* author %> `$= a` <b>", "<% st %>", "`$= comment`", true) }, "http://x/7", DateTimeOffset.Now);
        // ни в тексте файла, ни в значениях свойств (Dataview показывает их как Markdown) — ни команд, ни html, ни «%%»
        Debug.Assert(!Regex.IsMatch(hostile, @"<[A-Za-z/!?%]|\$=|%%"), hostile);
        Debug.Assert(hostile.Contains("title: \"&lt;%* tp.a %> `&#36;= b` &lt;i>\"\n"), hostile);
        // в имени файла нет обратных кавычек (в оглавлении [[…]] с кодом внутри) и «%%» (комментарий Obsidian в оглавлении)
        Debug.Assert(FileName(5, "a `$= b` %% c") == "5 — a $= b % c.md" && FileName(5, "x %%% y") == "5 — x % y.md", FileName(5, "a `$= b` %% c"));
        // оглавление месяца берёт статус из свойств файла и пишет его в Markdown — тем же правилом, что и шапка файла; и
        // у прежнего файла (формат 2), где значение как есть
        var indexDir = Path.Combine(Path.GetTempPath(), $"tb-index-{Guid.NewGuid():N}");
        try
        {
            Directory.CreateDirectory(Path.Combine(indexDir, "2026-08"));
            var indexed = new IntraserviceFound(1, "x", "#Ожидает `$= dv.x` <% s %> <b> %%", null, new DateTimeOffset(2026, 8, 1, 0, 0, 0, TimeSpan.Zero));
            File.WriteAllText(Path.Combine(indexDir, "2026-08", FileName(1, "x")), Format(indexed, Array.Empty<IntraserviceEvent>(), "http://x/1", DateTimeOffset.Now));
            File.WriteAllText(Path.Combine(indexDir, "2026-08", "2 — y.md"), $"---\nid: 2\nstatus: \"#Ожидает `$= dv.x` <%x%> <b> %%\"\nsource: {SourceName}\n---\n");
            // имя прежнего файла (до 0.15.0) с обратными кавычками и «%%» — в оглавлении текстом, не ссылкой [[…]]: код внутри
            // ссылки и комментарий Obsidian; переименуется, когда заявку выгрузят снова
            File.WriteAllText(Path.Combine(indexDir, "2026-08", "3 — `$= dv.x` %% z.md"), $"---\nid: 3\nsource: {SourceName}\n---\n");
            File.WriteAllText(Path.Combine(indexDir, "2026-08", "4 — a `b.md"), $"---\nid: 4\nstatus: \"`x`\"\nsource: {SourceName}\n---\n");
            WriteMonthIndex(indexDir, "2026-08");
            var index = File.ReadAllText(Path.Combine(indexDir, "2026-08", IndexFile));
            Debug.Assert(index.Contains("[[1 — x]]") && index.Contains("[[2 — y]]") && !index.Contains("[[3 —") && index.Contains("- 3 — ")
                && index.Contains("- 4 — a b · `x`\n")
                && !Regex.IsMatch(index, @"<[A-Za-z/!?%]|\$=|%%|(?<!\\)#Ожидает"), index);
        }
        finally { Directory.Delete(indexDir, true); }

        // карточка + строка списка: поле карточки главнее (и пустое — тоже ответ), чего в ней нет — из строки
        var row = new IntraserviceFound(9, "N", "Закрыта", null, null, ExecutorGroup: "ИТ",
            Extra: new(null, null, null, new DateTimeOffset(2026, 9, 1, 10, 0, 0, TimeSpan.Zero)));
        var merged = WithDetails(row, new IntraserviceTask(9, "N", "Закрыта", null, ExecutorGroup: "", Extra: new("Почта", "Инцидент", "", null)));
        Debug.Assert(merged is { ExecutorGroup: "", Extra: { Service: "Почта", Type: "Инцидент", Categories: "", Resolved: not null } });
        Debug.Assert(WithDetails(row, new IntraserviceTask(9, "N", "Закрыта", null)) is { ExecutorGroup: "ИТ", Extra.Resolved: not null });

        // формат 5 (этап 3б): из карточки — организация, оборудование, вложения, закрытие; из истории — участники, моя роль,
        // реакция и решение в часах, переоткрытия, комментарии, закрывающий; пароли скрыты — в названии, описании, переписке
        static DateTimeOffset On(int day, int hour, int minute) => new(2026, 10, day, hour, minute, 0, TimeSpan.FromHours(3));
        var card715 = new IntraserviceFound(715007, "Настроить ТСД, пароль: 555", "Выполнена", "Петров Пётр", On(14, 11, 2),
            "Нужно две учётки.\nПароль: 777", "Иванов Иван, Сидоров Сидор", "ТП склад", On(15, 9, 20),
            Extra: new("Склад", "Запрос", "ТСД", On(15, 9, 2), "Отдел логистики", "ТСД Zebra, Принтер", 2, On(15, 9, 14)),
            CreatorId: 100, ExecutorIds: new[] { 200, 300 });
        var life715 = new List<IntraserviceEvent>
        {
            new(On(15, 9, 14), "Сидоров Сидор", "Выполнена", "Готово, логин: ТСД_3, пароль: 11111111", true, 300, Final: true),
            new(On(14, 12, 6), "Иванов Иван", "Ожидание ответа", "Давай как в прошлый раз", true, 200),
            new(On(14, 11, 32), "Диспетчерова Дарья", "Передана исполнителю", null, null, 600),
            new(On(14, 11, 2), "Диспетчер Денис", "Открыта", null, null, 500),
        };
        var v5 = Format(card715, life715, "https://hd/Task/View/715007", On(15, 10, 0),
            context: new ExportContext(new IntraserviceUser(200, "Иванов Иван"), new HashSet<string>()));
        Debug.Assert(v5.StartsWith("""
            ---
            id: 715007
            title: "Настроить ТСД, пароль: [скрыт]"
            status: "Выполнена"
            created: 2026-10-14T11:02:00+03:00
            changed: 2026-10-15T09:20:00+03:00
            resolved: 2026-10-15T09:02:00+03:00
            service: "Склад"
            type: "Запрос"
            categories: ["ТСД"]
            creator: "Петров Пётр"
            executors: ["Иванов Иван", "Сидоров Сидор"]
            group: "ТП склад"
            company: "Отдел логистики"
            assets: ["ТСД Zebra", "Принтер"]
            files: 2
            closed: 2026-10-15T09:14:00+03:00
            participants: ["Диспетчер Денис", "Диспетчерова Дарья", "Иванов Иван", "Сидоров Сидор"]
            my_role: "исполнитель"
            first_reaction_h: 0.5
            resolve_h: 22.0
            reopened: 0
            comments_public: 2
            comments_internal: 0
            closing_comment: "Готово, логин: ТСД_3, пароль: [скрыт]"
            url: "https://hd/Task/View/715007"
            source: intraservice
            format: 5
            exported: 2026-10-15T10:00:00+03:00
            ---

            """, StringComparison.Ordinal), v5);
        Debug.Assert(!v5.Contains("555") && !v5.Contains("777") && !v5.Contains("11111111") && v5.Contains("Пароль: [скрыт]")
            && v5.Contains("логин: ТСД_3"), v5);
        // история урезана — её начала нет: участников, реакции и счёта не пишем; решение и роль — пишем
        var cut = Format(card715, life715, "https://hd/Task/View/715007", On(15, 10, 0), truncated: true,
            context: new ExportContext(new IntraserviceUser(200, "Иванов Иван"), new HashSet<string>()));
        Debug.Assert(!cut.Contains("\nparticipants:") && !cut.Contains("\nfirst_reaction_h:") && !cut.Contains("\nreopened:")
            && !cut.Contains("\ncomments_public:") && !cut.Contains("\ncomments_internal:") && cut.Contains("\nresolve_h: 22.0\n")
            && cut.Contains("\nmy_role: \"исполнитель\"\n"), cut);
        // название в одну строку складывает «пароль: x» и через пустую строку — скрыт и в свойствах, и в шапке
        var folded = Format(card715 with { Name = "Учётка, пароль:\n\n556" }, life715, "https://hd/Task/View/715007", On(15, 10, 0));
        Debug.Assert(!folded.Contains("556") && folded.Contains("\ntitle: \"Учётка, пароль: [скрыт]\"\n"), folded);
        // имя файла — тоже без пароля, и с неразрывным пробелом, и сложенное из строк
        Debug.Assert(FileName(5, "Учётка, пароль: 555") == "5 — Учётка, пароль скрыт.md", FileName(5, "Учётка, пароль: 555"));
        Debug.Assert(FileName(5, "Учётка, пароль:\u00A0555") == "5 — Учётка, пароль скрыт.md" && FileName(5, "пароль:\n\n555") == "5 — пароль скрыт.md",
            FileName(5, "Учётка, пароль:\u00A0555") + " | " + FileName(5, "пароль:\n\n555"));

        // сколько заявок: 0 — все, от единицы — потолок, потолка сверху нет; не годится только отрицательное. Выгрузка с неверным
        // числом не начинается (на сервер не ходит)
        Debug.Assert(InvalidLimit(0) is null && InvalidLimit(1) is null && InvalidLimit(1_000_000) is null && InvalidLimit(-1) is not null);
        var none = RunAsync(new HttpIntraserviceClient("http://127.0.0.1:1", "u", "p"), new TaskQuery(), -1,
            Path.GetTempPath(), _ => "", null, CancellationToken.None).GetAwaiter().GetResult();
        Debug.Assert(none.Error.Length > 0 && none.Found == 0);

        // папка заявки — месяц создания в том поясе, что прислал сервер; даты нет — «без даты»
        Debug.Assert(ShardOf(new DateTimeOffset(2026, 8, 31, 23, 30, 0, TimeSpan.FromHours(3))) == "2026-08" && ShardOf(null) == NoDateFolder);
        // срок для человека: в подписи хода и итоге
        Debug.Assert(HumanSpan(TimeSpan.FromSeconds(30)) == "меньше минуты" && HumanSpan(TimeSpan.FromMinutes(42.9)) == "42 мин"
            && HumanSpan(TimeSpan.FromMinutes(185)) == "3 ч 05 мин" && HumanSpan(TimeSpan.FromHours(30.5)) == "30 ч 30 мин");

        var now = DateTimeOffset.Now;
        static string Iso(DateTimeOffset d) => d.ToString("yyyy-MM-dd'T'HH:mm:ss", CultureInfo.InvariantCulture);
        // 504 и 502 — на 45 дней назад, 501 — на 14: месяцы всегда разные (между датами 31 день, а в месяце их не больше 31)
        var dates = new Dates(Created501: Iso(now.AddDays(-14)), Recent: Iso(now.AddDays(-45)), Old: Iso(now.AddDays(-800)));
        var m501 = ShardOf(now.AddDays(-14));
        var m504 = ShardOf(now.AddDays(-45));
        Debug.Assert(m501 != m504);

        var title501 = "Принтер не печатает";
        var changed501 = Iso(now.AddDays(-10));
        var cardsAsked = 0;   // запросов карточки заявки: неизменные заявки не перечитываются вовсе
        var (listener, port) = FakeIntraservice.Start(target =>
        {
            if (target.StartsWith("/api/task/")) Interlocked.Increment(ref cardsAsked);
            return Respond(target, title501, changed501, dates);
        });
        var dir = Path.Combine(Path.GetTempPath(), $"tb-export-{Guid.NewGuid():N}");
        try
        {
            var client = new HttpIntraserviceClient($"http://127.0.0.1:{port}", "user", "pass");
            // кто я и закрытые названия — как передаёт окно и автовыгрузка; «Выполнена» здесь закрыта только по названию
            var context = new ExportContext(new IntraserviceUser(7, "Я Сам"), new HashSet<string>(StringComparer.OrdinalIgnoreCase) { "Выполнена" });
            // мои закрытые за последние 365 дней по изменению: список обрывается на первой заявке старше границы
            var mine = new TaskQuery(ExecutorIds: new[] { 7 }, StatusIds: new[] { 29, 30 }, Changed: new(now.AddDays(-365).DateTime, null));
            ExportResult Run() => RunAsync(client, mine, 100, dir, id => $"https://hd/Task/View/{id}", null, CancellationToken.None,
                context: context).GetAwaiter().GetResult();

            // первый заход: 503 старше периода — не попала; 502 не отдала карточку — в «не прочитались», файла нет;
            // у 504 ровно 50 записей и Paginator «страница 1 из 1» — вторую страницу не просим (её нет — 404 сломал бы)
            var first = Run();
            Debug.Assert(first is { Found: 3, Created: 2, Updated: 0, Unchanged: 0, Failed: 1, Error: "" } && first.FirstError.StartsWith("#502"));
            // файлы — в папках месяцев создания; у 502 карточка не прочиталась — файла нет нигде
            var file = Path.Combine(dir, TicketsFolder, m501, "501 — Принтер не печатает.md");
            Debug.Assert(File.Exists(file) && OnDisk(dir).Count == 2 && !OnDisk(dir).Any(p => Path.GetFileName(p).StartsWith("502")));
            var text = File.ReadAllText(file, Encoding.UTF8);
            Debug.Assert(text.StartsWith("---\nid: 501\ntitle: \"Принтер не печатает\"\nstatus: \"Выполнена\"\n"));
            Debug.Assert(text.Contains("\nexecutors: [\"Иванов И.\", \"Я Сам\"]\n")
                && text.Contains("\nurl: \"https://hd/Task/View/501\"\n") && text.Contains($"\nformat: {FormatVersion}\n"));
            // сервис, тип, категории, группа и дата решения — из карточки: в строке списка их нет, как у живого сервера
            Debug.Assert(text.Contains("\nservice: \"Принтеры\"\ntype: \"Инцидент\"\ncategories: [\"Печать\", \"Картриджи\"]\n")
                && text.Contains("\ngroup: \"Вторая линия\"\n") && text.Contains("\nresolved: ")
                && text.Contains("\nСервис: Принтеры · тип: Инцидент · категории: Печать, Картриджи  \n"));
            Debug.Assert(!text.Contains("+7 999") && !text.Contains("petrova@"));   // контакты людей — не в базу знаний
            // пароль из html письма («Пароль:&nbsp;…» — неразрывный пробел после двоеточия) — скрыт
            Debug.Assert(!text.Contains("Qwerty123") && text.Contains("[скрыт]"), text);
            // формат 5: моя роль (я — среди исполнителей по имени), дата закрытия из строки списка, закрывающий комментарий —
            // статус закрыт названием из настроек, которые выгрузка получила
            Debug.Assert(text.Contains("\nmy_role: \"исполнитель\"\n") && text.Contains("\nclosed: ") && text.Contains("\ncompany: \"Склад\"\n")
                && text.Contains("\nclosing_comment: \"запись 51\"\n") && text.Contains("\nparticipants: [\"Сидоров\", \"Иванов И.\"]\n"), text);
            // настоящая выгрузка кладёт таблицу заявок: строка на каждую выгруженную, и контактов в ней нет
            var csv = File.Exists(Path.Combine(dir, KnowledgeTable.TableFile)) ? File.ReadAllText(Path.Combine(dir, KnowledgeTable.TableFile)) : "";
            Debug.Assert(csv.Split("\r\n").Count(l => l.StartsWith("501;") || l.StartsWith("504;")) == 2 && !csv.Contains("\r\n502;")
                && !csv.Contains("+7 999") && !csv.Contains("petrova@"), csv);
            Debug.Assert(text.Contains("# 501 · Принтер не печатает\n") && text.Contains("## Описание\n\nЗамятие в лотке 2\n"));
            // вся переписка — обе страницы, от первой записи к последней; внутренний помечен; статус — где поменялся
            var firstNote = text.IndexOf("запись 1\n", StringComparison.Ordinal);
            var lastNote = text.IndexOf("запись 51\n", StringComparison.Ordinal);
            Debug.Assert(firstNote > 0 && lastNote > firstNote && text.Split("\n### ").Length == 51 + 1);   // 51 запись — 51 заголовок
            Debug.Assert(text.Contains("— Сидоров (внутренний)\n\nзапись 2\n") && text.Contains("— Иванов И. · статус «Выполнена»\n"));
            // две записи в одну секунду — в том порядке, в каком их сделали (сервер отдаёт свежие сверху)
            var text504 = File.ReadAllText(Path.Combine(dir, TicketsFolder, m504, "504 — Почта не уходит.md"), Encoding.UTF8);
            var body504 = text504[text504.IndexOf("\n---\n", 3, StringComparison.Ordinal)..];   // закрывающий «ответ B» — и в свойствах
            Debug.Assert(text504.Contains("\nclosing_comment: \"ответ B\"\n")
                && body504.IndexOf("ответ A", StringComparison.Ordinal) < body504.IndexOf("ответ B", StringComparison.Ordinal)
                && text504.Split("\n### ").Length == 50 + 1);
            Debug.Assert(text504.Contains("\ntype: \"Запрос на обслуживание\"\n") && !text504.Contains("\nservice:"));   // сервиса нет — строки нет
            var cardsFirst = cardsAsked;
            Debug.Assert(cardsFirst == 3);
            // оглавление месяца со ссылками на заявки; общее — по месяцам, свежие сверху, с числом заявок и ссылкой на оглавление месяца
            var month = File.ReadAllText(Path.Combine(dir, TicketsFolder, m501, IndexFile), Encoding.UTF8);
            Debug.Assert(month.Contains($"- [[501 — Принтер не печатает]] · {now.AddDays(-14):dd.MM.yyyy} · Выполнена\n") && month.Contains("Заявок: 1 ")
                && !month.Contains("504"));
            var index = File.ReadAllText(Path.Combine(dir, IndexFile), Encoding.UTF8);
            // общее оглавление говорит агенту о свойствах формата 5 и о скрытых паролях
            Debug.Assert(index.Contains("ваша роль") && index.Contains("закрывающий комментарий") && index.Contains("заменён на «[скрыт]»"));
            Debug.Assert(index.Contains("заявок: 2.") && index.Contains($"- [{m501}](tickets/{m501}/_index.md) — заявок: 1\n")
                && index.Contains($"- [{m504}](tickets/{m504}/_index.md) — заявок: 1\n") && index.IndexOf($"[{m501}]", StringComparison.Ordinal) < index.IndexOf($"[{m504}]", StringComparison.Ordinal));

            // повторный заход без изменений: переписку не перечитываем
            var again = Run();
            Debug.Assert(again is { Found: 3, Created: 0, Updated: 0, Unchanged: 2, Failed: 1 });
            Debug.Assert(cardsAsked == cardsFirst + 1);   // карточку перечитали только у 502: её файла нет
            // файл прежнего формата (4) переписывается, хоть заявка не менялась: новых свойств в нём нет
            File.WriteAllText(file, File.ReadAllText(file, Encoding.UTF8).Replace($"\nformat: {FormatVersion}\n", "\nformat: 4\n"), new UTF8Encoding(false));
            var upgraded = Run();
            Debug.Assert(upgraded is { Created: 0, Updated: 1, Unchanged: 1, Failed: 1 }
                && File.ReadAllText(file, Encoding.UTF8).Contains($"\nformat: {FormatVersion}\n") && FormatVersion == 5);

            // заявку переименовали и она менялась: файл переписан под новым именем, прежний наш убран. Чужое с номером в имени не
            // трогаем и не считаем дублем: испорченный файл без наших свойств, заметка пользователя со своими свойствами; а чужая
            // заметка без номера и подавно
            var broken = Path.Combine(dir, TicketsFolder, m501, "501 — Принтер.md");
            var userNote = Path.Combine(dir, TicketsFolder, m501, "501 — мои заметки.md");
            var note = Path.Combine(dir, TicketsFolder, "заметки.md");
            File.WriteAllText(broken, "испорчен");
            File.WriteAllText(userNote, "---\ntitle: \"моё\"\n---\n\nпишу про принтер\n");
            File.WriteAllText(note, "моё");
            title501 = "Принтер HP не печатает";
            changed501 = Iso(now.AddDays(-9));
            var renamed = Run();
            Debug.Assert(renamed is { Created: 0, Updated: 1, Unchanged: 1 } && !File.Exists(file) && File.Exists(broken) && File.Exists(userNote)
                && File.Exists(note) && File.Exists(Path.Combine(dir, TicketsFolder, m501, "501 — Принтер HP не печатает.md")));
            // и чужие файлы не заставляют заявку переписываться каждый раз
            var cardsNow = cardsAsked;
            var stable = Run();
            Debug.Assert(stable is { Created: 0, Updated: 0, Unchanged: 2, Failed: 1 } && cardsAsked == cardsNow + 1 && File.Exists(userNote));
            // и в оглавлении месяца — новое имя, не прежнее
            var renamedIndex = File.ReadAllText(Path.Combine(dir, TicketsFolder, m501, IndexFile), Encoding.UTF8);
            Debug.Assert(renamedIndex.Contains("[[501 — Принтер HP не печатает]]") && !renamedIndex.Contains("[[501 — Принтер не печатает]]"));

            // сервер условие по дате не применил (отдал и старые заявки): строки вне периода создания пропускаются, об этом сказано
            var dir2 = Path.Combine(dir, "вторая");
            var recent = new TaskQuery(ExecutorIds: new[] { 7 }, StatusIds: new[] { 29, 30 }, Created: new(now.AddDays(-20).DateTime, null));
            var guarded = RunAsync(client, recent, 100, dir2, id => $"https://hd/Task/View/{id}", null, CancellationToken.None).GetAwaiter().GetResult();
            Debug.Assert(guarded is { Found: 1, Created: 1, Failed: 0 } && guarded.Error.Contains("вне выбранного периода (3)"));
            Debug.Assert(!guarded.Error.Contains("закончился раньше"));   // список прочитан весь: пропущенные не в счёт «не дочитали»
            Debug.Assert(OnDisk(dir2).Count == 1);

            // заявка одним куском — для просмотра и буфера: тот же текст, что в файле; не прочиталась карточка — пустой текст и причина
            var row501 = new IntraserviceFound(501, title501, "Выполнена", "Петрова А.", now.AddDays(-14), Changed: now.AddDays(-9));
            var built = BuildAsync(client, row501, "https://hd/Task/View/501", CancellationToken.None).GetAwaiter().GetResult();
            Debug.Assert(built.Error == "" && built.Text.StartsWith("---\nid: 501\n") && built.Text.Contains("запись 51\n")
                && built.Text.Contains("\nservice: \"Принтеры\"\n"));
            var lost = BuildAsync(client, new IntraserviceFound(502, "VPN", "Закрыта", null, null), "", CancellationToken.None).GetAwaiter().GetResult();
            Debug.Assert(lost.Text == "" && lost.Error.Length > 0);

            // выбранные в окне заявки — файлами в другую папку: только они и оглавление на них
            var dir3 = Path.Combine(dir, "третья");
            var picked = ExportRowsAsync(client, new[] { row501 }, dir3, id => $"https://hd/Task/View/{id}", null, CancellationToken.None)
                .GetAwaiter().GetResult();
            Debug.Assert(picked is { Found: 1, Created: 1, Error: "" } && File.ReadAllText(Path.Combine(dir3, IndexFile)).Contains("заявок: 1."));

            // сервер отдаёт по одной заявке на страницу, а их сто: потолка страниц нет — читаются все сто; у заявок нет даты
            // создания, они ложатся в «без даты»
            var (slow, slowPort) = FakeIntraservice.Start(target =>
            {
                if (target.StartsWith("/api/task?"))
                {
                    var page = int.Parse(target.Split("page=")[1].Split('&')[0]);
                    return (200, $"{{\"Tasks\":[{{\"Id\":{5000 + page},\"Name\":\"З{page}\",\"StatusId\":29,\"Changed\":\"{changed501}\"}}],\"Statuses\":[{{\"Id\":29,\"Name\":\"Выполнена\"}}],"
                        + "\"Paginator\":{\"Count\":100,\"Page\":" + page + ",\"PageCount\":100,\"PageSize\":1,\"CountOnPage\":1}}");
                }
                if (target.StartsWith("/api/task/")) return (200, $"{{\"Task\":{{\"Id\":{target["/api/task/".Length..].Split('?')[0]},\"Name\":\"З\",\"StatusName\":\"Выполнена\"}}}}");
                return (200, """{"TaskLifetimeList":{"TaskLifetimes":[],"Paginator":{"Page":1,"PageCount":1}}}""");
            });
            try
            {
                var dir4 = Path.Combine(dir, "четвёртая");
                var ceiling = RunAsync(new HttpIntraserviceClient($"http://127.0.0.1:{slowPort}", "u", "p"), new TaskQuery(), 100,
                    dir4, id => $"https://hd/Task/View/{id}", null, CancellationToken.None).GetAwaiter().GetResult();
                Debug.Assert(ceiling is { Found: 100, Created: 100, Failed: 0, Complete: true, Error: "" } && OnDisk(dir4).Count == 100
                    && Directory.Exists(Path.Combine(dir4, TicketsFolder, NoDateFolder)));
            }
            finally { slow.Stop(); }

            // запись не удалась (на месте файла — папка): временный рядом записался — это «путь занят» (PathBlockedException),
            // и временный файл не остаётся
            var blocked = Path.Combine(dir, "занято.md");
            Directory.CreateDirectory(blocked);
            var threw = false;
            try { WriteAtomic(blocked, "текст"); }
            catch (PathBlockedException) { threw = true; }
            Debug.Assert(threw && !Directory.EnumerateFiles(dir, "занято.md*.tmp").Any(), "путь занят — своя ошибка, временного нет");
            // имя временного — своё у каждой записи: оставшееся «X.md.tmp» (папка, чужой файл) записи не мешает
            var free = Path.Combine(dir, "свободно.md");
            Directory.CreateDirectory(free + ".tmp");
            WriteAtomic(free, "текст");
            Debug.Assert(File.ReadAllText(free) == "текст", "временный «X.md.tmp» занят — запись всё равно прошла");

            // что считать «занят на время» (повторяем запись): нарушение доступа и блокировка Windows, отказ в доступе; остальное — нет
            Debug.Assert(IsBusy(new IOException("занят", unchecked((int)0x80070020))) && IsBusy(new IOException("блок", unchecked((int)0x80070021)))
                && IsBusy(new UnauthorizedAccessException()));
            Debug.Assert(!IsBusy(new IOException("диск полон", unchecked((int)0x80070070))) && !IsBusy(new IOException("уже есть", unchecked((int)0x800700B7)))
                && !IsBusy(new InvalidOperationException()));

            // файл с прошлой выгрузки годится, пока формат тот же и заявку не меняли
            var props = Path.Combine(dir, "props.md");
            File.WriteAllText(props, $"---\nid: 1\nchanged: 2026-09-01T10:00:00+03:00\nformat: {FormatVersion}\n---\n\nтекст\n");
            var stamp = new DateTimeOffset(2026, 9, 1, 10, 0, 0, TimeSpan.FromHours(3));
            Debug.Assert(IsCurrent(props, stamp) && IsCurrent(props, stamp.ToOffset(TimeSpan.Zero)));   // тот же момент в другом поясе — тот же
            Debug.Assert(!IsCurrent(props, stamp.AddSeconds(1)) && !IsCurrent(Path.Combine(dir, "нет.md"), stamp));
            File.WriteAllText(props, "---\nid: 1\nchanged: 2026-09-01T10:00:00+03:00\nformat: 1\n---\n");
            Debug.Assert(!IsCurrent(props, stamp));   // старый формат — перепишется

            SelfCheckMass();
        }
        finally
        {
            listener.Stop();
            try { Directory.Delete(dir, recursive: true); } catch (IOException) { }
        }
    }

    private sealed record Dates(string Created501, string Recent, string Old);

    /// <summary>Ответы поддельного сервера: я — 7, статусы, четыре мои закрытые заявки (503 — старше периода), их карточки
    /// (у 502 — 404), переписка 501 на двух страницах (51 запись, свежие сверху), 504 — ровно 50 записей на одной странице.</summary>
    private static (int Code, string Json) Respond(string target, string title501, string changed501, Dates d)
    {
        if (target.StartsWith("/api/user?getcurrentuserinfo=true")) return (200, """{"Id":7,"Name":"Я Сам"}""");
        if (target.StartsWith("/api/taskstatus"))
            return (200, """[{"Id":31,"Name":"Открыта"},{"Id":29,"Name":"Выполнена","IsFixed":true},{"Id":30,"Name":"Закрыта"}]""");
        if (target.StartsWith("/api/task/"))
        {
            // карточка — в форме живого ответа: блоки рядом с Task — null, статус строкой; сервис и тип есть только здесь
            if (target.StartsWith("/api/task/502?")) return (404, """{"Message":"not found"}""");
            if (target.StartsWith("/api/task/501?"))
                return (200, """
                    {"Services":null,"Statuses":null,"Task":{"Id":501,"Name":"TITLE","StatusName":"Выполнена","ServiceName":"Принтеры",
                     "Type":"Инцидент","Categories":"Печать, Картриджи","ExecutorGroup":"Вторая линия","ResolutionDateFact":"CHANGED","CreatorCompanyName":"Склад",
                     "CreatorPhone":"+7 999 000-00-00","CreatorEmail":"petrova@example.ru"}}
                    """.Replace("TITLE", title501).Replace("CHANGED", changed501));
            return (200, """{"Task":{"Id":504,"Name":"Почта не уходит","StatusName":"Выполнена","ServiceName":null,"Type":"Запрос на обслуживание","Categories":null,"ExecutorGroup":null}}""");
        }
        if (target.StartsWith("/api/task?"))
        {
            // отбор дошёл до сервера: мои (7), только закрытые статусы (29 — признак сервера, 30 — по названию из настроек)
            Debug.Assert(target.Contains("ExecutorIds=7&") && target.Contains("StatusIds=29,30&") && !target.Contains("search="));
            Debug.Assert(!target.Contains("MoreThan=") || target.Contains("ChangedMoreThan=") || target.Contains("CreatedMoreThan="));
            // счёт — по умолчанию (count=all живой сервер не принимает, HTTP 400) или выключен на страницах после первой
            Debug.Assert(!target.Contains("count=") || target.Contains("&count=false&"));
            return (200, """
                {"Tasks":[
                  {"Id":501,"Name":"TITLE","StatusId":29,"Created":"CREATED","Changed":"CHANGED",
                   "Creator":"Петрова А.","CreatorPhone":"+7 999 000-00-00","CreatorEmail":"petrova@example.ru",
                   "Executors":"Иванов И., Я Сам","Description":"<p>Замятие в лотке 2<br>Пароль:&nbsp;Qwerty123</p>","Closed":"CHANGED"},
                  {"Id":502,"Name":"VPN не подключается","StatusId":30,"Created":"RECENT","Changed":"RECENT"},
                  {"Id":504,"Name":"Почта не уходит","StatusId":29,"Created":"RECENT","Changed":"RECENT"},
                  {"Id":503,"Name":"Старое","StatusId":29,"Created":"OLD","Changed":"OLD"}],
                 "Statuses":[{"Id":29,"Name":"Выполнена"},{"Id":30,"Name":"Закрыта"}],"Paginator":{"Count":4,"Page":1,"PageCount":1}}
                """.Replace("TITLE", title501).Replace("CHANGED", changed501).Replace("CREATED", d.Created501)
                   .Replace("RECENT", d.Recent).Replace("OLD", d.Old));
        }
        const string Statuses = """[{"Id":31,"Name":"Открыта"},{"Id":29,"Name":"Выполнена"}]""";
        static string Lifetime(IEnumerable<string> rows, int page, int pages) =>
            """{"TaskLifetimeList":{"TaskLifetimes":[ROWS],"Statuses":STATUSES,"Paginator":{"Page":PAGE,"PageCount":PAGES}}}"""
                .Replace("ROWS", string.Join(",", rows)).Replace("STATUSES", Statuses)
                .Replace("PAGES", pages.ToString()).Replace("PAGE", page.ToString());
        static string Row(string date, string editor, int status, string comment, bool isPublic) =>
            "{\"Date\":\"" + date + "\",\"Editor\":\"" + editor + "\",\"StatusId\":" + status + ",\"Comments\":\"<p>" + comment
            + "</p>\",\"IsPublic\":" + (isPublic ? "true" : "false") + "}";
        if (target.StartsWith("/api/tasklifetime?taskid=501"))
        {
            // 51 запись: первая страница — 50 свежих (51…2), вторая — самая первая; запись 2 — внутренняя
            var page = target.Contains("&page=2") ? 2 : 1;
            var numbers = page == 1 ? Enumerable.Range(2, 50).Reverse() : new[] { 1 };
            return (200, Lifetime(numbers.Select(n => Row(
                $"2026-09-{(n == 51 ? 20 : 18):00}T{9 + n / 10:00}:{n % 10 * 5:00}:00", n == 51 ? "Иванов И." : "Сидоров",
                n == 51 ? 29 : 31, $"запись {n}", n != 2)), page, 2));
        }
        // у 502 переписка есть — файла нет только из-за карточки: без сервиса и типа файл не пишем
        if (target.StartsWith("/api/tasklifetime?taskid=502"))
            return (200, Lifetime(new[] { Row("2026-09-12T10:00:00", "Петров", 30, "переподключил", true) }, 1, 1));
        if (target.StartsWith("/api/tasklifetime?taskid=504"))
        {
            if (target.Contains("&page=")) return (404, """{"Message":"no such page"}""");   // страницы 2 нет
            // ровно 50: две последние — в одну секунду (сервер ставит более позднюю первой), до них — 48 по минуте
            var rows = new[] { Row("2026-09-11T10:00:00", "Сидоров", 29, "ответ B", true), Row("2026-09-11T10:00:00", "Сидоров", 31, "ответ A", true) }
                .Concat(Enumerable.Range(1, 48).Reverse().Select(n => Row($"2026-09-10T{9 + n / 60:00}:{n % 60:00}:00", "Петров", 31, $"строка {n}", true)));
            return (200, Lifetime(rows, 1, 1));
        }
        return (404, """{"Message":"not found"}""");
    }
}
