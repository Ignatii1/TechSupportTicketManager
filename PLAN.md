# PLAN.md — import the tickets where I am Исполнитель

Round of 2026-09-22. Working doc for implementation agents; delete at merge, the record goes to `PROGRESS.md`.
This is API-IDEAS item 1.1. Read `AGENTS.md` first. API doc extracted to plain text at
`/tmp/claude-0/-home-user-TechSupportTicketManager/a9fd112e-3d32-5cf9-a9b4-faccc88d7fc6/scratchpad/api.txt`
(page markers `===== PAGE n =====`; current user p. 56-57, statuses p. 38-40, task list and filters p. 14-20).

## What the user asked for, decided with him

- **On demand only.** A tray menu item and a button on the board. Nothing imports by itself — no startup import, no timer.
- **Open tickets only.** Closed = the status has `IsFixed` («Заявка выполнена») or `IsFinal` («Конечный») **or** its name is in
  a configurable list, defaulting to: `Выполнена`, `Ожидание ответа с автозакрытием`, `Закрыта`, `Отменена`.
  The name list is not optional decoration: «Ожидание ответа с автозакрытием» is a *waiting* status, so the server almost
  certainly does not flag it, yet the user counts it as closed. Status names differ per server — hence a setting.
- **Everything lands in «Входящие».** The board stays the user's own workflow; no status→column mapping.
- Re-running the import must be safe: a ticket already on the board (matched by `IntraserviceId`) is skipped, never
  duplicated and never moved between columns.

## Step 1 — the client (blocking; steps 2 and 3 code against these signatures)

Owns **`TicketBoard/Services/HttpIntraserviceClient.cs`** and nothing else.

```csharp
public sealed record IntraserviceStatus(int Id, string Name, bool IsFixed, bool IsFinal);

public async Task<(int? Id, string Error)> GetCurrentUserIdAsync(CancellationToken ct = default);
public async Task<(IReadOnlyList<IntraserviceStatus> Statuses, string Error)> GetStatusesAsync(CancellationToken ct = default);
public async Task<IntraserviceSearchResult> GetExecutorTasksAsync(int executorId, IReadOnlyCollection<int> statusIds, int page, CancellationToken ct = default);

internal static int? ParseCurrentUserId(string json);
internal static IReadOnlyList<IntraserviceStatus>? ParseStatuses(string json);
```

- `GetCurrentUserIdAsync` → `GET api/user?getcurrentuserinfo=true`. **The doc's XML root is `<CurrenUserInfo>` — that
  missing `t` is the doc's own spelling.** Accept a bare object with `Id`, and both `CurrenUserInfo` and
  `CurrentUserInfo` as a wrapper. `notFound` message: «не удалось определить пользователя».
- `GetStatusesAsync` → `GET api/taskstatus`. Fields `Id`, `Name`, `IsFixed`, `IsFinal` (the existing `Bool` helper already
  copes with `true` and `"True"`). Root may be a bare array or wrapped — reuse `Unwrap` with a name like
  `TaskStatusView`/`Statuses` as the doc's XML is `<ArrayOfTaskStatusView>`; tolerate a bare array, which is the likely
  JSON. A row with no `Id` is skipped.
- `GetExecutorTasksAsync` → `GET api/task?ExecutorIds={id}&StatusIds={ids}&include=status&sort=Changed%20desc&pagesize=200&page={page}`.
  **Reuse `ParseSearch` verbatim** — this is the same `Tasks` + `Statuses` + `Paginator` shape the search already parses.
  Keep the "no `fields=`" decision: an unknown field name could reject the whole request and we cannot test that here.
- **Add `string? Description = null` as the last positional parameter of `IntraserviceFound`** and fill it in `ParseSearch`
  through the existing `HtmlToText` (descriptions are editor HTML like everywhere else). A default value keeps every
  existing construction site compiling. It costs nothing on the wire — we already receive the field.
- `SelfCheck` additions in the existing style: the doc's own current-user sample both bare and wrapped under the
  misspelled key; a status list with mixed `IsFixed`/`IsFinal` as bool and as `"True"`; a status row with no `Id`
  (skipped); `{"Message":"The request is invalid."}` → `null` for both new parsers; and one search sample proving
  `Description` is HTML-stripped.

## Step 2 — the import itself (after step 1)

Owns **`TicketBoard/ViewModels/MainViewModel.cs`** and **`TicketBoard/Services/AppSettings.cs`**.

`AppSettings`: one new property, no settings-window field (it is a list, and the file is hand-editable and now sits next
to the exe). A row in the README settings table is step 4's job.

```csharp
/// <summary>Названия статусов Интрасервиса, которые считаем закрытыми при импорте, сверх признаков
/// «Заявка выполнена» и «Конечный». Правится руками в settings.json.</summary>
public string[] ClosedStatusNames { get; set; } = { "Выполнена", "Ожидание ответа с автозакрытием", "Закрыта", "Отменена" };
```

`MainViewModel`:

```csharp
[ObservableProperty] private bool _isImporting;      // кнопка и пункт меню на время импорта недоступны
[RelayCommand(CanExecute = nameof(CanImport))] private async Task ImportMine();
private bool CanImport() => !IsImporting;
```

Sequence, all through the existing client, all errors as short Russian strings:

1. No API client → message «API не настроен: трей → Настройки…», stop.
2. `GetCurrentUserIdAsync` → on error, show it and stop.
3. `GetStatusesAsync` → open ids = statuses where **not** (`IsFixed` or `IsFinal` or the name matches
   `ClosedStatusNames`, trimmed, `OrdinalIgnoreCase`). If the open list is empty, stop with
   «все статусы считаются закрытыми — проверьте ClosedStatusNames в settings.json» rather than importing the world.
4. Page through `GetExecutorTasksAsync` from page 1 until the rows collected reach `Total`, a page comes back empty, or
   **10 pages** — whichever first. `// ponytail: потолок 10 страниц по 200 — 2000 заявок; упрётся — добавить постраничную докачку.`
5. Skip any `Id` already on the board (`AllTickets` where `IntraserviceId` is not null). Count them separately.
6. For each new one build the ticket **before** `Track`, so the property sets don't each schedule a save:
   `Title = Name`, `IntraserviceId = Id`, `Url` = the existing `TicketUrl(id)` helper, `Description` = what came back,
   `ExternalStatus = Status`, `LastSyncAt = now`, priority `Mid` (server priorities are not mapped yet — API-IDEAS 2.2).
   Then `Track`, append to the **Inbox** column in the order received, and `ScheduleSave()` **once** at the end.
   No `MarkAppear`: fifty cards animating in at once is noise, not feedback.
7. Report with a `MessageBox` (the app's existing way of talking, see `DeleteSelected`): «Добавлено: N, уже было: M» —
   or the error. Wrap the whole body so `IsImporting` is cleared in a `finally`.

## Step 3 — how it is reached (after step 1, parallel with step 2)

Owns **`TicketBoard/App.xaml.cs`** and **`TicketBoard/Views/MainWindow.xaml`**. Must not touch `MainViewModel.cs` or
`AppSettings.cs` — step 2 owns those and runs at the same time. Bind to `ImportMineCommand`; do not add view model members.

- Tray menu: a «Импорт моих заявок» item in `SetupTray`, right after «Быстрое добавление», using the existing
  `MenuItemFor` helper. It is the main entry point.
- Board: an icon button in the `ui:TitleBar.Header` toolbar, immediately left of the settings gear, `Style="{StaticResource
  IconButton}"`, tooltip «Импорт моих заявок из Интрасервиса». The toolbar is already full (search 260 px + filter 150 px
  + toggle + «+ Заявка» + gear), so an icon-only button is the only thing that fits — no text label.
  Pick the symbol from WPF-UI 4.3.0's `SymbolRegular`; verify the name exists rather than guessing
  (`ArrowDownload16` is a likely candidate, `PersonArrowRight16` another). Symbols already proven in this repo:
  `Add16`, `Search16`, `Settings16`, `Open16`, `Note16`, `Dismiss16`, `Checkmark12`, `Warning12`, `ArrowSync16`, `Eye16`.

## Rules for every implementation agent

1. `AGENTS.md` rules hold: Russian UI text and code comments, English commit messages, no new NuGet packages,
   `// ponytail:` on deliberate shortcuts naming their ceiling.
2. **Do not run git. Do not touch `PROGRESS.md`, `PLAN.md`, `README.md`, `TicketBoard/README.md` or `AGENTS.md`** — the
   orchestrator commits and writes every doc.
3. Stay inside your file list. If a fix needs a file another step owns, stop and report it instead of editing it.
4. Nothing can be built or run here: no .NET SDK, its download is blocked, the app is Windows-only. CI after the push is
   the only compiler. Never claim a UI change works; say plainly what you could not verify.
5. Re-read your own diff adversarially before reporting.
6. Report: what changed and where, every choice the plan left open, and what you could not verify.

## Review gates

1. Orchestrator read of each diff against its brief and file list.
2. CI compile (`workflow_dispatch` on the branch) — the only compiler.
3. `/code-review high` over the round's diff; fixes re-pushed; CI re-run.
4. The user on Windows.

## What the user has to check on Windows

1. Tray → «Импорт моих заявок» (and the toolbar button) pulls in the tickets assigned to him and says how many.
2. Nothing closed comes in — specifically nothing in «Выполнена», «Ожидание ответа с автозакрытием», «Закрыта»,
   «Отменена». If something closed slips through, the status name on the card says which name to add to
   `ClosedStatusNames` in `settings.json`.
3. Running it twice adds nothing the second time and moves no existing card between columns.
4. Cards that were already on the board keep their column, their notes and their priority.
5. Imported cards carry the real title, the description, the Intraservice status, and ↗ opens the right page.
6. With the API misconfigured or the VPN down, it says so in one message box instead of throwing.
7. The count in the message box matches what actually appeared in «Входящие».
