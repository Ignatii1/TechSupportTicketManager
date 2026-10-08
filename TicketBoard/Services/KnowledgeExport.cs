using System.Collections.Concurrent;
using System.Diagnostics;
using System.IO;

namespace TicketBoard.Services;

/// <summary>Итог выгрузки: обработано Found заявок — записано новых Created, переписано изменившихся Updated, не менялись
/// Unchanged (их переписку не перечитывали), не прочитались Failed (с первой ошибкой). Error — что сказать об итоге: остановили,
/// прервалась, список пришёл не целиком; что успели — сохранено. Complete — дошли до конца списка (или до заказанного числа
/// заявок); false — выгрузку остановили, она прервалась или список не дочитан: повторный запуск продолжит с этого места.</summary>
public sealed record ExportResult(int Found, int Created, int Updated, int Unchanged, int Failed, string FirstError, string Error,
    bool Complete = true);

/// <summary>Заявки для базы знаний: по отбору (TaskQuery) — список с сервера, у каждой заявки — карточка (сервис, тип: в
/// списке их нет) и вся переписка (все страницы), и всё это — Markdown. Одним куском (BuildAsync) — для просмотра и
/// буфера обмена; файлами (RunAsync, ExportRowsAsync), по одному на заявку, в папки по месяцам создания:
/// tickets/2026-08/«номер — название».md, в каждой папке оглавление _index.md, в корне — общее.
/// Формат — для агентов и Obsidian: свойства (YAML) в начале файла, описание, переписка по времени — от первой записи к
/// последней. Повторная выгрузка перечитывает только изменившиеся заявки (по Changed) — на этом держится и продолжение
/// остановленной: уже готовые заявки пропускаются. Выгрузка всех заявок учётной записи (сотни тысяч, часы) читает список
/// страницами и сразу пишет файлы, в памяти — только страница и имена прежних файлов. В Интрасервис ничего не пишет;
/// телефоны и почта людей в файлы не попадают.
/// Части класса: этот файл — ход выгрузки, .Format — текст и имя файла, .Files — папки, поиск прежнего, оглавления.</summary>
public static partial class KnowledgeExport
{
    /// <summary>Версия формата файла. Поменялся формат — увеличить: следующая выгрузка перепишет все файлы.
    /// 2 — сервис, тип, категории и группа из карточки заявки (0.11.0). Раскладка по папкам (0.13.0) формат файла не меняет.</summary>
    public const int FormatVersion = 2;
    public const string TicketsFolder = "tickets";
    public const string IndexFile = "_index.md";

    /// <summary>Заявок на страницу списка — столько же просят импорт и F5, на живом сервере проверено. ponytail: сервер отдаст до
    /// 2000, но ответ толще (в строках описания) и дольше счёт; на сотни страниц лишние запросы — не цена рядом с запросами
    /// на каждую заявку.</summary>
    private const int ListPageSize = HttpIntraserviceClient.ExecutorPageSize;

    /// <summary>Не больше стольких страниц переписки одной заявки (по 50 записей, самые свежие первыми): у живых заявок столько не
    /// бывает, а сервер, не понимающий page, не должен гонять нас по кругу. Длиннее — переписка обрезается, и в файле об этом
    /// сказано. Меняется только самопроверкой.</summary>
    internal static int MaxLifetimePages { get; set; } = 100;

    /// <summary>Запросов к серверу разом: он общий, с остальными пользователями.</summary>
    private const int MaxParallel = 4;

    /// <summary>Подряд столько заявок не прочиталось из-за сети или сервера, и ни одной удачной между ними, — сервер лёг или
    /// пропала сеть (компьютер уснул): выгрузку прерываем, а не долбим его часами.</summary>
    private const int TransientLimit = 30;

    /// <summary>Подряд столько ответов 500 без единой удачи между ними — сервер сломался. 500 вдвое-втрое чаще, чем сеть, значит «эта заявка
    /// ему не по зубам» (битая запись): десяток-другой таких подряд — не повод бросать выгрузку, иначе её не пройти дальше битого
    /// места ни при каком повторе. Такой заявке повтор даётся один (не три), чтобы не терять по двадцать секунд на каждую.</summary>
    private const int ServerErrorLimit = 100;

    /// <summary>Столько отказов (не сеть: нет такой заявки, нет доступа), а ни одна заявка не записалась и не оказалась готовой
    /// с прошлого раза, — нет доступа к карточкам, сменился адрес: прерываем. Повторная выгрузка, где почти всё уже готово,
    /// из-за пары десятков отказов не обрывается.</summary>
    private const int NothingWorksLimit = 25;

    /// <summary>Подряд столько файлов не записалось — диск полон или папка недоступна.</summary>
    private const int WriteFailLimit = 10;

    /// <summary>Страницу списка ждём дольше обычного: описания в строках и точный счёт по сотням тысяч заявок — тяжёлый запрос.</summary>
    private static readonly TimeSpan ListTimeout = TimeSpan.FromSeconds(60);

    /// <summary>Паузы перед повторами сбойного запроса (сеть, 5xx): 2, 5 и 15 секунд; не помогло — сбой, заявка дочитается в
    /// следующий раз. Меняется только самопроверкой.</summary>
    internal static TimeSpan[] RetryDelays { get; set; } = { TimeSpan.FromSeconds(2), TimeSpan.FromSeconds(5), TimeSpan.FromSeconds(15) };

    /// <summary>Что не так с числом заявок — текстом для окна; null — всё в порядке. 0 — без ограничения, все найденные.</summary>
    public static string? InvalidLimit(int limit) =>
        limit < 0 ? "«Не больше, заявок» — число заявок или 0, если нужны все" : null;

    /// <summary>Выгрузка по отбору: список страницами и сразу файлы по каждой странице. limit — не больше стольких заявок
    /// (свежие по изменению первыми, строки вне периода не в счёт); 0 — все, тогда список идёт по дате создания от старых к
    /// новым (TaskQuery.StableSort): порядок не «едет», пока выгрузка идёт часами. Остановили, прервалась или оборвалась сеть —
    /// что записано, остаётся, повторный запуск продолжит (готовое пропускается). Условия по дате сервер, возможно, не
    /// применит (формат даты в запросе на живом сервере не проверен) — страхуемся: чтение обрывается за границей периода, строки
    /// вне него не берутся, о чём сказано в итоге.</summary>
    public static async Task<ExportResult> RunAsync(HttpIntraserviceClient client, TaskQuery query, int limit, string dir,
        Func<int, string> ticketUrl, IProgress<string>? progress, CancellationToken ct)
    {
        if (InvalidLimit(limit) is { } invalid) return new(0, 0, 0, 0, 0, "", invalid, Complete: false);
        using var job = new Job(client, dir, ticketUrl, progress, ct);
        var everything = limit == 0;
        var listQuery = everything ? query with { Sort = TaskQuery.StableSort } : query;
        var seen = new HashSet<int>();
        var notes = new List<string>();
        var listError = "";
        int total = 0, taken = 0, outside = 0;
        bool reachedEnd = false, hitLimit = false, stuck = false, unsorted = false, sortFallback = false, fallbackWorked = false, cancelled = false;
        var counting = false;   // сервер не принял список без счёта (count=false) — все страницы со счётом, как первая
        bool countingWorked = false, sawHasNext = false;
        DateTimeOffset? lastCreated = null;
        Task<IntraserviceSearchResult>? ahead = null;   // следующая страница, запрошенная заранее
        // страница списка занимает одно из четырёх мест, как и запрос по заявке: «не больше четырёх запросов разом» держится и тогда,
        // когда следующая страница запрошена заранее, пока идёт выгрузка этой (паузы между повторами место не держат). Общее число
        // (для хода и «осталось») — только с первой страницы; дальше сервер не считает, а говорит, есть ли следующая (count=false):
        // счёт по умолчанию упирается в тысячу, и если сервер вслед за ним обрезает и сам список, дальше тысячи прочли бы пустоту
        Task<IntraserviceSearchResult> Fetch(int page) => Retrying(async () =>
            {
                await job.Gate.WaitAsync(job.Abort.Token);
                try
                {
                    return await client.GetTasksAsync(listQuery, page, job.Abort.Token, pageSize: ListPageSize, detailed: true,
                        timeout: ListTimeout, caller: nameof(KnowledgeExport), counted: page == 1 || counting);
                }
                finally { job.Gate.Release(); }
            }, x => x.Error, true, job.Abort.Token,
            error => progress?.Report($"Список: {HttpIntraserviceClient.Brief(error)} — повторяю…"));
        try
        {
            Directory.CreateDirectory(job.TicketsDir);
            progress?.Report("Смотрю, что уже выгружено…");
            job.Moved = MigrateFlat(job.TicketsDir, job.Touch, ct);
            job.Existing = ScanNames(job.TicketsDir, null, progress, ct);
            progress?.Report("Читаю список заявок…");
            for (var page = 1; ; page++)
            {
                var r = await (ahead ?? Fetch(page));
                ahead = null;
                if (r.Error.Length > 0)
                {
                    // порядок по созданию сервер, возможно, не принял (запись сортировки на живом сервере не проверена; отказ бывает
                    // и 4xx, и 500 на незнакомое поле): с первой страницы возвращаемся к порядку по изменению. Логин и доступ тут
                    // ни при чём, а срок и «слишком часто» (408, 429) повторы уже исчерпали, — это не сортировка
                    if (page == 1 && everything && !sortFallback && HttpIntraserviceClient.HttpCode(r.Error) is >= 400 and not (401 or 403 or 408 or 429))
                    {
                        sortFallback = true;
                        listQuery = query;
                        page = 0;
                        continue;
                    }
                    // count=false живой сервер принять должен (count у него логический), но не проверено — отказ (400) на странице
                    // после первой не обрывает выгрузку: та же страница ещё раз, со счётом по умолчанию, и дальше так же
                    if (page > 1 && !counting && HttpIntraserviceClient.HttpCode(r.Error) == 400)
                    {
                        counting = true;
                        page--;
                        continue;
                    }
                    listError = r.Error;
                    break;
                }

                if (sortFallback) fallbackWorked = true;   // тот же запрос без сортировки по созданию прошёл — значит, дело было в ней
                if (counting) countingWorked = true;       // а со счётом страница пришла — значит, дело было в count=false
                sawHasNext |= r.HasNext is not null;
                total = Math.Max(total, r.Total);
                var batch = new List<IntraserviceFound>();
                var fresh = 0;
                foreach (var f in r.Found)
                {
                    if (!seen.Add(f.Id)) continue;   // список сдвинулся, пока читали, — заявка на стыке страниц пришла дважды
                    fresh++;
                    if (listQuery.Sort == TaskQuery.StableSort && f.Created is { } created)
                    {
                        if (lastCreated is { } before && created < before) unsorted = true;   // порядок, который заказали, не соблюдён
                        lastCreated = created;
                    }
                    if (listQuery.PastEnd(f)) { reachedEnd = true; break; }
                    if (listQuery.Outside(f)) { outside++; continue; }
                    if (limit > 0 && taken + batch.Count >= limit) { hitLimit = true; break; }
                    batch.Add(f);
                }
                taken += batch.Count;
                // общее число «тысяча или больше» (Capped) — лишь нижняя граница: сколько всего, неизвестно (0 — ход без «из» и
                // «осталось»), разве что «Не больше, заявок» не выше её
                var exact = !HttpIntraserviceClient.Capped(total);
                job.Target = exact ? Math.Max(everything ? total : Math.Min(limit, total), taken)
                    : limit > 0 && limit <= total ? limit : 0;
                stuck = fresh == 0 && r.Found.Count > 0;   // сервер отдаёт ту же страницу снова: page им не понимается
                if (limit > 0 && taken >= limit) hitLimit = true;
                // Нужна ли следующая страница, известно до выгрузки этой — тогда её запрос уходит сразу: пока читаются заявки, сервер
                // готовит следующую страницу, и все четыре запроса не простаивают между страницами. Сказал сервер, есть ли следующая
                // (HasNextPage), — верим. Не сказал: последняя страница — неполная, когда набрано всё обещанное; но если счёта нет
                // (общее число равно пришедшему) или он лишь нижняя граница, неполная страница ничего не доказывает — её порцию мог
                // урезать сам сервер, и конец подтвердит только пустая страница
                var more = !(reachedEnd || hitLimit || stuck || r.Found.Count == 0)
                    && (r.HasNext ?? !(exact && seen.Count >= total && r.Found.Count < ListPageSize && total > r.Found.Count));
                if (more) ahead = Fetch(page + 1);
                if (batch.Count > 0 && !await ExportBatchAsync(job, batch)) { cancelled = true; break; }
                job.Report(force: true);
                if (!more) break;
            }
        }
        catch (OperationCanceledException) { cancelled = true; }   // остановили — итог ниже
        finally
        {
            // страница, запрошенная вперёд, не понадобилась (остановили, прервали, список кончился) — дожидаемся её, не бросая запрос
            if (ahead is not null) { try { await ahead; } catch (Exception) { /* не нужна */ } }
        }

        var stopped = cancelled && ct.IsCancellationRequested;   // остановили не на самом последнем шаге, когда всё уже готово
        if (listError.Length > 0) notes.Add($"Список пришёл не целиком: {listError}");
        else if (stuck) notes.Add($"Сервер отдаёт одну и ту же страницу списка — дальше не пройти (прочитано {seen.Count})");
        else if (!cancelled && job.Fatal.Length == 0 && !reachedEnd && !hitLimit && seen.Count < total)
            notes.Add($"Список закончился раньше, чем обещал сервер ({seen.Count} из {total}) — выгружено то, что пришло. Повторите выгрузку позже: недостающее подтянется");
        if (outside > 0)
            notes.Add($"Сервер вернул заявки вне выбранного периода ({outside}) — они пропущены: условие по дате он, похоже, не применил");
        if (countingWorked)
            notes.Add("Сервер не принял список без счёта (count=false) — страницы списка шли со счётом.");
        if (fallbackWorked)
            notes.Add("Сервер не принял сортировку по дате создания — список читался по дате изменения. Заявки, тронутые за время выгрузки, могли не попасть: запустите выгрузку ещё раз, она дозагрузит");
        else if (unsorted)
            notes.Add("Сервер отдал список не по дате создания, как заказано. Заявки, тронутые за время выгрузки, могли не попасть: запустите выгрузку ещё раз, она дозагрузит");
        // счёт упёрся в потолок, ни разу не сказано, есть ли следующая страница (count=false не принят или не понят), и список
        // кончился, не перевалив за потолок: похоже, сервер обрезал сам список — «всё» выгружено не всё. Ровно тысяча найденных
        // выглядит так же — потому «похоже», и выгрузка не названа законченной
        var cut = HttpIntraserviceClient.Capped(total) && !sawHasNext && seen.Count <= total && !reachedEnd && !hitLimit && !stuck
            && listError.Length == 0 && !cancelled && job.Fatal.Length == 0;
        if (cut)
            notes.Add($"Список кончился ровно на потолке счёта сервера ({total}): если найдено больше, сервер, похоже, отдал только первые {total} — выгружено столько. Пришлите этот итог.");
        var complete = !cancelled && job.Fatal.Length == 0 && listError.Length == 0 && !stuck && !cut;
        if (job.Done == 0 && listError.Length > 0 && job.Moved == 0) return new(0, 0, 0, 0, 0, "", listError, Complete: false);
        return Conclude(job, dir, notes, stopped, complete);
    }

    /// <summary>Заявка Markdown-ом: карточка (сервис, тип, категории, группа — в строках списка их нет) и вся переписка,
    /// по формату файла выгрузки. Для просмотра, буфера обмена и файлов. Не прочиталась карточка или переписка — пустой
    /// текст и причина: половина заявки хуже, чем никакой (файл потом сочли бы неизменным). retry — сетевые сбои повторять
    /// (выгрузка файлов); просмотр не ждёт и показывает ошибку сразу.</summary>
    public static async Task<(string Text, string Error)> BuildAsync(HttpIntraserviceClient client, IntraserviceFound row,
        string url, CancellationToken ct, bool retry = false)
    {
        // ponytail: карточка — запросом на каждую заявку: в строках списка живой сервер не присылает сервис и тип. Может,
        // отдал бы их fields= у списка — не проверено на живом сервере.
        var details = await Retrying(() => client.GetTaskAsync(row.Id, ct), r => r.Error, retry, ct);
        if (details.Task is not { } task) return ("", details.Error);
        var (events, error, truncated) = await ReadLifetimeAsync(client, row.Id, ct, retry);
        return error.Length > 0 ? ("", error) : (Format(WithDetails(row, task), events, url, DateTimeOffset.Now, truncated), "");
    }

    /// <summary>Файлы по готовому списку заявок (выбранные в окне): по 4 разом; не менявшиеся с прошлой выгрузки пропускаются
    /// без запросов. notes — что сказать в итоге о списке.</summary>
    public static async Task<ExportResult> ExportRowsAsync(HttpIntraserviceClient client, IReadOnlyList<IntraserviceFound> rows,
        string dir, Func<int, string> ticketUrl, IProgress<string>? progress, CancellationToken ct, IReadOnlyList<string>? notes = null)
    {
        using var job = new Job(client, dir, ticketUrl, progress, ct) { Target = rows.Count };
        var all = new List<string>(notes ?? Array.Empty<string>());
        var finished = false;
        try
        {
            Directory.CreateDirectory(job.TicketsDir);
            job.Moved = MigrateFlat(job.TicketsDir, job.Touch, ct);
            job.Existing = ScanNames(job.TicketsDir, rows.Select(r => r.Id).ToHashSet(), progress, ct, rows.Select(r => ShardOf(r.Created)));
            finished = await ExportBatchAsync(job, rows);
        }
        catch (OperationCanceledException) { /* остановили — итог ниже */ }
        return Conclude(job, dir, all, ct.IsCancellationRequested && !finished, finished && job.Fatal.Length == 0);
    }

    // ---------- ход выгрузки ----------

    /// <summary>Конец любой выгрузки: оглавления тронутых месяцев (даже если остановили или прервали — файлы уже лежат) и
    /// строки итога о том, как она кончилась.</summary>
    private static ExportResult Conclude(Job job, string dir, List<string> notes, bool stopped, bool complete)
    {
        var done = job.Target > 0 ? $"{job.Done} из {job.Target}" : $"{job.Done}";   // 0 — сколько всего, неизвестно
        if (stopped)
            notes.Insert(0, $"Остановлено — что успели, сохранено (обработано {done}). Запустите выгрузку снова: готовое пропустится, она продолжится с этого места.");
        if (job.Fatal.Length > 0)
            notes.Insert(0, $"{job.Fatal} Что успели, сохранено (обработано {done}); исправьте причину и запустите выгрузку снова — она продолжится с этого места.");
        if (job.Moved > 0) notes.Add($"Файлы прежней раскладки переложены в папки по месяцам: {job.Moved}.");
        if (WriteIndexes(dir, job.Touched.Keys.ToList()) is { Length: > 0 } indexError) notes.Add(indexError);
        if (!stopped && job.Elapsed.TotalMinutes >= 1) notes.Add($"Заняло {HumanSpan(job.Elapsed)}.");
        return new(job.Done, job.Created, job.Updated, job.Unchanged, job.Failed, job.FirstError,
            string.Join("\n", notes.Where(n => n.Length > 0)), complete);
    }

    /// <summary>Страница (или весь выбранный список): по 4 заявки разом. false — отменили (пользователь или выключатель), часть
    /// заявок осталась необработанной.</summary>
    private static async Task<bool> ExportBatchAsync(Job job, IEnumerable<IntraserviceFound> rows)
    {
        try
        {
            await Task.WhenAll(rows.Select(f => ExportOneAsync(job, f)).ToList());
            return true;
        }
        catch (OperationCanceledException) { return false; }
    }

    private static async Task ExportOneAsync(Job job, IntraserviceFound f)
    {
        await job.Gate.WaitAsync(job.Abort.Token);
        try { await ExportCoreAsync(job, f); }
        finally { job.Gate.Release(); }
        Interlocked.Increment(ref job.Done);   // сюда не доходит отменённая: её и не считаем
        job.Report();
    }

    /// <summary>Одна заявка: не менялась — пропуск; иначе карточка и переписка с сервера и файл на место. Сбой одной заявки
    /// считается и не останавливает остальные (до выключателя Job).</summary>
    private static async Task ExportCoreAsync(Job job, IntraserviceFound f)
    {
        try
        {
            var (shard, rel) = PlaceOf(job.TicketsDir, f);
            // Файлы с номером этой заявки в имени. Нашими считаются только с нашими свойствами (source: intraservice): заметка
            // пользователя, чьё имя начинается с номера, не наша — её не удаляем (как «лишнюю») и она не мешает заявке считаться
            // неизменной. Испорченный файл с другим именем тоже останется лежать: потерять заметку хуже, чем оставить мусор
            var ours = job.Existing.GetValueOrDefault(f.Id).AllPaths()
                .Select(p => (Rel: p, Props: ReadProps(Path.Combine(job.TicketsDir, p))))
                .Where(x => x.Props?.GetValueOrDefault("source") == SourceName).ToList();
            // не менялась с прошлой выгрузки (формат тот же, лежит там, где должна, и один файл) — переписку не перечитываем
            if (ours is [var only] && string.Equals(only.Rel, rel, StringComparison.OrdinalIgnoreCase)
                && f.Changed is { } changed && IsCurrent(only.Props!, changed))
            {
                Interlocked.Increment(ref job.Unchanged);
                return;
            }

            var (text, error) = await BuildAsync(job.Client, f, job.TicketUrl(f.Id), job.Abort.Token, retry: true);
            if (error.Length > 0) { job.Fail(f.Id, error); return; }
            Volatile.Write(ref job.Transient, 0);   // сервер отвечает — серии сбоев прервались
            Volatile.Write(ref job.ServerErrors, 0);
            var path = Path.Combine(job.TicketsDir, rel);
            try
            {
                Directory.CreateDirectory(Path.GetDirectoryName(path)!);
                WriteAtomic(path, text);
                job.Touch(shard);
                // переименовали заявку (или остался дубль) — прежние наши файлы с этим номером больше не нужны
                foreach (var (stale, _) in ours)
                {
                    if (string.Equals(stale, rel, StringComparison.OrdinalIgnoreCase)) continue;
                    TryDelete(Path.Combine(job.TicketsDir, stale));
                    job.Touch(Path.GetDirectoryName(stale));
                }
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                job.WriteFailed(f.Id, ex.Message);   // занят редактором, антивирусом — в следующий раз; много подряд — диск
                return;
            }
            Volatile.Write(ref job.WriteFails, 0);
            if (ours.Count == 0) Interlocked.Increment(ref job.Created);
            else Interlocked.Increment(ref job.Updated);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            job.Fail(f.Id, $"{ex.GetType().Name}: {ex.Message}");   // неожиданное с одной заявкой не должно обрывать многочасовую выгрузку
        }
    }

    /// <summary>Вызвать call; сбой сети или сервера (HttpIntraserviceClient.IsTransient) повторить после пауз из RetryDelays.
    /// Остальное — логин, доступ, «нет такой», непонятный ответ — повтор не лечит: ответ возвращается сразу. retry == false —
    /// без повторов.</summary>
    private static async Task<T> Retrying<T>(Func<Task<T>> call, Func<T, string> errorOf, bool retry, CancellationToken ct,
        Action<string>? onRetry = null)
    {
        for (var attempt = 0; ; attempt++)
        {
            var result = await call();
            var error = errorOf(result);
            if (!retry || error.Length == 0 || !HttpIntraserviceClient.IsTransient(error)
                || attempt >= (HttpIntraserviceClient.HttpCode(error) == 500 ? Math.Min(1, RetryDelays.Length) : RetryDelays.Length)) return result;
            onRetry?.Invoke(error);
            await Task.Delay(RetryDelays[attempt], ct);
        }
    }

    /// <summary>Вся переписка заявки — страница за страницей. Сервер присылает Paginator — верим ему: «страниц больше нет» —
    /// стоп (иначе на заявке ровно в 50 записей просили бы несуществующую страницу). Не присылает — конец по неполной
    /// странице; не понимающий page сервер отдал бы ту же страницу снова — новых записей нет, стоп. Дошли до MaxLifetimePages, а
    /// страницы ещё есть, — Truncated: самые ранние записи не прочитаны (страницы идут от свежих), в файле об этом пометка.</summary>
    internal static async Task<(List<IntraserviceEvent> Events, string Error, bool Truncated)> ReadLifetimeAsync(HttpIntraserviceClient client,
        int id, CancellationToken ct, bool retry = false)
    {
        var all = new List<IntraserviceEvent>();
        var keys = new HashSet<(DateTimeOffset?, string, string, string?)>();
        for (var page = 1; page <= MaxLifetimePages; page++)
        {
            var r = await Retrying(() => client.GetLifetimePageAsync(id, page, ct), x => x.Error, retry, ct);
            if (r.Error.Length > 0) return (all, r.Error, false);
            var added = 0;
            foreach (var e in r.Events)
                if (keys.Add((e.Date, e.Author, e.Status, e.Comment))) { all.Add(e); added++; }
            if (added == 0 || (r.Paged ? !r.HasMore : r.Events.Count < HttpIntraserviceClient.LifetimePageSize)) break;
            if (page == MaxLifetimePages) return (all, "", true);
        }
        return (all, "", false);
    }

    /// <summary>Строка списка, дополненная карточкой заявки: сервис, тип, категории, дата решения и группа — из карточки,
    /// а поля, которого карточка не прислала, — из строки. Остальное (название, статус, даты, люди, описание) — из строки:
    /// по её Changed выгрузка узнаёт неизменные заявки, а по названию — имя файла.</summary>
    internal static IntraserviceFound WithDetails(IntraserviceFound f, IntraserviceTask task)
    {
        var (card, row) = (task.Extra, f.Extra);
        return f with
        {
            ExecutorGroup = task.ExecutorGroup ?? f.ExecutorGroup,
            Extra = new(card?.Service ?? row?.Service, card?.Type ?? row?.Type, card?.Categories ?? row?.Categories,
                card?.Resolved ?? row?.Resolved),
        };
    }

    /// <summary>Срок для человека: «меньше минуты», «42 мин», «3 ч 05 мин».</summary>
    internal static string HumanSpan(TimeSpan t) =>
        t.TotalMinutes < 1 ? "меньше минуты"
        : t.TotalHours < 1 ? $"{(int)t.TotalMinutes} мин"
        : $"{(int)t.TotalHours} ч {t.Minutes:00} мин";

    /// <summary>Одна выгрузка: общие счётчики, выключатель и всё, что нужно шагу одной заявки. Счётчики трогают до четырёх
    /// заявок разом (Interlocked); Existing после подготовки только читается.</summary>
    private sealed class Job : IDisposable
    {
        public readonly HttpIntraserviceClient Client;
        public readonly string TicketsDir;
        public readonly Func<int, string> TicketUrl;
        /// <summary>Отменяется и пользователем, и выключателем (Trip): всё, что идёт к серверу, смотрит на него.</summary>
        public readonly CancellationTokenSource Abort;
        public readonly SemaphoreSlim Gate = new(MaxParallel);
        /// <summary>Месяцы, где что-то записано или убрано, — их оглавления перепишутся.</summary>
        public readonly ConcurrentDictionary<string, byte> Touched = new(StringComparer.OrdinalIgnoreCase);
        public Dictionary<int, Known> Existing = new();
        /// <summary>Transient — подряд сбоев сети и сервера (обнуляется удачей); Refused — отказов сервера, которые повтор не лечит
        /// (нет такой заявки, нет доступа…), за всю выгрузку; WriteFails — подряд ошибок записи файла.</summary>
        public int Created, Updated, Unchanged, Failed, Done, Transient, ServerErrors, Refused, WriteFails;
        public int Target, Moved;
        public volatile string FirstError = "", Fatal = "";

        private int _tripped;
        private readonly IProgress<string>? _progress;
        private readonly Stopwatch _clock = Stopwatch.StartNew();
        private long _lastReport;
        private readonly object _rateLock = new();
        private long _windowAt;
        private int _windowDone;
        private double _rate;

        public Job(HttpIntraserviceClient client, string dir, Func<int, string> ticketUrl, IProgress<string>? progress, CancellationToken ct)
        {
            Client = client;
            TicketsDir = Path.Combine(dir, TicketsFolder);
            TicketUrl = ticketUrl;
            _progress = progress;
            Abort = CancellationTokenSource.CreateLinkedTokenSource(ct);
        }

        public TimeSpan Elapsed => _clock.Elapsed;

        public void Touch(string? shard)
        {
            if (!string.IsNullOrEmpty(shard)) Touched[shard] = 0;
        }

        /// <summary>Выключатель: останавливает выгрузку по причине (первая причина остаётся, как бы ни совпали две заявки).</summary>
        private void Trip(string reason)
        {
            if (Interlocked.Exchange(ref _tripped, 1) != 0) return;
            Fatal = reason;
            Abort.Cancel();
        }

        /// <summary>Заявка не прочиталась. Неверный логин — сразу стоп; серия сбоев сети без единого успеха, или ни одной
        /// удачной из первых многих — тоже: так выгрузка не долбит лежащий сервер и не жжёт часы впустую.</summary>
        public void Fail(int id, string error)
        {
            if (Interlocked.Increment(ref Failed) == 1) FirstError = $"#{id}: {error}";
            if (HttpIntraserviceClient.HttpCode(error) == 401)
                Trip("Выгрузка прервана: сервер не принимает логин и пароль (HTTP 401) — проверьте их в настройках.");
            else if (HttpIntraserviceClient.IsTransient(error))
            {
                if (HttpIntraserviceClient.HttpCode(error) == 500)
                {
                    if (Interlocked.Increment(ref ServerErrors) >= ServerErrorLimit)
                        Trip($"Выгрузка прервана: {ServerErrorLimit} заявок подряд сервер ответил ошибкой 500 (последняя — #{id}: {HttpIntraserviceClient.Brief(error)}). Похоже, он не справляется или сломан: подождите и запустите выгрузку снова.");
                }
                else if (Interlocked.Increment(ref Transient) >= TransientLimit)
                    Trip($"Выгрузка прервана: {TransientLimit} заявок подряд не прочитались из-за сети или сервера (последняя — #{id}: {HttpIntraserviceClient.Brief(error)}). Проверьте связь; если компьютер засыпал — не давайте ему.");
            }
            else if (Interlocked.Increment(ref Refused) >= NothingWorksLimit
                && Volatile.Read(ref Created) + Volatile.Read(ref Updated) + Volatile.Read(ref Unchanged) == 0)
                Trip($"Выгрузка прервана: ни одна из {NothingWorksLimit} заявок не прочиталась. Первая ошибка — {FirstError}");
        }

        public void WriteFailed(int id, string message)
        {
            if (Interlocked.Increment(ref Failed) == 1) FirstError = $"#{id}: файл не записан: {message}";
            if (Interlocked.Increment(ref WriteFails) >= WriteFailLimit)
                Trip($"Выгрузка прервана: файлы не записываются ({WriteFailLimit} подряд) — {message}. Проверьте место на диске, доступ к папке и длину пути к ней.");
        }

        /// <summary>Ход для окна: «Выгружено 1250 из 98000 · осталось ~3 ч 12 мин», не чаще четырёх раз в секунду. Сколько всего,
        /// неизвестно (Target 0: сервер досчитал только до тысячи) — вместо «осталось» скорость: «~40 в минуту».</summary>
        public void Report(bool force = false)
        {
            if (_progress is null) return;
            var now = _clock.ElapsedMilliseconds;
            if (!force && now - Volatile.Read(ref _lastReport) < 250) return;
            Volatile.Write(ref _lastReport, now);
            var done = Volatile.Read(ref Done);
            var target = Volatile.Read(ref Target);
            var line = target > 0 && target >= done ? $"Выгружено {done} из {target}" : $"Выгружено {done}";
            if (Volatile.Read(ref Failed) is > 0 and var failed) line += $" · не прочитались {failed}";
            var speed = target == 0 || target > done ? SpeedPerSecond(now, done) : 0;
            if (speed > 0)
                line += target > done ? $" · осталось ~{HumanSpan(TimeSpan.FromSeconds((target - done) / speed))}" : $" · ~{Math.Round(speed * 60)} в минуту";
            _progress.Report(line);
        }

        /// <summary>Заявок в секунду за последние полминуты, а не с самого начала: не менявшиеся заявки пролетают мгновенно, а
        /// читаемые с сервера — долго, и средняя со старта врала бы в обе стороны. Полминуты ещё не прошло — средняя со старта,
        /// но не раньше чем по десяти заявкам и пяти секундам.</summary>
        private double SpeedPerSecond(long now, int done)
        {
            lock (_rateLock)
            {
                if (now - _windowAt >= 30_000)
                {
                    _rate = (done - _windowDone) / ((now - _windowAt) / 1000.0);
                    _windowAt = now;
                    _windowDone = done;
                }
                return _rate > 0 ? _rate : done >= 10 && now >= 5_000 ? done / (now / 1000.0) : 0;
            }
        }

        public void Dispose()
        {
            Abort.Dispose();
            Gate.Dispose();
        }
    }
}
