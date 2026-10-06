using System.ComponentModel;
using System.Runtime.CompilerServices;
namespace DiskBurrow.App.ViewModels;
public abstract class ObservableModel : INotifyPropertyChanged
{
    public event PropertyChangedEventHandler? PropertyChanged;
    protected void Changed([CallerMemberName] string? name = null) => PropertyChanged?.Invoke(this,new(name));
    protected void Refresh() => Changed("");
}
