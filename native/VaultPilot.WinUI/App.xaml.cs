using H.NotifyIcon;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media.Imaging;

namespace VaultPilot.WinUI;

public partial class App : Application
{
    private MainWindow? _window;
    private TaskbarIcon? _trayIcon;
    private bool _isExiting;

    public App()
    {
        InitializeComponent();
    }

    public void ShowMainWindow()
    {
        if (_window == null)
        {
            _window = new MainWindow();
            _window.Closed += OnWindowClosed;
        }

        _window.ShowAndActivate();
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        InitializeTrayIcon();
        if (_window == null)
        {
            _window = new MainWindow();
            _window.Closed += OnWindowClosed;
        }
        _window.Activate();
    }

    private void InitializeTrayIcon()
    {
        _trayIcon = new TaskbarIcon
        {
            IconSource = new BitmapImage(new Uri("ms-appx:///icon.ico")),
            ToolTipText = "VaultPilot",
            ContextMenuMode = ContextMenuMode.SecondWindow,
        };

        var showItem = new MenuFlyoutItem { Text = "显示窗口" };
        showItem.Click += (_, _) => ShowMainWindow();

        var exitItem = new MenuFlyoutItem { Text = "退出" };
        exitItem.Click += (_, _) => ExitApplication();

        _trayIcon.ContextFlyout = new MenuFlyout
        {
            Items = { showItem, new MenuFlyoutSeparator(), exitItem },
        };

        _trayIcon.ForceCreate();
    }

    private void OnWindowClosed(object sender, WindowEventArgs args)
    {
        if (_isExiting)
        {
            return;
        }

        args.Handled = true;
        (sender as MainWindow)?.Hide();
    }

    public void BeginExitForUpdate()
    {
        _isExiting = true;
    }

    private async void ExitApplication()
    {
        _isExiting = true;

        if (_window != null)
        {
            _window.Closed -= OnWindowClosed;
            await _window.PrepareExitAsync();
            _window.Close();
            _window = null;
        }

        _trayIcon?.Dispose();
        Environment.Exit(0);
    }
}
