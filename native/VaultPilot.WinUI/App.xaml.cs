using H.NotifyIcon;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media.Imaging;
using System.Diagnostics;

namespace VaultPilot.WinUI;

public partial class App : Application
{
    private MainWindow? _window;
    private TaskbarIcon? _trayIcon;
    private bool _isExiting;
    private Mutex? _instanceMutex;

    public App()
    {
        InitializeComponent();
        UnhandledException += OnUnhandledException;
        TaskScheduler.UnobservedTaskException += OnUnobservedTaskException;
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
        _instanceMutex = new Mutex(true, "VaultPilot-SingleInstance", out bool isNewInstance);
        if (!isNewInstance)
        {
            _instanceMutex.Dispose();
            _instanceMutex = null;
            Exit();
            return;
        }

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
        _window?.SignalStopping();
        _isExiting = true;
    }

    private async void ExitApplication()
    {
        _isExiting = true;

        try
        {
            if (_window != null)
            {
                _window.Closed -= OnWindowClosed;
                await _window.ShutdownAsync();
                _window.Close();
                _window = null;
            }
        }
        catch (Exception ex)
        {
            Debug.WriteLine($"[VaultPilot] ExitApplication error: {ex}");
            LogToFile("EXIT", ex);
        }
        finally
        {
            _trayIcon?.Dispose();
            _instanceMutex?.ReleaseMutex();
            _instanceMutex?.Dispose();
            Application.Current.Exit();
        }
    }

    private void OnUnhandledException(object sender, Microsoft.UI.Xaml.UnhandledExceptionEventArgs e)
    {
        e.Handled = true;
        var ex = e.Exception;
        Debug.WriteLine($"[VaultPilot] UnhandledException: {ex}");
        LogToFile("UNHANDLED", ex);
    }

    private void OnUnobservedTaskException(object? sender, UnobservedTaskExceptionEventArgs e)
    {
        e.SetObserved();
        Debug.WriteLine($"[VaultPilot] UnobservedTaskException: {e.Exception}");
        LogToFile("UNOBSERVED", e.Exception);
    }

    private static void LogToFile(string kind, Exception? ex)
    {
        try
        {
            var logDir = Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                "com.local.vaultpilot", "logs");
            Directory.CreateDirectory(logDir);
            var logPath = Path.Combine(logDir, "crash.log");
            var entry = $"[{DateTimeOffset.Now:O}] {kind}: {ex}\n";
            File.AppendAllText(logPath, entry);
        }
        catch
        {
            // If we can't log, there's nothing more we can do
        }
    }
}
