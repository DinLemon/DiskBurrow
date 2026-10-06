using System.Windows.Threading;
namespace DiskBurrow.App.Services;
public static class BoundedDispatcherDrain
{
    public static bool Wait(Task shutdown,Dispatcher dispatcher,TimeSpan limit)
    {
        if(!dispatcher.CheckAccess()||limit<=TimeSpan.Zero)throw new ArgumentException("A positive bound on the owning Dispatcher is required.");
        if(!shutdown.IsCompleted&&!dispatcher.HasShutdownStarted)
        {
            var frame=new DispatcherFrame();var timer=new DispatcherTimer(DispatcherPriority.Send,dispatcher){Interval=limit};
            timer.Tick+=(_,_)=>frame.Continue=false;
            _ = shutdown.ContinueWith(_=>frame.Continue=false,CancellationToken.None,TaskContinuationOptions.ExecuteSynchronously,TaskScheduler.Default);
            timer.Start();try{if(!shutdown.IsCompleted)Dispatcher.PushFrame(frame);}finally{timer.Stop();}
        }
        if(shutdown.IsFaulted)_ = shutdown.Exception;
        return shutdown.IsCompletedSuccessfully;
    }
}
