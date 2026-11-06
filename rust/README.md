# TicketBoard на Rust — прототип

Зачем: размер, память и скорость (C#-exe — 65,6 МБ, самодостаточный .NET с WPF). Что умеет и чего нет — `RELEASE.md`;
решения и проверки — `docs/HISTORY.md` (2026-11-06). Только Windows; на Linux собирается для проверок и снимков окна.

```
cd rust
cargo test                         # все проверки (~5 с после сборки); окно — программной отрисовкой, без экрана
cargo clippy --all-targets         # и под Windows: --target x86_64-pc-windows-gnu (DPAPI, реестр, память, клавиша)
cargo run --example shot -- <папка>  # снимки окна: board-light.png, board-dark.png, drag.png
cargo build --release              # exe: target/release/ticketboard-rs(.exe)
```

- `src/board.rs` — `tickets.json` (формат TicketBoard.exe) и правила доски: возраст, скрытые «Готово», лимит, перенос.
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
- `src/app.rs` — доска → модели Slint, выбор, перенос, чтение заявки с перепиской, «Проверить подключение»;
  `src/main.rs` — окно, трей, клавиша.
- Проверки — `tests/*.rs`: образцы C#-проверок один в один (`parse`, `html`, `errors`, `query`, `rules`, `link`),
  клиент — против поддельного сервера с маршрутами (`tests/common`), окно — `ui.rs`. Мутанты:
  `cargo mutants -f <файл> --cargo-test-arg=--test --cargo-test-arg=<проверки>` (копии, рабочее дерево не трогается).
- `ui/board.slint` — окно (Slint 1.18, стиль Fluent, программная отрисовка); цвета — из `Themes/Tokens.*.xaml`.
- CI — `.github/workflows/rust.yml` (windows-latest), пре-релиз — по `release_tag` (`rust-X.Y.Z`).
