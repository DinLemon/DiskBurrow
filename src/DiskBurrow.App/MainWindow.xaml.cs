using System.ComponentModel;
using System.Windows;
using System.Windows.Controls;
using DiskBurrow.App.ViewModels;
using DiskBurrow.Core.Scanning;
using DiskBurrow.Core.Monitoring;
using DiskBurrow.App.Services;
namespace DiskBurrow.App;
public partial class MainWindow : Window
{
    private object? selected;
    public bool AllowClose {get;set;}
    public MainWindow(MainViewModel model){InitializeComponent();DataContext=model;Closing+=ClosingWindow;}
    private MainViewModel Model=>(MainViewModel)DataContext;
    private void ClosingWindow(object? sender,CancelEventArgs e){if(!AllowClose){e.Cancel=true;Hide();}}
    private void ActionClick(object sender,RoutedEventArgs e)
    {
        if(e.OriginalSource is Button {Tag:string action} button)
        {
            e.Handled=true;
            if(action=="Save"&&FormValidation.HasErrors(this))
            {
                Model.Observe(Model.RunActionAsync("Status.Error",_=>Task.FromException(new ArgumentException(LocalizationService.ReadText(Model.Language,"Settings.Invalid")))));return;
            }
            Model.Observe(Model.HandleActionAsync(action,action=="Recommendation"?button.DataContext:selected));
        }
    }
    private void SelectionChanged(object sender,SelectionChangedEventArgs e)
    {
        if(e.OriginalSource is not DataGrid grid)return;
        selected=grid.SelectedItem;
        if(grid.Tag as string=="History" && selected is ScanSnapshot snapshot && Model.History.SelectedSnapshotId!=snapshot.Id)Model.Observe(Model.History.CompareSelectedAsync(snapshot));
    }
    public void ActivateWindow(AlertEvent? alert=null)
    {
        Show();if(WindowState==WindowState.Minimized)WindowState=WindowState.Normal;Activate();
        if(alert is not null)Pages.SelectedIndex=alert.Kind==AlertKind.LowSpace?0:1;
        Model.Observe(Model.ActivateAlertAsync(alert));
    }
}
