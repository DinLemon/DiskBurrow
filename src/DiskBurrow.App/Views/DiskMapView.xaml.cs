using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using DiskBurrow.App.ViewModels;
namespace DiskBurrow.App.Views;
public partial class DiskMapView : UserControl
{
 public DiskMapView(){InitializeComponent();}
 private DiskMapViewModel? Model=>MapCanvas.Model;
 private void MapAction(object sender,RoutedEventArgs e)
 {
  if(Model is not {} vm)return;
  switch((sender as Button)?.Tag?.ToString()) {case "Back":vm.Back();break;case "Forward":vm.Forward();break;case "Up":vm.Up();break;case "Root":vm.Root();break;case "ZoomIn":vm.ChangeZoom(1.3);break;case "ZoomOut":vm.ChangeZoom(1/1.3);break;case "ZoomReset":MapCanvas.HandleKey(Key.D0);break;case "Mark":vm.ToggleFocusedMark();break;case "Open":MapCanvas.OpenFocused();break;}
  e.Handled=true;
 }
 private void UnmarkClick(object sender,RoutedEventArgs e){if(DataContext is MainViewModel main&&!main.Busy&&(sender as Button)?.DataContext is string path)main.Manual.Select(path,false);e.Handled=true;}
 private void BreadcrumbClick(object sender,RoutedEventArgs e){if((sender as Button)?.Tag is int index)Model?.Navigate(index);e.Handled=true;}
 private void SearchSelection(object sender,SelectionChangedEventArgs e){if((sender as ListBox)?.SelectedItem is MapSearchResult row)Model?.Focus(row.Index);e.Handled=true;}
 private void SearchDoubleClick(object sender,MouseButtonEventArgs e){if((sender as ListBox)?.SelectedItem is MapSearchResult row)Model?.Navigate(row.Index);e.Handled=true;}
}
