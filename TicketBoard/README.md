# Заявки (TicketBoard)

Личный трекер заявок Интрасервиса: доска из четырёх колонок, живёт в трее, окно быстрого добавления по глобальному хоткею.
WPF + .NET 10 + WPF-UI (Fluent / Mica). Данные — локально в `%APPDATA%\TicketBoard`.

## Запуск из исходников

1. Поставить .NET 10 SDK: https://dotnet.microsoft.com/download (или без админа —
   `dotnet-install.ps1 -Channel 10.0`, см. https://learn.microsoft.com/dotnet/core/tools/dotnet-install-script).
2. В папке проекта:
   ```
   dotnet run
   ```

## Сборка одного .exe

```
dotnet publish -c Release
```
Результат: `bin\Release\net10.0-windows\win-x64\publish\TicketBoard.exe` — самодостаточный, .NET на машине не нужен.
Положить куда угодно, запустить, в трее включить «Запускать вместе с Windows».

## Как пользоваться

| Действие | Как |
|---|---|
| Быстро добавить заявку из любого окна | `Win+Shift+Space` → вставить ссылку (или она уже подставится из буфера) → `1/2/3` приоритет → `Enter` |
| Быстрое добавление с доски | `N` или кнопка «+ Заявка» |
| Выбрать карточку | клик, `↑ ↓` в колонке |
| Перенести между колонками | перетащить мышью, `← →`, статус в панели, или правая кнопка |
| Открыть / скрыть панель деталей | `Enter` по выбранной карточке, `Esc` — закрыть |
| Открыть в Интрасервисе | кнопка ↗ на карточке, двойной клик, ссылка в панели |
| Поиск | `/` или `Ctrl+F`; ищет по номеру, названию, описанию, заметкам |
| Заметка к заявке | поле внизу панели → `Enter` |
| Удалить | правая кнопка → Удалить, или `Ctrl+Del` |
| Выход | только из меню трея (крестик сворачивает в трей) |

Название, ссылку и описание в панели можно править прямо по месту — они выглядят как текст, рамка появляется при наведении.

## Настройки — `%APPDATA%\TicketBoard\settings.json`

Создаётся при первом запуске. Правится руками, применяется после перезапуска.

| Поле | Что делает | По умолчанию |
|---|---|---|
| `Hotkey` | глобальный хоткей быстрого добавления | `Win+Shift+Space` |
| `IntraserviceIdPattern` | регулярка, группа 1 — номер заявки (`…/Task/View/702180`, `#702180`); если не совпало — последнее число в ссылке | см. выше |
| `HideDoneOlderThanDays` | скрывать «Готово» старше N дней | 7 |
| `WipLimit` | подсветка перегруза колонки «В работе» | 5 |
| `OverdueDays` | дней в колонке до «просрочена» (красный); за день до этого — жёлтый; «Готово» не подсвечивается | 3 |
| `IntraserviceBaseUrl`, `IntraserviceApiToken` | под API (пока не используются) | — |

Если ссылки Интрасервиса выглядят иначе, чем `…/Task/View/702180`, поправь `IntraserviceIdPattern`.

## Данные

- `%APPDATA%\TicketBoard\tickets.json` — все заявки, запись атомарная.
- `%APPDATA%\TicketBoard\backups\tickets-ГГГГ-ММ-ДД.json` — бэкап раз в день, хранится 30 штук.
- Битый файл не затирается, а переименовывается в `tickets.json.corrupt-…`.

## Структура

```
Models/Ticket.cs                 заявка, заметка, статусы; поля под API (IntraserviceId, ExternalStatus, LastSyncAt)
Services/TokenTheme.cs           подключает Themes/Tokens.*.xaml под тему, акцент — системный
Services/TicketStore.cs          JSON-хранилище, бэкапы
Services/AppSettings.cs          settings.json
Services/IntraserviceLinkParser  ссылка → номер заявки
Services/HotkeyService.cs        RegisterHotKey
Services/AutostartService.cs     HKCU\...\Run
Services/IIntraserviceClient.cs  заготовка под API
ViewModels/MainViewModel.cs      доска, команды, фильтры, автосохранение
ViewModels/ColumnViewModel.cs    колонка + приём drag&drop
Views/MainWindow.xaml            доска, панель деталей — по макетам Claude Design
Views/QuickCaptureWindow.xaml    окно быстрого добавления
Themes/Tokens.Light|Dark.xaml    цвета из tokens.css макета
Themes/Styles.xaml               карточка, бейджи, чипы, kbd, кнопки
```

## Дальше

- [x] вёрстка по макетам из Claude Design
- [ ] бейдж с числом входящих на иконке в трее
- [x] анимация появления карточки после переноса
- [ ] API Интрасервиса: `HttpIntraserviceClient`, подтягивать название и статус по номеру
- [ ] окно настроек вместо правки json
