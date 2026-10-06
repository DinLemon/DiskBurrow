using System.Windows.Input;
namespace DiskBurrow.App.Services;
public sealed class ActionCommand(Action execute,Func<bool>? canExecute=null) : ICommand
{
    public bool CanExecute(object? parameter)=>canExecute?.Invoke()??true;
    public void Execute(object? parameter){if(CanExecute(parameter))execute();}
    public event EventHandler? CanExecuteChanged;
    public void Refresh()=>CanExecuteChanged?.Invoke(this,EventArgs.Empty);
}
