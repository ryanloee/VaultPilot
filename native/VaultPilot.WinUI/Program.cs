using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using System.Threading;
using Velopack;

namespace VaultPilot.WinUI;

public static class Program
{
    [STAThread]
    private static void Main(string[] args)
    {
        WinRT.ComWrappersSupport.InitializeComWrappers();
        VelopackApp.Build().SetArgs(args).Run();

        Application.Start(_ =>
        {
            var context = new DispatcherQueueSynchronizationContext(DispatcherQueue.GetForCurrentThread());
            SynchronizationContext.SetSynchronizationContext(context);
            new App();
        });
    }
}
