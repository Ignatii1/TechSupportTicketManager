using System.Globalization;
using System.IO;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using CommunityToolkit.Mvvm.Input;
using H.NotifyIcon;
using H.NotifyIcon.Core;
using H.NotifyIcon.Interop;
using TicketBoard.Converters;
using TicketBoard.Models;
using TicketBoard.Services;
using TicketBoard.ViewModels;
using TicketBoard.Views;
using Wpf.Ui.Appearance;

namespace TicketBoard;

public partial class App : Application
{
    /// <summary>Данные лежат рядом с exe: папку с программой можно целиком скопировать на другой ПК или на флешку.
    /// Цена — exe должен лежать там, куда есть запись (не Program Files).</summary>
    public static string DataDir { get; } = AppContext.BaseDirectory;

    private Mutex? _mutex;
    private bool _ownsMutex;
    private HotkeyService? _hotkeys;
    private TaskbarIcon? _tray;
    private MainViewModel? _vm;
    private MainWindow? _main;
    private QuickCaptureWindow? _capture;
    private QuickCaptureViewModel? _captureVm;
    private SearchWindow? _search;
    private SearchViewModel? _searchVm;
    private SettingsWindow? _settingsWindow;
    private MenuItem? _captureItem;
    private AppSettings? _settings;
    private HttpIntraserviceClient? _intraservice;   // текущий клиент API — его же берёт ответ для Claude
    private ClipboardWatcher? _clipboard;            // не null — отвечаем Claude через буфер (ClaudeRelayEnabled)
    private bool _relayBusy;
    private bool _relayAgain;   // буфер менялся, пока собирался ответ, — посмотреть его ещё раз
    private Action? _onNotificationClick;   // что сделать по щелчку на последнем уведомлении; null — ничего

    protected override void OnStartup(StartupEventArgs e)
    {
        _mutex = new Mutex(true, @"Local\TicketBoard.SingleInstance", out _ownsMutex);
        if (!_ownsMutex)
        {
            Shutdown();
            return;
        }

        base.OnStartup(e);
        DispatcherUnhandledException += (_, ex) => { ShowError(ex.Exception); ex.Handled = true; };
        // ошибки фоновых задач (синхронизация с Интрасервисом и т.п.) иначе пропадают молча; всплывают при сборке мусора
        TaskScheduler.UnobservedTaskException += (_, ex) => { ex.SetObserved(); Dispatcher.BeginInvoke(() => ShowError(ex.Exception)); };
        // падение не в UI-потоке не спасти, но пусть останется в логе
        AppDomain.CurrentDomain.UnhandledException += (_, ex) => LogError(ex.ExceptionObject as Exception);

        // ответ сервера, который не разобрался, — сразу в errors.log: по нему и чинится разбор
        HttpIntraserviceClient.LogUnparsed = AppendLog;
        HttpIntraserviceClient.SelfCheck();
        IntraserviceLinkParser.SelfCheck();
        if (!CanWriteToDataDir())
        {
            MessageBox.Show($"Нет доступа на запись в папку программы:\n{DataDir}\n\n" +
                "Перенеси TicketBoard.exe в папку, куда можно писать (например %LOCALAPPDATA%\\Programs\\TicketBoard), и запусти снова.",
                "Заявки", MessageBoxButton.OK, MessageBoxImage.Error);
            Shutdown();
            return;
        }
        var settings = _settings = AppSettings.Load(DataDir, out var settingsProblem);
        var parser = new IntraserviceLinkParser(settings);
        var intraservice = _intraservice = HttpIntraserviceClient.From(settings);
        _vm = new MainViewModel(new TicketStore(DataDir), settings, parser, intraservice) { Log = AppendLog };

        // Тема: WPF-UI следит за системой, мы подкладываем свои токены и иконку трея под неё.
        ApplicationThemeManager.Changed += (theme, _) =>
        {
            TokenTheme.Apply(theme);
            UpdateTrayIcon();
        };
        ApplicationThemeManager.ApplySystemTheme();
        TokenTheme.Apply(ApplicationThemeManager.GetAppTheme());

        _main = new MainWindow(_vm);
        _captureVm = new QuickCaptureViewModel(parser, settings, intraservice);
        _capture = new QuickCaptureWindow(_vm, _captureVm, parser);
        _searchVm = new SearchViewModel(_vm, settings, intraservice);
        _search = new SearchWindow(_searchVm);
        _vm.CaptureRequested += () => _capture.ShowCapture();
        // уведомления автообновления: по щелчку — доска, а о закрытых ещё и вопрос о переносе
        _vm.Notify += (title, text, onClick) => ShowTrayNotification(title, text, NotificationIcon.Info, () =>
        {
            _main!.ShowAndActivate();
            onClick?.Invoke();
        });
        _vm.SettingsRequested += ShowSettings;
        _main.ServerSearchRequested += text => _search.ShowSearch(text);

        SetupTray(settings);
        SetupHotkey(settings);
        WarnIfInsecure(settings, intraservice);
        if (settingsProblem.Length > 0)
        {
            AppendLog(settingsProblem);
            ShowTrayNotification("Настройки не прочитаны", settingsProblem, NotificationIcon.Warning);
        }
        ApplyRelay(settings);

        if (!e.Args.Contains("--minimized"))
            _main.ShowAndActivate();
    }

    private void SetupTray(AppSettings settings)
    {
        var menu = new ContextMenu();
        menu.Items.Add(MenuItemFor("Открыть", () => _main!.ShowAndActivate()));
        _captureItem = MenuItemFor($"Быстрое добавление\t{settings.Hotkey}", () => _capture!.ShowCapture());
        menu.Items.Add(_captureItem);
        // главный вход в импорт: работает и когда окно доски ещё ни разу не открывали (запуск с --minimized).
        // Execute у команды CanExecute не проверяет, поэтому проверяем сами — второй импорт поверх идущего не нужен.
        var importItem = MenuItemFor("Импорт моих заявок", () =>
        {
            if (_vm!.ImportMineCommand.CanExecute(null)) _vm.ImportMineCommand.Execute(null);
        });
        menu.Items.Add(importItem);
        var refreshItem = MenuItemFor("Обновить статусы\tF5", () =>
        {
            if (_vm!.RefreshAllCommand.CanExecute(null)) _vm.RefreshAllCommand.Execute(null);
        });
        menu.Items.Add(refreshItem);
        menu.Opened += (_, _) =>
        {
            importItem.IsEnabled = _vm!.ImportMineCommand.CanExecute(null); // пункты сереют, пока идёт своё
            refreshItem.IsEnabled = _vm.RefreshAllCommand.CanExecute(null);
        };
        menu.Items.Add(new Separator());

        var autostart = new MenuItem { Header = "Запускать вместе с Windows", IsCheckable = true, IsChecked = AutostartService.IsEnabled() };
        autostart.Click += (_, _) => AutostartService.Set(autostart.IsChecked);
        menu.Items.Add(autostart);

        menu.Items.Add(MenuItemFor("Открыть папку с данными", () =>
            System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo(DataDir) { UseShellExecute = true })));
        menu.Items.Add(MenuItemFor("Настройки…", ShowSettings));
        menu.Items.Add(new Separator());
        menu.Items.Add(MenuItemFor("Выход", Shutdown));

        _tray = new TaskbarIcon
        {
            ContextMenu = menu,
            NoLeftClickDelay = true,
            LeftClickCommand = new RelayCommand(() => _main!.ShowAndActivate()),
        };
        _tray.TrayBalloonTipClicked += (_, _) =>
        {
            var action = _onNotificationClick;
            _onNotificationClick = null;   // одно уведомление — одно действие
            action?.Invoke();
        };
        // добавление, перенос (в т.ч. drag&drop) и удаление меняют Items колонок — бейдж следом
        _vm!.ColumnFor(TicketStatus.Inbox).Items.CollectionChanged += (_, _) => QueueTrayUpdate();
        _vm.ColumnFor(TicketStatus.InProgress).Items.CollectionChanged += (_, _) => QueueTrayUpdate();
        UpdateTrayIcon();
        _tray.ForceCreate();
    }

    /// <summary>
    /// Монохромный глиф под цвет панели задач (тёмный на светлой теме, белый на тёмной) + бейдж с числом «Входящих»
    /// и точка при перегрузе «В работе». Считаем все карточки — поиск и фильтры трей не трогают.
    /// </summary>
    private bool _trayUpdateQueued;

    /// <summary>Перерисовка значка дорогая (иконка → битмап → HICON → Shell_NotifyIcon), а импорт добавляет заявки
    /// по одной. Склеиваем пачку изменений в одну перерисовку.</summary>
    private void QueueTrayUpdate()
    {
        if (_trayUpdateQueued) return;
        _trayUpdateQueued = true;
        Dispatcher.BeginInvoke(() => { _trayUpdateQueued = false; UpdateTrayIcon(); }, DispatcherPriority.Background);
    }

    private void UpdateTrayIcon()
    {
        if (_tray is null || _vm is null) return;
        var dark = ApplicationThemeManager.GetSystemTheme() is SystemTheme.Dark or SystemTheme.HCBlack or SystemTheme.HC1 or SystemTheme.HC2;
        var inbox = _vm.ColumnFor(TicketStatus.Inbox).Items.Count;
        var overloaded = _vm.ColumnFor(TicketStatus.InProgress).IsOverloadedTotal;
        // IconSource у H.NotifyIcon понимает только BitmapImage/BitmapFrame из файла, поэтому отдаём готовую Icon.
        // Предыдущую TaskbarIcon диспоузит сам (OnIconChanged).
        _tray.Icon = RenderTrayIcon(dark ? "tray-dark.ico" : "tray-light.ico", inbox, overloaded);
        _tray.ToolTipText = (inbox > 0 ? $"Заявки — входящих: {inbox}" : "Заявки — входящих нет")
            + (overloaded ? "\n«В работе» больше лимита" : "");
    }

    /// <summary>
    /// Размеры из макета заданы для 16 px, рисуем в реальном размере иконки трея (20–32 px при 125–200 %).
    /// Бейдж — в правом нижнем углу, чтобы точка перегруза в правом верхнем не закрывала цифру.
    /// </summary>
    private System.Drawing.Icon RenderTrayIcon(string glyph, int count, bool overloaded)
    {
        var px = Math.Max(16, IconUtilities.GetRequiredCustomIconSize(false).Width);
        var k = px / 16.0;
        var frames = BitmapDecoder.Create(new Uri($"pack://application:,,,/Assets/{glyph}"),
            BitmapCreateOptions.None, BitmapCacheOption.OnLoad).Frames;
        var baseFrame = frames.OrderBy(f => f.PixelWidth).FirstOrDefault(f => f.PixelWidth >= px) ?? frames.MaxBy(f => f.PixelWidth)!;

        var full = new Rect(0, 0, px, px);
        var dotR = 3.5 * k;
        var dotC = new Point(px - dotR, dotR);
        var visual = new DrawingVisual();
        RenderOptions.SetBitmapScalingMode(visual, BitmapScalingMode.HighQuality);
        using (var dc = visual.RenderOpen())
        {
            // вокруг точки — прозрачный зазор 1 px, чтобы она не сливалась с глифом и бейджем
            if (overloaded)
                dc.PushClip(Geometry.Combine(new RectangleGeometry(full), new EllipseGeometry(dotC, dotR + k, dotR + k), GeometryCombineMode.Exclude, null));
            dc.DrawImage(baseFrame, full);
            if (count > 0)
            {
                var onAccent = (Brush)FindResource("OnAccent");
                var text = new FormattedText(count > 9 ? "9+" : count.ToString(), CultureInfo.InvariantCulture, FlowDirection.LeftToRight,
                    new Typeface((FontFamily)FindResource("FontUi"), FontStyles.Normal, FontWeights.SemiBold, FontStretches.Normal),
                    9 * k, onAccent, 1.0).BuildGeometry(new Point());
                var ink = text.Bounds;
                var h = Math.Round(12 * k);
                var w = Math.Min(px, Math.Round(Math.Max(h, ink.Width + 4 * k)));
                var badge = new Rect(px - w, px - h, w, h);
                dc.DrawRoundedRectangle((Brush)FindResource("Accent"), null, badge, h / 2, h / 2);
                dc.PushTransform(new TranslateTransform(badge.X + (w - ink.Width) / 2 - ink.X, badge.Y + (h - ink.Height) / 2 - ink.Y));
                dc.DrawGeometry(onAccent, null, text);
                dc.Pop();
            }
            if (overloaded)
            {
                dc.Pop();
                dc.DrawEllipse((Brush)FindResource("CautionFg"), null, dotC, dotR, dotR);
            }
        }

        var bmp = new RenderTargetBitmap(px, px, 96, 96, PixelFormats.Pbgra32);
        bmp.Render(visual);
        using var ico = bmp.ToStream();                // H.NotifyIcon: PNG, завёрнутый в .ico
        return new System.Drawing.Icon(ico, px, px);   // своя HICON — Dispose её освобождает
    }

    private static void ShowError(Exception ex)
    {
        LogError(ex);
        MessageBox.Show(ex.ToString(), "Заявки — ошибка", MessageBoxButton.OK, MessageBoxImage.Error);
    }

    /// <summary>Данные лежат рядом с exe, значит папка должна быть доступна на запись. Без этого сохранение падало бы
    /// раз в полсекунды, и даже errors.log некуда было бы писать — честнее сказать сразу и не запускаться.</summary>
    private static bool CanWriteToDataDir()
    {
        try
        {
            Directory.CreateDirectory(DataDir);
            var probe = Path.Combine(DataDir, ".write-test");
            File.WriteAllText(probe, "");
            File.Delete(probe);
            return true;
        }
        catch { return false; }
    }

    /// <summary>errors.log рядом с exe — его можно прислать, чтобы разобраться с ошибкой.</summary>
    private static void LogError(Exception? ex) => AppendLog($"{ex}");

    private static readonly object LogLock = new();

    /// <summary>Пишут сюда и UI-поток, и пул (неразобранные ответы идут из параллельных запросов): без блокировки второй
    /// писатель получает sharing violation, catch его глотает — и запись пропадает молча.</summary>
    private static void AppendLog(string text)
    {
        // ponytail: без ротации — ошибки редкие; вырастет — обрезать при старте
        lock (LogLock)
        {
            try { File.AppendAllText(Path.Combine(DataDir, "errors.log"), $"[{DateTime.Now:yyyy-MM-dd HH:mm:ss}] {text}\n\n"); }
            catch { /* лог не должен ронять приложение */ }
        }
    }

    private static MenuItem MenuItemFor(string header, Action action)
    {
        var item = new MenuItem { Header = header };
        item.Click += (_, _) => action();
        return item;
    }

    private void SetupHotkey(AppSettings settings)
    {
        _hotkeys = new HotkeyService();
        _hotkeys.Pressed += () => _capture!.ToggleCapture();
        RegisterHotkey(settings);
    }

    private void RegisterHotkey(AppSettings settings)
    {
        if (!_hotkeys!.TryRegister(settings.Hotkey, out var error))
            ShowTrayNotification("Хоткей не работает", error + "\nПоменяй хоткей в настройках", NotificationIcon.Warning);
    }

    private void ShowSettings()
    {
        if (_settingsWindow is null)
        {
            _settingsWindow = new SettingsWindow(_settings!);
            _settingsWindow.Saved += ApplySettings;
            _settingsWindow.Closed += (_, _) => _settingsWindow = null;
        }
        _settingsWindow.Show();
        _settingsWindow.Activate();
    }

    /// <summary>Настройки сохранены — применяем без перезапуска: пороги и лимит, клиент API, хоткей. Регулярку парсер читает сам.</summary>
    private void ApplySettings()
    {
        var settings = _settings!;
        var intraservice = _intraservice = HttpIntraserviceClient.From(settings);
        _vm!.ApplySettings(intraservice);
        _captureVm!.ApplySettings(intraservice);
        _searchVm!.ApplySettings(intraservice);
        _captureItem!.Header = $"Быстрое добавление\t{settings.Hotkey}";
        UpdateTrayIcon(); // точка перегруза — лимит мог поменяться
        RegisterHotkey(settings);
        WarnIfInsecure(settings, intraservice);
        ApplyRelay(settings);
    }

    /// <summary>Ответ Claude через буфер (Services/ClaudeRelay.cs): включён — слушаем буфер обмена, выключен — в него
    /// не заглядываем вовсе.</summary>
    private void ApplyRelay(AppSettings settings)
    {
        if (settings.ClaudeRelayEnabled == (_clipboard is not null)) return;
        if (_clipboard is not null)
        {
            _clipboard.Dispose();
            _clipboard = null;
            return;
        }
        try
        {
            _clipboard = new ClipboardWatcher();
            _clipboard.Changed += OnClipboardChanged;
        }
        catch (System.ComponentModel.Win32Exception ex)
        {
            LogError(ex);
            ShowTrayNotification("Ответы Claude через буфер не работают", ex.Message, NotificationIcon.Warning);
        }
    }

    /// <summary>В буфере блок запросов Claude («TB …») — выполняем и кладём ответ туда же. Всё прочее в буфере не трогаем
    /// и нигде не храним; Office, файлы и данные менеджеров паролей даже не читаем (ClipboardWatcher.HasPlainText).</summary>
    private async void OnClipboardChanged()
    {
        if (_relayBusy) { _relayAgain = true; return; }   // посмотрим буфер ещё раз, когда соберём текущий ответ

        var watcher = _clipboard;
        var sequence = ClipboardWatcher.SequenceNumber;
        IReadOnlyList<RelayRequest>? requests;
        try { requests = ClipboardWatcher.HasPlainText && ClipboardWatcher.TryGetText() is { } text ? ClaudeRelay.Parse(text) : null; }
        catch (Exception ex) { LogError(ex); return; }   // чужое содержимое буфера не должно ронять приложение
        if (requests is null) return;

        _relayBusy = true;
        try
        {
            var answer = await ClaudeRelay.RunAsync(requests, _intraservice, BoardSnapshot, _settings!, _vm!.HideOldDone);
            if (_clipboard is null || _clipboard != watcher) return;   // галочку сняли, пока собирали, — буфер не трогаем
            if (ClipboardWatcher.SequenceNumber != sequence)
            {
                // пока собирали, в буфер положили другое — его не затираем. Новый блок «TB …» разберёт повторный заход
                var newBlock = ClipboardWatcher.HasPlainText && ClipboardWatcher.TryGetText() is { } now && ClaudeRelay.Parse(now) is not null;
                if (!newBlock)
                    ShowTrayNotification("Ответ для Claude не вставлен в буфер",
                        "Пока он собирался, туда скопировали другое. Нажмите «Copy» на блоке ещё раз", NotificationIcon.Warning);
                return;
            }
            if (ClipboardWatcher.TrySetText(answer))
                ShowTrayNotification("Ответ для Claude — в буфере",
                    $"Запросов: {Math.Min(requests.Count, ClaudeRelay.MaxRequests)}. Вставьте в чат: Ctrl+V", NotificationIcon.Info);
            else
                ShowTrayNotification("Ответ для Claude не попал в буфер",
                    "Буфер занят другой программой — нажмите «Copy» на блоке ещё раз", NotificationIcon.Warning);
        }
        catch (Exception ex)
        {
            LogError(ex);
            ShowTrayNotification("Ответ для Claude не собрался", ex.Message, NotificationIcon.Error);
        }
        finally
        {
            _relayBusy = false;
            if (_relayAgain)
            {
                _relayAgain = false;
                OnClipboardChanged();
            }
        }
    }

    /// <summary>Снимок доски для ответа Claude (id — только карточки с этим номером). Коллекции карточек живут в UI-потоке —
    /// собираем и фильтруем там, отдаём копию.</summary>
    private Task<IReadOnlyList<BoardCard>> BoardSnapshot(int? id) => Dispatcher.InvokeAsync(() => (IReadOnlyList<BoardCard>)_vm!.Columns
        .SelectMany(c => c.Items.Where(t => id is null || t.IntraserviceId == id).Select(t => new BoardCard(t.IntraserviceId, t.Title,
            c.Title, PriorityToTextConverter.Text(t.Priority), t.ExternalStatus, t.DaysInStatus, t.CompletedAt, t.Url, t.Description,
            t.Notes.Select(n => new BoardNote(n.CreatedAt, n.Text)).ToList(), t.Creator, t.Executors, t.ExecutorGroup)))
        .ToList()).Task;

    /// <summary>Уведомление в трее. onClick — что сделать по щелчку на нём (Windows сообщает о щелчке, пока уведомление
    /// на экране); у каждого нового уведомления своё действие, прошлое забывается. Длину режем сами: у Windows под
    /// заголовок 63 символа, под текст 255, а H.NotifyIcon на строке длиннее бросает исключение.</summary>
    private void ShowTrayNotification(string title, string text, NotificationIcon icon, Action? onClick = null)
    {
        static string Cut(string s, int max) => s.Length <= max ? s : s[..(max - 1)] + "…";
        try
        {
            _tray?.ShowNotification(Cut(title, 63), Cut(text, 250), icon);
            _onNotificationClick = onClick;   // не показалось — на экране прошлое уведомление, и щелчок по нему — его
        }
        catch (Exception ex) { LogError(ex); }   // уведомление — не повод ронять таймер или приложение
    }

    /// <summary>У API только базовая авторизация: по http пароль уходит открытым текстом.</summary>
    private void WarnIfInsecure(AppSettings settings, HttpIntraserviceClient? client)
    {
        if (client is not null && HttpIntraserviceClient.IsHttp(settings.IntraserviceBaseUrl))
            ShowTrayNotification("Интрасервис по http", "Пароль передаётся открытым текстом. Лучше адрес https://", NotificationIcon.Warning);
    }

    protected override void OnExit(ExitEventArgs e)
    {
        _vm?.SaveNow();
        _clipboard?.Dispose();
        _hotkeys?.Dispose();
        _tray?.Dispose();
        if (_ownsMutex) _mutex?.ReleaseMutex();
        base.OnExit(e);
    }
}
