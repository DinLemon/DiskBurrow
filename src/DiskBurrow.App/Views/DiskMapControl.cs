using System.Globalization;
using System.Windows;
using System.Windows.Input;
using System.Windows.Media;
using DiskBurrow.App.ViewModels;
using DiskBurrow.App.Services;
namespace DiskBurrow.App.Views;

public sealed class DiskMapControl : FrameworkElement
{
 public static readonly DependencyProperty ModelProperty=DependencyProperty.Register(nameof(Model),typeof(DiskMapViewModel),typeof(DiskMapControl),new FrameworkPropertyMetadata(null,FrameworkPropertyMetadataOptions.AffectsRender,ModelChanged));
 public static readonly DependencyProperty LanguageCodeProperty=DependencyProperty.Register(nameof(LanguageCode),typeof(string),typeof(DiskMapControl),new FrameworkPropertyMetadata("ru",FrameworkPropertyMetadataOptions.AffectsRender));
 public DiskMapViewModel? Model {get=>(DiskMapViewModel?)GetValue(ModelProperty);set=>SetValue(ModelProperty,value);}
 public string LanguageCode {get=>(string)GetValue(LanguageCodeProperty);set=>SetValue(LanguageCodeProperty,value);}
 private IReadOnlyList<DisplayMapTile> tiles=[];private Point pan;private Point last;private bool dragging;
 private static readonly Typeface Typeface=new("Segoe UI");
 public DiskMapControl(){Focusable=true;ClipToBounds=true;Cursor=Cursors.Hand;Loaded+=(_,_)=>Subscribe();Unloaded+=(_,_)=>Unsubscribe();}
 private static void ModelChanged(DependencyObject d,DependencyPropertyChangedEventArgs e){var view=(DiskMapControl)d;if(e.OldValue is DiskMapViewModel old)old.MapChanged-=view.ModelUpdated;if(e.NewValue is DiskMapViewModel current&&view.IsLoaded)current.MapChanged+=view.ModelUpdated;view.pan=new();}
 private void Subscribe(){if(Model is {} model){model.MapChanged-=ModelUpdated;model.MapChanged+=ModelUpdated;}}
 private int observedDirectory=-1;
 private void ModelUpdated(){if(Model is {} m&&(m.CurrentIndex!=observedDirectory||m.Zoom==1)){pan=new();observedDirectory=m.CurrentIndex;}InvalidateVisual();}
 private void Unsubscribe(){if(Model is {} model)model.MapChanged-=ModelUpdated;}
 protected override void OnRender(DrawingContext dc)
 {
  base.OnRender(dc);dc.DrawRectangle(new SolidColorBrush(Color.FromRgb(239,244,249)),null,new Rect(RenderSize));
  if(Model is not {} model||model.Tree is not {} tree)return;
  tiles=model.Layout(ActualWidth,ActualHeight);dc.PushTransform(new MatrixTransform(model.Zoom,0,0,model.Zoom,pan.X,pan.Y));
  foreach(var tile in tiles)
  {
   var r=tile.Bounds;var rect=new Rect(r.X+1,r.Y+1,Math.Max(0,r.Width-2),Math.Max(0,r.Height-2));if(rect.Width<1||rect.Height<1)continue;
   var entry=tile.Index>=0?tree.Entries[tile.Index]:default;var color=tile.Index<0?Color.FromRgb(126,145,163):ClassColor(entry.Name,entry.IsDirectory);
   var pen=new Pen(tile.Matched?Brushes.Gold:tile.Index>=0&&model.IsMarked(tile.Index)?Brushes.OrangeRed:Brushes.White,tile.Matched||tile.Index>=0&&model.IsMarked(tile.Index)?3:1);
   dc.DrawRoundedRectangle(new SolidColorBrush(color),pen,rect,2,2);
   if(tile.Index>=0&&model.IsCoveredByMark(tile.Index))dc.DrawRectangle(null,new Pen(Brushes.OrangeRed,2){DashStyle=DashStyles.Dash},rect);
   if(tile.Index==model.FocusedIndex)dc.DrawRectangle(null,new Pen(Brushes.Black,2),rect);
   if(rect.Width>35&&rect.Height>18)
   {
    var label=tile.Index<0?LocalizationService.ReadText(LanguageCode,"Map.Others"):entry.Name;
    var formatted=new FormattedText(label,CultureInfo.GetCultureInfo(LanguageCode),FlowDirection.LeftToRight,Typeface,12,Brushes.White,VisualTreeHelper.GetDpi(this).PixelsPerDip){MaxTextWidth=Math.Max(1,rect.Width-10),MaxTextHeight=Math.Min(40,rect.Height-4),Trimming=TextTrimming.CharacterEllipsis};
    dc.DrawText(formatted,new(rect.X+5,rect.Y+3));
   }
  }
  dc.Pop();
 }
 public static Color ClassColor(string name,bool directory)
 {
  if(directory)return Color.FromRgb(45,92,144);return Path.GetExtension(name).ToLowerInvariant() switch{
   ".mp4" or ".mkv" or ".avi" or ".mp3" or ".wav"=>Color.FromRgb(132,74,152),
   ".jpg" or ".png" or ".webp" or ".gif"=>Color.FromRgb(52,129,136),
   ".zip" or ".7z" or ".rar" or ".gz"=>Color.FromRgb(172,112,45),
   ".exe" or ".dll" or ".msi"=>Color.FromRgb(89,100,157),
   ".txt" or ".pdf" or ".docx" or ".md"=>Color.FromRgb(69,135,86),
   ".vhdx" or ".vhd" or ".iso"=>Color.FromRgb(153,75,75),_=>Color.FromRgb(72,117,165)};
 }
 private DisplayMapTile? Hit(Point point){if(Model is null)return null;var x=(point.X-pan.X)/Model.Zoom;var y=(point.Y-pan.Y)/Model.Zoom;for(var i=tiles.Count-1;i>=0;i--)if(tiles[i].Bounds.Contains(x,y))return tiles[i];return null;}
 protected override void OnMouseMove(MouseEventArgs e)
 {
  base.OnMouseMove(e);var p=e.GetPosition(this);if(dragging){pan+=p-last;last=p;InvalidateVisual();return;}
  var hit=Hit(p);if(Model?.Tree is {} tree&&hit is {Index:>=0} tile){var item=tree.Entries[tile.Index];ToolTip=$"{tree.GetPath(tile.Index)}\n{LocalizationService.ReadText(LanguageCode,"Logical")}: {FormatBytes(item.LogicalBytes)}\n{LocalizationService.ReadText(LanguageCode,"Allocated")}: {(item.AllocatedBytes is {} bytes?FormatBytes(bytes):LocalizationService.ReadText(LanguageCode,"Unknown"))}\n{LocalizationService.ReadText(LanguageCode,"Coverage")}: {LocalizationService.ReadText(LanguageCode,item.CoverageComplete?"Yes":"No")}";}else ToolTip=null;
 }
 protected override void OnMouseDown(MouseButtonEventArgs e)
 {
  base.OnMouseDown(e);Focus();var p=e.GetPosition(this);
  if(e.ChangedButton==MouseButton.Middle){dragging=true;last=p;CaptureMouse();e.Handled=true;return;}
  if(Hit(p) is not {Index:>=0} tile)return;Model!.Focus(tile.Index);
  if(e.ChangedButton==MouseButton.Left){if(Keyboard.Modifiers.HasFlag(ModifierKeys.Control))Model.ToggleMark(tile.Index);else if(e.ClickCount==2)Model.Navigate(tile.Index);}
  if(e.ChangedButton==MouseButton.Right)
  {
   var menu=new System.Windows.Controls.ContextMenu();void Add(string key,Action action){var item=new System.Windows.Controls.MenuItem{Header=LocalizationService.ReadText(LanguageCode,key)};item.Click+=(_,_)=>action();menu.Items.Add(item);}
   Add("Map.Enter",()=>Model.Navigate(tile.Index));Add("Map.Mark",()=>Model.ToggleMark(tile.Index));Add("Action.Open",()=>OpenFocused());ContextMenu=menu;menu.IsOpen=true;
  }
  e.Handled=true;
 }
 protected override void OnMouseUp(MouseButtonEventArgs e){base.OnMouseUp(e);if(e.ChangedButton==MouseButton.Middle){dragging=false;ReleaseMouseCapture();e.Handled=true;}}
 protected override void OnMouseWheel(MouseWheelEventArgs e)
 {
  base.OnMouseWheel(e);if(Model is null)return;var p=e.GetPosition(this);var old=Model.Zoom;Model.ChangeZoom(e.Delta>0?1.3:1/1.3);var ratio=Model.Zoom/old;pan=Model.Zoom==1?new():new(p.X-(p.X-pan.X)*ratio,p.Y-(p.Y-pan.Y)*ratio);InvalidateVisual();e.Handled=true;
 }
 private string FormatBytes(long bytes)=>$"{bytes/1073741824d:N2} {(LanguageCode=="ru"?"ГиБ":"GiB")}";
 public void OpenFocused(){if(Model?.Tree is not {} tree||Model.FocusedIndex<0)return;var entry=tree.Entries[Model.FocusedIndex];var path=tree.GetPath(Model.FocusedIndex);try{Recommendations.OpenFolder(entry.IsDirectory?path:Path.GetDirectoryName(path)!);}catch(Exception error) when(error is ArgumentException or System.ComponentModel.Win32Exception or IOException){Model.ReportError(error.Message);}}
 public bool HandleKey(Key key,ModifierKeys modifiers=ModifierKeys.None)
 {
  if(Model is not {} model)return false;
  switch(key){case Key.Left:if(modifiers.HasFlag(ModifierKeys.Alt))model.Back();else model.MoveFocus(-1);break;case Key.Right:if(modifiers.HasFlag(ModifierKeys.Alt))model.Forward();else model.MoveFocus(1);break;
   case Key.Up:case Key.Back:case Key.Escape:model.Up();break;case Key.Down:case Key.Enter:if(model.FocusedIndex>=0)model.Navigate(model.FocusedIndex);break;
   case Key.Space:model.ToggleFocusedMark();break;
   case Key.Add:case Key.OemPlus:model.ChangeZoom(1.3);break;case Key.Subtract:case Key.OemMinus:model.ChangeZoom(1/1.3);break;case Key.D0:case Key.NumPad0:model.ResetZoom();pan=new();break;default:return false;}
  InvalidateVisual();return true;
 }
 protected override void OnKeyDown(KeyEventArgs e){base.OnKeyDown(e);e.Handled=HandleKey(e.Key==Key.System?e.SystemKey:e.Key,Keyboard.Modifiers);}
}
