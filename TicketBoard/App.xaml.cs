using System.IO;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using CommunityToolkit.Mvvm.Input;
using H.NotifyIcon;
using H.NotifyIcon.Core;
using TicketBoard.Services;
using TicketBoard.ViewModels;
using TicketBoard.Views;
using Wpf.Ui.Appearance;

namespace TicketBoard;

public partial class App : Application
{
    public static string DataDir { get; } =
        Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "TicketBoard");

    private Mutex? _mutex;
    private bool _ownsMutex;
    private HotkeyService? _hotkeys;
    private TaskbarIcon? _tray;
    private MainViewModel? _vm;
    private MainWindow? _main;
    private QuickCaptureWindow? _capture;
    private QuickCaptureViewModel? _captureVm;
    private SettingsWindow? _settingsWindow;
    private MenuItem? _captureItem;
    private AppSettings? _settings;

    protected override void OnStartup(StartupEventArgs e)
    {
        _mutex = new Mutex(true, @"Local\TicketBoard.SingleInstance", out _ownsMutex);
        if (!_ownsMutex)
        {
            Shutdown();
            return;
        }

        base.OnStartup(e);
        DispatcherUnhandledException += (_, ex) =>
        {
            MessageBox.Show(ex.Exception.ToString(), "Заявки — ошибка", MessageBoxButton.OK, MessageBoxImage.Error);
            ex.Handled = true;
        };

        Directory.CreateDirectory(DataDir);
        var settings = _settings = AppSettings.Load(DataDir);
        var parser = new IntraserviceLinkParser(settings);
        var intraservice = HttpIntraserviceClient.From(settings);
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
        _vm.CaptureRequested += () => _capture.ShowCapture();
        _vm.SettingsRequested += ShowSettings;

        SetupTray(settings);
        SetupHotkey(settings);
        WarnIfInsecure(settings, intraservice);

        if (!e.Args.Contains("--minimized"))
            _main.ShowAndActivate();
    }

    private void SetupTray(AppSettings settings)
    {
        var menu = new ContextMenu();
        menu.Items.Add(MenuItemFor("Открыть", () => _main!.ShowAndActivate()));
        _captureItem = MenuItemFor($"Быстрое добавление\t{settings.Hotkey}", () => _capture!.ShowCapture());
        menu.Items.Add(_captureItem);
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
            ToolTipText = "Заявки",
            ContextMenu = menu,
            NoLeftClickDelay = true,
            LeftClickCommand = new RelayCommand(() => _main!.ShowAndActivate()),
        };
        UpdateTrayIcon();
        _tray.ForceCreate();
    }

    /// <summary>Монохромный глиф под цвет панели задач: тёмный на светлой теме, белый на тёмной.</summary>
    private void UpdateTrayIcon()
    {
        if (_tray is null) return;
        var dark = ApplicationThemeManager.GetSystemTheme() is SystemTheme.Dark or SystemTheme.HCBlack or SystemTheme.HC1 or SystemTheme.HC2;
        var name = dark ? "tray-dark.ico" : "tray-light.ico";
        _tray.IconSource = new BitmapImage(new Uri($"pack://application:,,,/Assets/{name}"));
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
        var intraservice = HttpIntraserviceClient.From(settings);
        _vm!.ApplySettings(intraservice);
        _captureVm!.ApplySettings(intraservice);
        _captureItem!.Header = $"Быстрое добавление\t{settings.Hotkey}";
        RegisterHotkey(settings);
        WarnIfInsecure(settings, intraservice);
    }

    /// <summary>У API только базовая авторизация: по http пароль уходит открытым текстом.</summary>
    private void WarnIfInsecure(AppSettings settings, IIntraserviceClient client)
    {
        if (client is HttpIntraserviceClient && HttpIntraserviceClient.IsHttp(settings.IntraserviceBaseUrl))
            _tray?.ShowNotification("Интрасервис по http", "Пароль передаётся открытым текстом. Лучше адрес https://", NotificationIcon.Warning);
    }

    protected override void OnExit(ExitEventArgs e)
    {
        _vm?.SaveNow();
        _hotkeys?.Dispose();
        _tray?.Dispose();
        if (_ownsMutex) _mutex?.ReleaseMutex();
        base.OnExit(e);
    }
}
