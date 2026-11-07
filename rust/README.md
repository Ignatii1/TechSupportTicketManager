# TicketBoard на Rust — прототип

Зачем: размер, память и скорость (C#-exe — 65,6 МБ, самодостаточный .NET с WPF). Что умеет и чего нет — `RELEASE.md`;
решения и проверки — `docs/HISTORY.md` (2026-11-06, 2026-11-07). Только Windows; на Linux собирается для проверок и снимков окна.

```
cd rust
cargo test                         # все проверки (~5 с после сборки); окно — программной отрисовкой, без экрана
cargo clippy --all-targets         # и под Windows: --target x86_64-pc-windows-gnu (DPAPI, реестр, память, клавиша)
cargo run --example shot -- <папка>  # снимки: board-light.png, board-dark.png, drag.png, capture.png
cargo build --release              # exe: target/release/ticketboard-rs(.exe)
```

- `src/board.rs` — `tickets.json` (формат TicketBoard.exe: все поля `Ticket` в том порядке, в каком их пишет C#; чужие
  поля сохраняются) и правила доски: возраст, скрытые «Готово», лимит, перенос, заметки, новая карточка.
- `src/store.rs` — запись `tickets.json` как `TicketStore`: через `.tmp`, копия за день в `backups/`, 30 копий.
- `src/single.rs` — хозяин доски один: мьютекс TicketBoard.exe; не наш — доска только читается.
- `src/journal.rs` — `errors.log` (только у своей доски); `src/capture.rs` — быстрое добавление; `src/sync.rs` — что ⟳
  меняет в карточке (`Apply`); `src/refresh.rs` — импорт моих и F5 без окна (что добавить, что перечитать, что закрыто,
  итог и вопрос о переносе); `src/hotkey.rs` — клавиша из настроек; `src/clipboard.rs` — текст из буфера.
- `src/settings.rs` — `settings.json` (с комментариями и висячими запятыми, как `AppSettings`).
- `src/dpapi.rs` — пароль: DPAPI CurrentUser, без энтропии, base64.
- `src/intraservice/` — клиент Интрасервиса (слой 1, перенос `HttpIntraserviceClient`): `mod.rs` — типы и запросы
  (карточка, страница истории, пользователь, статусы, «Проверить», список по отбору, «мои открытые»), журнал `Log`,
  прокси Windows в диагностике; `parse.rs` — разбор всех ответов (имена полей API — только там); `html.rs` — html → текст
  своими сканерами (без регулярок: полный `regex` — +1,4 МБ) и сущности; `errors.rs` — тексты ошибок; `charset.rs` —
  тело по BOM и `charset`; `query.rs` — адрес списка (`TaskQuery`).
- `src/rules.rs` — правила автообновления (`AutoSyncRules`) и закрытые статусы; `src/link.rs` — ссылка и номер в тексте
  (шаблон из настроек — `regex-lite`, +83 КБ).
- `src/metrics.rs` — размер exe, память, время от запуска процесса.
- `src/app.rs` — доска → модели Slint, выбор, правки (запись через 600 мс и при выходе), панель, окно добавления, ⟳ с
  перепиской, импорт и F5 в потоках, окна поверх доски по очереди, тихое чтение открытой карточки (пауза 400 мс,
  свежесть 2 мин), «Проверить подключение»; `src/main.rs` — окно, трей, клавиша, мьютекс.
- Проверки — `tests/*.rs`: образцы C#-проверок один в один (`parse`, `html`, `errors`, `query`, `rules`, `link`,
  `store` — образец `StoreCheck` и золотой текст, `capture`, `sync`, `refresh`), клиент — против поддельного сервера с
  маршрутами (`tests/common`), окно — `ui.rs` (и запись, и режим чтения; ответы потоков — `pump_until` с условием). Мутанты:
  `CARGO_PROFILE_DEV_DEBUG=0 cargo mutants -j 4 -f <файл> -C=--test -C=<проверки>` (копии, рабочее дерево не
  трогается; `-C` — собирать только эти проверки, ~8 с на мутанта).
- `ui/board.slint` — окно (Slint 1.18, стиль Fluent, программная отрисовка); цвета — из `Themes/Tokens.*.xaml`.
- CI — `.github/workflows/rust.yml` (windows-latest), пре-релиз — по `release_tag` (`rust-X.Y.Z`).
