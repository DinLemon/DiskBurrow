using System.Windows.Threading;
namespace DiskBurrow.App.Services;
public interface IUiDispatcher { Task InvokeAsync(Action action); }
public sealed class WpfDispatcher(Dispatcher dispatcher) : IUiDispatcher
{
    public Task InvokeAsync(Action action) => dispatcher.CheckAccess() ? InvokeHere(action) : dispatcher.InvokeAsync(action).Task;
    private static Task InvokeHere(Action action) { action(); return Task.CompletedTask; }
}
