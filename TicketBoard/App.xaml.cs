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
    private HttpIntraserviceClient? _intraservice;   // текущий клиент API — его же берёт мост для Claude
    private ClaudeBridge? _bridge;
    private (bool Enabled, int Port, string Key) _bridgeState;

    /// <summary>Что сказать о мосте для Claude в окне настроек; пусто — выключен.</summary>
    public static string BridgeStatus { get; private set; } = "";

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
        ClaudeBridge.Log = AppendLog;
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
        var settings = _settings = AppSettings.Load(DataDir);
        var parser = new IntraserviceLinkParser(settings);
        var intraservice = _intraservice = HttpIntraserviceClient.From(settings);
        _vm = new MainViewModel(new TicketStore(DataDir), settings, parser, intraservice);

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
        _vm.SettingsRequested += ShowSettings;
        _main.ServerSearchRequested += text => _search.ShowSearch(text);

        SetupTray(settings);
        SetupHotkey(settings);
        WarnIfInsecure(settings, intraservice);
        ApplyBridge(settings);

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
            _tray?.ShowNotification("Хоткей не работает", error + "\nПоменяй хоткей в настройках", NotificationIcon.Warning);
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
        ApplyBridge(settings);
    }

    /// <summary>Мост для Claude (Services/ClaudeBridge.cs): включён — слушает 127.0.0.1:порт. Перезапускается, только если
    /// поменялись флаг, порт или ключ; клиент API и доску мост берёт на каждый запрос сам.</summary>
    private void ApplyBridge(AppSettings settings)
    {
        if (settings.ClaudeBridgeEnabled && settings.ClaudeBridgeKey.Length == 0)
        {
            // включили руками в settings.json или ключ не расшифровался на этой машине — нужен новый
            settings.ClaudeBridgeKey = ClaudeBridge.NewKey();
            try { settings.Save(DataDir); } catch (Exception ex) { LogError(ex); }
        }
        var state = (settings.ClaudeBridgeEnabled, settings.ClaudeBridgePort, settings.ClaudeBridgeKey);
        if (state == _bridgeState && (_bridge is not null || !state.ClaudeBridgeEnabled)) return;
        _bridgeState = state;
        _bridge?.Dispose();
        _bridge = null;
        BridgeStatus = "";
        if (!settings.ClaudeBridgeEnabled) return;
        if (settings.ClaudeBridgePort is < 1 or > 65535)   // 0 TcpListener понял бы как «любой свободный» — ссылка бы не сошлась
        {
            BridgeStatus = $"Не запустился: порт {settings.ClaudeBridgePort} — нужен от 1 до 65535 (ClaudeBridgePort в settings.json).";
            _tray?.ShowNotification("Мост для Claude не запустился", BridgeStatus, NotificationIcon.Warning);
            return;
        }

        try
        {
            var bridge = new ClaudeBridge(settings.ClaudeBridgePort, settings.ClaudeBridgeKey, settings, () => _intraservice,
                BoardSnapshot, typeof(App).Assembly.GetName().Version?.ToString(3) ?? "");
            bridge.Start();
            _bridge = bridge;
            BridgeStatus = $"Работает на порту {bridge.Port}.";
        }
        catch (Exception ex) when (ex is System.Net.Sockets.SocketException or ArgumentOutOfRangeException)
        {
            BridgeStatus = $"Не запустился на порту {settings.ClaudeBridgePort}: {ex.Message}. Порт — ClaudeBridgePort в settings.json.";
            _tray?.ShowNotification("Мост для Claude не запустился", BridgeStatus, NotificationIcon.Warning);
        }
    }

    /// <summary>Снимок доски для моста. Коллекции карточек живут в UI-потоке — собираем там, отдаём копию.</summary>
    private Task<IReadOnlyList<BridgeCard>> BoardSnapshot() => Dispatcher.InvokeAsync(() => (IReadOnlyList<BridgeCard>)_vm!.Columns
        .SelectMany(c => c.Items.Select(t => new BridgeCard(t.IntraserviceId, t.Title, c.Title, PriorityName(t.Priority),
            t.ExternalStatus, t.DaysInStatus, t.Url, t.Description, t.Notes.Select(n => new BridgeNote(n.CreatedAt, n.Text)).ToList())))
        .ToList()).Task;

    private static string PriorityName(TicketPriority p) => p switch
    {
        TicketPriority.High => "высокий",
        TicketPriority.Low => "низкий",
        _ => "средний",
    };

    /// <summary>У API только базовая авторизация: по http пароль уходит открытым текстом.</summary>
    private void WarnIfInsecure(AppSettings settings, HttpIntraserviceClient? client)
    {
        if (client is not null && HttpIntraserviceClient.IsHttp(settings.IntraserviceBaseUrl))
            _tray?.ShowNotification("Интрасервис по http", "Пароль передаётся открытым текстом. Лучше адрес https://", NotificationIcon.Warning);
    }

    protected override void OnExit(ExitEventArgs e)
    {
        _vm?.SaveNow();
        _bridge?.Dispose();
        _hotkeys?.Dispose();
        _tray?.Dispose();
        if (_ownsMutex) _mutex?.ReleaseMutex();
        base.OnExit(e);
    }
}
