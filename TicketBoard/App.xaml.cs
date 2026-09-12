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
        var settings = AppSettings.Load(DataDir);
        var parser = new IntraserviceLinkParser(settings);
        _vm = new MainViewModel(new TicketStore(DataDir), settings, parser);

        // Тема: WPF-UI следит за системой, мы подкладываем свои токены и иконку трея под неё.
        ApplicationThemeManager.Changed += (theme, _) =>
        {
            TokenTheme.Apply(theme);
            UpdateTrayIcon();
        };
        ApplicationThemeManager.ApplySystemTheme();
        TokenTheme.Apply(ApplicationThemeManager.GetAppTheme());

        _main = new MainWindow(_vm);
        _capture = new QuickCaptureWindow(_vm, new QuickCaptureViewModel(parser, settings), parser);
        _vm.CaptureRequested += () => _capture.ShowCapture();

        SetupTray(settings);
        SetupHotkey(settings);

        if (!e.Args.Contains("--minimized"))
            _main.ShowAndActivate();
    }

    private void SetupTray(AppSettings settings)
    {
        var menu = new ContextMenu();
        menu.Items.Add(MenuItemFor("Открыть", () => _main!.ShowAndActivate()));
        menu.Items.Add(MenuItemFor($"Быстрое добавление\t{settings.Hotkey}", () => _capture!.ShowCapture()));
        menu.Items.Add(new Separator());

        var autostart = new MenuItem { Header = "Запускать вместе с Windows", IsCheckable = true, IsChecked = AutostartService.IsEnabled() };
        autostart.Click += (_, _) => AutostartService.Set(autostart.IsChecked);
        menu.Items.Add(autostart);

        menu.Items.Add(MenuItemFor("Открыть папку с данными", () =>
            System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo(DataDir) { UseShellExecute = true })));
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
        if (!_hotkeys.TryRegister(settings.Hotkey, out var error))
            _tray?.ShowNotification("Хоткей не работает", error + "\nПоменяй Hotkey в settings.json", NotificationIcon.Warning);
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
