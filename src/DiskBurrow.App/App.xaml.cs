using System.Windows;
using DiskBurrow.App.Services;
using DiskBurrow.Core.Monitoring;
namespace DiskBurrow.App;
public partial class App : Application
{
    private AppRuntime? runtime;
    private SingleInstanceService? instance;
    private MainWindow? window;
    private TrayService? tray;
    private bool exiting;
    private Task? shutdownTask;
    protected override async void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);
        try
        {
            if(e.Args.Length>0)
            {
                if(e.Args.Length!=2||e.Args[0]!="--verify-runtime"){Shutdown(2);return;}
                Shutdown(await RuntimeVerifier.RunAsync(e.Args[1],Dispatcher));return;
            }
            instance=new("DiskBurrow",()=>Dispatcher.InvokeAsync(()=>window?.ActivateWindow()));
            if(!await instance.TryBecomePrimaryAsync()){await instance.DisposeAsync();instance=null;Shutdown();return;}
            var data=Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"DiskBurrow");
            runtime=await AppRuntime.CreateAsync(data,Dispatcher);
            window=new(runtime.Model);MainWindow=window;
            tray=new(runtime.Locale,alert=>window.ActivateWindow(alert),()=>runtime.Model.Observe(runtime.Model.ScanAsync(new DiskBurrow.Windows.System.WindowsEnvironment().SystemRoot)),()=>runtime.Model.TogglePause(),()=>_ = RequestExitAsync());
            var model=runtime.Model;
            runtime.Model.StateChanged+=()=>tray?.SetState(model.Busy,model.Paused);
            runtime.Coordinator.AlertsRaised+=AlertsRaised;
            tray.SetState(runtime.Model.Busy,runtime.Model.Paused);window.Show();runtime.Start();
        }
        catch(Exception error)
        {
            if(e.Args.Length==0)MessageBox.Show(LocalizationService.ReadText("ru","Status.Error")+Environment.NewLine+error.Message,"DiskBurrow",MessageBoxButton.OK,MessageBoxImage.Error);
            await DisposeAsync();Shutdown(2);
        }
    }
    private void AlertsRaised(IReadOnlyList<AlertEvent> alerts)=>Dispatcher.InvokeAsync(()=>{foreach(var alert in alerts)tray?.Notify(alert);});
    private async Task RequestExitAsync()
    {
        if(exiting)return;
        try{await BeginShutdown();Shutdown();}catch(Exception){Shutdown(2);}
    }
    private Task BeginShutdown()
    {
        exiting=true;if(window is not null)window.IsEnabled=false;runtime?.Model.Cancel();
        return shutdownTask??=DisposeAsync();
    }
    private async Task DisposeAsync()
    {
        tray?.Dispose();tray=null;
        if(runtime is not null){runtime.Coordinator.AlertsRaised-=AlertsRaised;await runtime.DisposeAsync();runtime=null;}
        if(instance is not null){await instance.DisposeAsync();instance=null;}
        if(window is not null){window.AllowClose=true;window.Close();window=null;}
    }
    protected override void OnSessionEnding(SessionEndingCancelEventArgs e)
    {
        // Let Dispatcher continuations finish partial reports without vetoing Windows shutdown.
        BoundedDispatcherDrain.Wait(BeginShutdown(),Dispatcher,TimeSpan.FromSeconds(2));
        base.OnSessionEnding(e);
    }
    protected override void OnExit(ExitEventArgs e)
    {
        var drain=BeginShutdown();if(drain.IsFaulted)_ = drain.Exception;
        // Session termination can exceed the bounded drain; never block an unpumped Dispatcher here.
        tray?.Dispose();tray=null;
        base.OnExit(e);
    }
}
