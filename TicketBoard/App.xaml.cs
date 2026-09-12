using System.Globalization;
using System.IO;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
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
            ContextMenu = menu,
            NoLeftClickDelay = true,
            LeftClickCommand = new RelayCommand(() => _main!.ShowAndActivate()),
        };
        // добавление, перенос (в т.ч. drag&drop) и удаление меняют Items колонок — бейдж следом
        _vm!.ColumnFor(TicketStatus.Inbox).Items.CollectionChanged += (_, _) => UpdateTrayIcon();
        _vm.ColumnFor(TicketStatus.InProgress).Items.CollectionChanged += (_, _) => UpdateTrayIcon();
        UpdateTrayIcon();
        _tray.ForceCreate();
    }

    /// <summary>
    /// Монохромный глиф под цвет панели задач (тёмный на светлой теме, белый на тёмной) + бейдж с числом «Входящих»
    /// и точка при перегрузе «В работе». Считаем все карточки — поиск и фильтры трей не трогают.
    /// </summary>
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
