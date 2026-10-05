using Microsoft.UI.Xaml;

namespace BenchWinUI;

public partial class App : Application
{
    private Window? _window;

    public App()
    {
        InitializeComponent();
        UnhandledException += (_, e) =>
            System.IO.File.AppendAllText("C:\\bench_winui3_err.log",
                $"UNHANDLED: {e.Exception}\n");
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        try
        {
            _window = new MainWindow();
            _window.AppWindow.Resize(new Windows.Graphics.SizeInt32(1280, 800));
            _window.Activate();
        }
        catch (System.Exception ex)
        {
            System.IO.File.AppendAllText("C:\\bench_winui3_err.log",
                $"LAUNCH FAIL: {ex}\n");
            throw;
        }
    }
}
