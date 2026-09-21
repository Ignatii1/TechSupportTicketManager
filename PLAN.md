# PLAN.md — Intraservice comments in the panel + server-side search

Round of 2026-09-21 (evening). Working doc for implementation agents; delete it at merge, the record goes to `PROGRESS.md`.
Read `AGENTS.md` first. API facts below are from *IntraService API v5.51*, extracted to plain text at
`/tmp/claude-0/-home-user-TechSupportTicketManager/a9fd112e-3d32-5cf9-a9b4-faccc88d7fc6/scratchpad/api.txt`
(page markers `===== PAGE n =====`; lifecycle pp. 64-66, task list and its parameters pp. 8-20). Read the pages, don't trust
this summary alone.

## What we are building

1. **Comments** — the Intraservice conversation on a ticket, shown in the detail panel under the local notes.
   `GET /api/tasklifetime?taskid={id}&include=status&lastcommentsontop=true&pagesize=50`
2. **Server-side search** — `Enter` in the board's search box queries the server (which searches ticket fields **and all
   comments**) and shows hits in a small window, from where a ticket can be opened or added to the board.
   `GET /api/task?search={text}&include=status&sort=Changed%20desc&pagesize=20`

## Decisions already taken — do not relitigate

- **Separate sections, not one merged timeline.** Dates from the API arrive in the *Intraservice* user's time zone (doc
  p. 10), local notes in the PC's. Interleaving them sorts silently wrong when the two differ. Notes stay the user's
  notebook; comments are the ticket's conversation, read-only here.
- **Comments are never persisted.** Not in `tickets.json`, not as a property on `Ticket`. `Ticket` is serialised by
  property name, and `MainViewModel.OnTicketChanged` schedules a save on any `PropertyChanged` outside the age whitelist —
  a collection on `Ticket` would rewrite the file on every card click. They also carry other people's names and internal
  (`IsPublic == false`) text, and the data file now sits next to a portable exe. Memory only, plus a per-session cache
  keyed by ticket number, cleared in `ApplySettings`.
- **No `fields=` on the search query.** It is documented, but an unknown field name may 400 the whole request and we
  cannot test that. Twenty fat rows are free on a LAN. `// ponytail: без fields — ответ жирнее, зато не упадёт на
  незнакомом имени поля; появится нужда экономить трафик — добавить fields и проверить на живом сервере.`
- **Status names on list/lifecycle responses come from the `Statuses` block**, not from the row: `StatusName` is
  documented under «Конкретная заявка», while `fields` for a list is «поля для списка». Hence `include=status` on both
  calls, and the `StatusId → Statuses[Id].Name → "статус {n}"` fallback the existing `Parse` already implements.
- **Add a standalone `Chip` style; do not rebase `PriorityChip` on it.** Touching the card's priority chip to save six
  lines risks a visual regression in the one place the user looks all day.
- **`Enter` in the search box is the trigger.** Not a button (the 48 px title bar is full), not an automatic fallback
  (typing «принтер» passes through several zero-result states, each firing a request, and a box that silently switches
  from filtering to querying is a box you stop trusting). `Enter` is unused there today.

## Step 1 — the client (blocking; steps 2 and 3 code against these signatures)

Owns **`TicketBoard/Services/HttpIntraserviceClient.cs`** and nothing else.

New records beside `IntraserviceTask`:

```csharp
public sealed record IntraserviceEvent(DateTimeOffset? Date, string Author, string Status, string? Comment, bool? IsPublic);
public sealed record IntraserviceLifetime(IReadOnlyList<IntraserviceEvent> Events, bool HasMore, string Error);
public sealed record IntraserviceFound(int Id, string Name, string Status, string? Creator, DateTimeOffset? Created);
public sealed record IntraserviceSearchResult(IReadOnlyList<IntraserviceFound> Found, int Total, string Error);
```

New members:

```csharp
public async Task<IntraserviceLifetime> GetLifetimeAsync(int id, CancellationToken ct = default);
public async Task<IntraserviceSearchResult> SearchAsync(string text, CancellationToken ct = default);
internal static (IReadOnlyList<IntraserviceEvent> Events, bool HasMore)? ParseLifetime(string json);
internal static (IReadOnlyList<IntraserviceFound> Found, int Total)? ParseSearch(string json);
```

- Both calls go through the existing `GetAsync(path, notFound, ct)` — do not change it; the Russian error convention and
  the timeout come for free. `notFound`: «заявка не найдена» for lifetime, «ничего не найдено» for search. A parser
  returning `null` → «непонятный ответ сервера», exactly as `GetTaskAsync` does.
- Escape the query: `Uri.EscapeDataString(text)`.
- Lifetime entry fields: `Date`, `Editor`, `EditorId`, `StatusId`, `Comments`, `IsPublic`. A bare status change has **no
  `Comments` key at all** — that is the normal case, not an error. Run `Comments` through the existing `HtmlToText`
  (comments are editor HTML like `Description`); empty or whitespace normalises to `null`.
- Search rows: `Id`, `Name`, `StatusId`, `Created`, `Creator`. A row with no `Name` is skipped, not fatal.
  `Paginator.Count` → `Total`; missing `Paginator` → `Total = Found.Count`, `HasMore = false`.
  `HasMore` for lifetime = `Paginator.Page < Paginator.PageCount`.
- **Tolerate every plausible wrapper**, since the doc only shows XML: root may be a bare array, `{"TaskLifetimes":[…]}`,
  or `{"TaskLifetimeList":{"TaskLifetimes":[…]}}`; likewise `Tasks` / `TaskList.Tasks` / `Tasks` at the root. `Prop` is
  already case-insensitive — keep using it.
- **Dates: handle three shapes** — ISO 8601, the doc's `dd.MM.yyyy HH:mm:ss` (InvariantCulture, `AssumeLocal`), and WCF
  `/Date(1447335893000)/`. The third is two lines and removes a whole class of "all dates are blank" failure.
- `IsPublic` may arrive as a JSON bool or as the string `"True"`/`"False"` (the doc's XML uses both casings elsewhere).
  Handle both; unknown → `null`.
- Extract the status-name lookup out of `Parse` into a shared private helper and use it in all three parsers. The
  existing `SelfCheck` asserts prove the extraction didn't change `Parse`.
- **`SelfCheck` additions** (same style as the existing ones, samples transcribed from the doc's own examples):
  the doc's two-entry lifetime with `Statuses` + `Paginator`; a bare array with an ISO date and an unknown `StatusId`
  (→ `"статус 7"`) and `"Comments":""` (→ `null`); `{"TaskLifetimes":[],"Paginator":{"Page":2,"PageCount":3}}` →
  `HasMore: true`; a search response with `Tasks` + `Statuses` + `Paginator.Count = 137`; a search row without `Name`
  (skipped); and `{"Message":"The request is invalid."}` → `null` for both parsers.

## Step 2 — comments in the panel (after step 1)

Owns **`ViewModels/MainViewModel.cs`**, **`Views/MainWindow.xaml`**, **`Themes/Styles.xaml`**.

```csharp
public sealed record CommentRow(string Author, DateTimeOffset? Date, string? Text, bool IsInternal, string? StatusChange);

public ObservableCollection<CommentRow> Comments { get; } = new();
public ICollectionView CommentsView { get; }            // фильтр по ShowAllEvents, как ColumnViewModel.View
[ObservableProperty] private string _commentsMessage = "";
[ObservableProperty] private bool _showAllEvents;
[ObservableProperty] private int _hiddenEventsCount;
[ObservableProperty] private bool _commentsTruncated;
[RelayCommand] private Task RefreshComments();          // ⟳ в заголовке секции, мимо кэша
private async void LoadComments(Ticket? t, bool force = false);
private readonly Dictionary<int, IReadOnlyList<CommentRow>> _commentCache = new();
private CancellationTokenSource? _commentsLookup;
```

- `LoadComments` copies the shape of `QuickCaptureViewModel.LookupTitle` exactly: `async void`, cancel the previous CTS,
  `Task.Delay(400, ct)` so arrow-keying across cards doesn't fire a request per card, swallow `OperationCanceledException`,
  and re-check `SelectedTicket == t` before writing anything (the guard `SyncAsync` uses). Called from
  `OnSelectedTicketChanged`. No ticket number → the section shows «у заявки нет номера»; no API client → «API не настроен».
- `StatusChange` is computed in the view model after sorting by `Date` desc: set it only when the entry's status differs
  from the next (older) entry's. The client stays a faithful mirror of the API.
- `OnShowAllEventsChanged` → `CommentsView.Refresh()` and recount `HiddenEventsCount`. Default `ShowAllEvents = false`:
  entries with no comment are folded away. The eye button's tooltip carries the count («Показать все события (7)») so no
  Russian plural helper is needed.
- **Panel layout.** Today rows 4 and 5 of `PanelHost` are the notes header and the notes `ScrollViewer`. Merge them into
  one `*` row holding a single `ScrollViewer` over a `StackPanel`: notes header → notes `ItemsControl` → `Rectangle`
  divider → comments header (title, count, ⟳, eye) → `CommentsMessage` line (`StringToVis`, same convention as
  `SyncMessage`) → comments `ItemsControl` over `CommentsView` → truncation caption. One scroll region — two scrollbars
  in a 380 px panel is the thing to avoid. **The note input footer stays pinned in its own row.**
- One `CommentItem` `DataTemplate` (`DataType="{x:Type vm:CommentRow}"`): line 1 = `Mono12` date + `Caption` author
  (`TextTrimming="CharacterEllipsis"` — the panel is 380 px and Russian names are long) + optional «внутр.» and
  status chips; line 2 = `Body`, `TextWrapping="Wrap"`, `Visibility` via `StringToVis` so a status-only entry collapses
  to one line. One template covers both kinds; no `DataTemplateSelector`.
- `Styles.xaml`: add `CommentRow` (Border: `SubtleFill` background, `CornerRadius="4"`, a 2 px left rail in
  `ControlStrokeBottom`) and a neutral `Chip`. Every brush named here already exists in **both** token files — do not add
  a colour key. Leave `PriorityChip` alone.

## Step 3 — server-side search (after step 1, parallel with step 2)

Owns **`ViewModels/SearchViewModel.cs`** *(new)*, **`Views/SearchWindow.xaml(.cs)`** *(new)*,
**`Views/MainWindow.xaml.cs`**, **`App.xaml.cs`**. It must not touch `MainWindow.xaml`, `MainViewModel.cs` or
`Styles.xaml` — step 2 owns those, and the two run at the same time.

```csharp
public sealed partial class FoundTicketViewModel : ObservableObject   // Id, Title, Status, Creator, Created, Url, DisplayNumber
{ [ObservableProperty] private bool _onBoard; }

public sealed partial class SearchViewModel : ObservableObject
{
    public SearchViewModel(MainViewModel board, AppSettings settings, HttpIntraserviceClient? intraservice);
    public void ApplySettings(HttpIntraserviceClient? intraservice);
    public ObservableCollection<FoundTicketViewModel> Results { get; } = new();
    [ObservableProperty] private string _query = "";
    [ObservableProperty] private string _message = "";   // «ищу…» / «ничего не найдено» / ошибка / «показаны первые 20 из 137»
    [ObservableProperty] private bool _isBusy;
    [RelayCommand] private Task Search();
    [RelayCommand] private void AddToBoard(FoundTicketViewModel? row);
    [RelayCommand] private void OpenInBrowser(FoundTicketViewModel? row);
}
```

- `SearchWindow`: `ui:FluentWindow`, Mica, ~560×620, `WindowStartupLocation="CenterOwner"`, `ShowInTaskbar="False"`,
  `Esc` hides, `OnClosing` → Cancel + Hide — the same convention as `QuickCaptureWindow`. `public void ShowSearch(string
  query)` sets `Query`, shows, activates, focuses and selects the box, and runs the search if the query is long enough.
- Query shorter than 3 characters → no request, message «минимум 3 символа».
- A row already on the board: `OnBoard` computed by matching `IntraserviceFound.Id` against
  `board.AllTickets.Select(t => t.IntraserviceId)`, flipped to `true` right after adding. Such a row is dimmed with the
  existing `BoolToOpacityConverter` and says «уже на доске» instead of showing the add button.
- Adding reuses the existing path, no new method on `MainViewModel`:
  `board.AddFromCapture(row.Url.Length > 0 ? $"{row.Url} {row.Title}" : $"#{row.Id}", TicketPriority.Mid)`.
  **The empty-URL branch matters:** with no base URL configured `$"{row.Url} {row.Title}"` would carry no number at all;
  `$"#{row.Id}"` gives the parser the number and the existing `SyncAsync` fills in the real title straight after.
- The result URL is built from `settings.IntraserviceBaseUrl` as `$"{b}/Task/View/{id}"`.
  `// ponytail: адрес заявки собирается в двух местах (ещё MainViewModel.TicketUrl) — поменяется путь, править оба.`
- The row template uses only styles that exist today (`Mono12`, `Body`, `Caption`, `CaptionTertiary`, `CardOpenButton`,
  `BoolToVis`, `InverseBoolToVisibilityConverter`, `BoolToOpacityConverter`) plus a window-local border style. **No
  dependency on step 2's `Chip`** — the two steps must be mergeable in either order.
- `MainWindow.xaml.cs`: add `public event Action<string>? ServerSearchRequested;` and, right after the `Ctrl+N` block and
  **before** `if (inText) return;`:

  ```csharp
  if (e.Key == Key.Enter && SearchBox.IsKeyboardFocusWithin)
  {
      ServerSearchRequested?.Invoke(SearchBox.Text);
      e.Handled = true;
      return;
  }
  ```

  **`IsKeyboardFocusWithin`, not `ReferenceEquals(Keyboard.FocusedElement, SearchBox)`:** `ui:TextBox` is a composite
  control, so the focused element is almost certainly its inner `TextBox` part. The existing `Esc` branch uses the
  `ReferenceEquals` form and has never been confirmed to work on Windows — fix it the same way while you are in the file.
- `App.xaml.cs`: `_searchVm` / `_search` fields, constructed after `_capture`; `_main.ServerSearchRequested += text =>
  _search.ShowSearch(text);`; `ApplySettings` → `_searchVm.ApplySettings(intraservice)`. No csproj change (XAML and CS
  are globbed).

## Rules for every implementation agent

1. `AGENTS.md` rules hold: Russian UI text and code comments, English commit messages, no new NuGet packages, no
   one-implementation interfaces, `// ponytail:` on deliberate shortcuts naming their ceiling.
2. **Do not run git. Do not touch `PROGRESS.md`, `PLAN.md`, `README.md`, `TicketBoard/README.md` or `AGENTS.md`** — the
   orchestrator commits and writes all the docs.
3. Stay inside your file list. If a fix needs a file another step owns, stop and report it instead of editing it.
4. Nothing can be built or run here: no .NET SDK, its download is blocked by network policy, and the UI is Windows-only.
   Never claim a UI change works. State plainly what you could not verify.
5. Re-read your own diff adversarially before reporting — a XAML typo costs a full CI round trip.
6. Report: what changed and where, every choice you made where this plan left room, what you could not verify, and
   anything suspicious you left alone.

## Review gates

1. **Orchestrator read** of each step's diff against its brief and file list, plus the XAML checklist: every
   `StaticResource` key exists, every `DynamicResource` colour key exists in **both** token files, `RelativeSource`
   ancestors are real, `Grid.Row`/`Grid.Column` fit the definitions.
2. **CI compile** — push, then `workflow_dispatch` `build.yml` on the branch. It is the only compiler available.
3. **`/code-review high`** over `main..claude/beautiful-albattani-pd6q7i`, fixes re-pushed, CI re-run.
4. **The user on Windows**, with the checklist below. Nothing here has ever touched a real server.

## What the user has to check on Windows (nothing below is verifiable here)

1. `/` → type → **Enter** opens the search window. This is the single most likely thing to be silently broken (see the
   `IsKeyboardFocusWithin` note). `Esc` in the search box still clears it.
2. Search a word that appears only inside a comment, never in a title — that is the proof the server really searches
   comments, which is the whole point of the feature.
3. A hit already on the board is dimmed and says «уже на доске»; ↗ and double-click open the right page; «+ На доску»
   creates a card whose title fills in from Intraservice a moment later.
4. Select a card with a ticket number → comments appear within about a second. Arrow-key across several cards fast:
   no stacked requests, and never another ticket's comments under the selected card.
5. Status-only events are hidden by default; the eye reveals them; the number in its tooltip matches what appears.
6. Dates and author names are non-empty and plausible. **If dates are blank, send one raw response** — it means the
   server uses a date format none of the three parsers handle.
7. A comment marked internal in the web UI carries the «внутр.» chip.
8. Disconnect the VPN and click a card: the section says «сервер недоступен», no exception dialog. And `tickets.json`
   keeps its modification time while you click through cards — that proves comments are not being persisted.
9. Both themes, and the panel in overlay mode (window narrower than 1100 px).
