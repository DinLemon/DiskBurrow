using System.Windows;
using System.Windows.Controls;
using DiskBurrow.App.Services;
using DiskBurrow.Core.Cleanup;

namespace DiskBurrow.App.Views;

// This read-only window never invokes execution. The separate Delete action asks again.
public sealed class ManualDeletionReviewWindow : Window
{
    public ManualDeletionReviewWindow(ManualDeletePlan? plan,CleanupReport? report,LocalizationService locale,Func<long?,string> bytes)
    {
        Title=locale.Text("Manual.Review");Width=760;Height=560;MinWidth=480;MinHeight=360;
        WindowStartupLocation=WindowStartupLocation.CenterOwner;
        var panel=new DockPanel{Margin=new Thickness(20)};
        var close=new Button{Content=locale.Text("Manual.Close"),IsCancel=true,IsDefault=true,HorizontalAlignment=HorizontalAlignment.Right,Margin=new Thickness(0,12,0,0)};
        close.Click+=(_,_)=>Close();DockPanel.SetDock(close,Dock.Bottom);panel.Children.Add(close);
        var content=new StackPanel();
        void Text(string value,bool strong=false){content.Children.Add(new TextBlock{Text=value,TextWrapping=TextWrapping.Wrap,FontWeight=strong?FontWeights.SemiBold:FontWeights.Normal,Margin=new Thickness(0,0,0,10)});}
        string Reason(string key)=>LocalizationService.ReadKeys(locale.Language).Contains(key)?locale.Text(key):key;
        Text(locale.Text("Manual.PermanentWarning"),true);
        if(plan is not null)
        {
            Text($"{locale.Text("Manual.Files")}: {plan.FileCount:N0}   {locale.Text("Manual.Folders")}: {plan.DirectoryCount:N0}   {locale.Text("Logical")}: {bytes(plan.EstimatedDataBytes)}",true);
            Text(locale.Text("Manual.Roots"),true);
            content.Children.Add(new TextBox{Text=string.Join(Environment.NewLine,plan.Roots),IsReadOnly=true,TextWrapping=TextWrapping.Wrap,MaxHeight=180,VerticalScrollBarVisibility=ScrollBarVisibility.Auto,Margin=new Thickness(0,0,0,10)});
            Text(locale.Text(plan.CanExecute?"Manual.Ready":"Manual.Blocked"),true);
            foreach(var warning in plan.Warnings)Text($"{warning.Path}\n{Reason(warning.ReasonKey)}");
        }
        if(report is not null)
        {
            Text(locale.Text("Manual.Result"),true);
            Text($"{locale.Text("Cleanup.FreeDelta")}: {(report.FreeSpaceDeltaAvailable?bytes(report.FreeSpaceDeltaBytes):locale.Text("Unknown"))}");
            if(report.WasCancelled)Text(locale.Text("Status.Cancelled"));
            var grid=new DataGrid{AutoGenerateColumns=false,IsReadOnly=true,CanUserAddRows=false,CanUserDeleteRows=false,Height=220,
                ItemsSource=report.Items.Take(2000).Select(item=>new{Path=item.Audit?.Path??locale.Text("Unknown"),Outcome=locale.Text("Outcome."+item.Outcome),Reason=item.ReasonKey is {} key?Reason(key):""}).ToArray()};
            grid.Columns.Add(new DataGridTextColumn{Header=locale.Text("Path"),Binding=new System.Windows.Data.Binding("Path"),Width=new DataGridLength(1,DataGridLengthUnitType.Star)});
            grid.Columns.Add(new DataGridTextColumn{Header=locale.Text("Outcome"),Binding=new System.Windows.Data.Binding("Outcome"),Width=110});
            grid.Columns.Add(new DataGridTextColumn{Header=locale.Text("Reason"),Binding=new System.Windows.Data.Binding("Reason"),Width=180});
            content.Children.Add(grid);Text(locale.Text("Cleanup.OutcomeBound"));
        }
        panel.Children.Add(new ScrollViewer{Content=content,VerticalScrollBarVisibility=ScrollBarVisibility.Auto});Content=panel;
    }
}
