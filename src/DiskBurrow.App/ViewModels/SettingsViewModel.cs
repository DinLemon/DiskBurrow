using DiskBurrow.Core.Monitoring;
using DiskBurrow.Storage;
using DiskBurrow.Windows.System;
namespace DiskBurrow.App.ViewModels;
public sealed class SettingsViewModel : ObservableModel
{
    private const long GB=DiskBurrow.App.Services.ByteDisplay.BytesPerGB;
    private readonly SettingsStore store;
    private readonly AutostartRegistration autostart;
    private readonly Func<string,string?> approve;
    private readonly Action<AppSettings> apply;
    private AppSettings current;
    public int[] Intervals {get;}=[1,6,12,24];
    public string[] Languages {get;}=["ru","en"];
    public string[] Themes {get;}=["light","dark"];
    private string theme="light";
    public string Theme {get=>theme;set{if(theme==value)return;DiskBurrow.App.Services.ThemeService.Apply(value);theme=value;Refresh();}}
    public int IntervalHours {get;set;}
    public decimal LowGB {get;set;}
    public decimal GrowthGB {get;set;}
    public bool AllowOnBattery {get;set;}
    public bool Autostart {get;set;}
    public bool Paused {get;set;}
    public string Language {get;set;}
    public string Exclusions {get;set;}
    public string CustomTemp {get;set;}
    public string? AutostartDetails {get;private set;}
    public SettingsViewModel(AppSettings settings,SettingsStore store,AutostartRegistration autostart,Func<string,string?> approve,Action<AppSettings> apply)
    {
        current=settings;this.store=store;this.autostart=autostart;this.approve=approve;this.apply=apply;Theme=settings.Theme;
        IntervalHours=settings.IntervalHours;LowGB=settings.LowSpaceBytes/(decimal)GB;GrowthGB=settings.GrowthBytes/(decimal)GB;AllowOnBattery=settings.AllowOnBattery;Autostart=settings.Autostart;Paused=settings.Paused;Language=settings.Language;Exclusions=string.Join(Environment.NewLine,settings.ExcludedPaths);CustomTemp=settings.ApprovedCustomTempPath??"";
        try {if(autostart.IsPathChanged)AutostartDetails="Settings.AutostartMoved";}catch(Exception error){AutostartDetails=error.Message;}
    }
    public async Task SaveAsync(CancellationToken ct)
    {
        if(LowGB<0||GrowthGB<=0||LowGB>long.MaxValue/(decimal)GB||GrowthGB>long.MaxValue/(decimal)GB)throw new ArgumentException("Thresholds must be in range.");
        string? approved=null;
        if(!string.IsNullOrWhiteSpace(CustomTemp))approved=approve(CustomTemp.Trim())??throw new ArgumentException("Custom TEMP failed safety validation.");
        var next=current with {IntervalHours=IntervalHours,LowSpaceBytes=checked((long)(LowGB*GB)),GrowthBytes=checked((long)(GrowthGB*GB)),AllowOnBattery=AllowOnBattery,Autostart=Autostart,Paused=Paused,Language=Language,ExcludedPaths=Exclusions.Split(['\r','\n'],StringSplitOptions.RemoveEmptyEntries|StringSplitOptions.TrimEntries),ApprovedCustomTempPath=approved};
        next=next with {Theme=Theme};next.Validate();await store.SaveAsync(next,ct);
        // Only explicit Save can mutate the user's Run registration.
        autostart.Apply(next.Autostart);AutostartDetails=autostart.LastUserMessage;
        current=next;apply(next);Refresh();
    }
}
