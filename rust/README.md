# TicketBoard на Rust — прототип

Зачем: размер, память и скорость (C#-exe — 65,6 МБ, самодостаточный .NET с WPF). Что умеет и чего нет — `RELEASE.md`;
решения и проверки — `docs/HISTORY.md` (2026-11-06). Только Windows; на Linux собирается для проверок и снимков окна.

```
cd rust
cargo test                         # все проверки (~1 с после сборки); окно — программной отрисовкой, без экрана
cargo clippy --all-targets         # и под Windows: --target x86_64-pc-windows-gnu (DPAPI, реестр, память, клавиша)
cargo run --example shot -- <папка>  # снимки окна: board-light.png, board-dark.png, drag.png
cargo build --release              # exe: target/release/ticketboard-rs(.exe)
```

- `src/board.rs` — `tickets.json` (формат TicketBoard.exe) и правила доски: возраст, скрытые «Готово», лимит, перенос.
- `src/settings.rs` — `settings.json` (с комментариями и висячими запятыми, как `AppSettings`).
- `src/dpapi.rs` — пароль: DPAPI CurrentUser, без энтропии, base64.
- `src/intraservice.rs` — `GET api/task/{номер}?include=status`; ошибки — фразами C#-клиента; прокси Windows — в
  диагностике.
- `src/metrics.rs` — размер exe, память, время от запуска процесса.
- `src/app.rs` — доска → модели Slint, выбор, перенос, чтение заявки; `src/main.rs` — окно, трей, клавиша.
- `ui/board.slint` — окно (Slint 1.18, стиль Fluent, программная отрисовка); цвета — из `Themes/Tokens.*.xaml`.
- CI — `.github/workflows/rust.yml` (windows-latest), пре-релиз — по `release_tag` (`rust-X.Y.Z`).
