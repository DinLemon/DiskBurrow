using System.Windows;
using System.Drawing;
using Forms=System.Windows.Forms;
using DiskBurrow.Core.Monitoring;
namespace DiskBurrow.App.Services;
public sealed class TrayService : IDisposable
{
    private readonly Forms.NotifyIcon icon;
    private readonly Icon ownedIcon;
    private readonly Forms.ToolStripMenuItem open=new(),scan=new(),pause=new(),exit=new();
    private readonly LocalizationService locale;
    private readonly Action<AlertEvent?> activate;
    private AlertEvent? pending;
    public TrayService(LocalizationService locale,Action<AlertEvent?> activate,Action scanNow,Action togglePause,Action exitApp)
    {
        this.locale=locale;this.activate=activate;
        using var stream=Application.GetResourceStream(new Uri("pack://application:,,,/Resources/DiskBurrow.ico"))!.Stream;
        ownedIcon=new Icon(stream);icon=new(){Icon=ownedIcon,Visible=true};
        var menu=new Forms.ContextMenuStrip();menu.Items.AddRange([open,scan,pause,exit]);icon.ContextMenuStrip=menu;
        open.Click+=(_,_)=>Activate();scan.Click+=(_,_)=>scanNow();pause.Click+=(_,_)=>togglePause();exit.Click+=(_,_)=>exitApp();
        icon.DoubleClick+=(_,_)=>Activate();icon.BalloonTipClicked+=(_,_)=>Activate();locale.Changed+=Localize;Localize();
    }
    private void Activate(){var alert=pending;pending=null;activate(alert);}
    private void Localize(){icon.Text=locale.Text("Tray.Hint");open.Text=locale.Text("Action.Open");scan.Text=locale.Text("Action.Scan");pause.Text=locale.Text("Action.Pause");exit.Text=locale.Text("Action.Exit");}
    public void SetState(bool busy,bool paused){scan.Enabled=!busy;pause.Checked=paused;}
    public void Notify(AlertEvent alert)
    {
        pending=alert;icon.BalloonTipTitle=locale.Text(alert.Kind==AlertKind.LowSpace?"Alert.Low":"Alert.Growth");icon.BalloonTipText=alert.DestinationPath;icon.BalloonTipIcon=Forms.ToolTipIcon.Warning;icon.ShowBalloonTip(10000);
    }
    public void Dispose(){locale.Changed-=Localize;icon.Visible=false;icon.ContextMenuStrip?.Dispose();icon.Dispose();ownedIcon.Dispose();}
}
