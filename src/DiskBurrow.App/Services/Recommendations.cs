using System.Diagnostics;
using DiskBurrow.Core.Scanning;
namespace DiskBurrow.App.Services;
public static class Recommendations
{
    public const string NuGet="https://learn.microsoft.com/en-us/nuget/consume-packages/managing-the-global-packages-and-cache-folders";
    public const string Pip="https://pip.pypa.io/en/stable/topics/caching/";
    public static IReadOnlyList<CacheRecommendation> FindObservedCaches(IReadOnlyList<DirectoryObservation> directories,string userProfile,string localAppData)
    {
        var known=new Dictionary<string,(string Program,string Url)>(StringComparer.OrdinalIgnoreCase)
        {
            [Path.Combine(userProfile,".nuget","packages")]=("NuGet",NuGet),
            [Path.Combine(localAppData,"NuGet","v3-cache")]=("NuGet",NuGet),
            [Path.Combine(localAppData,"pip","Cache")]=("pip",Pip)
        };
        return directories.Where(d=>known.ContainsKey(Path.TrimEndingDirectorySeparator(d.Path))).Select(d=>new CacheRecommendation(known[Path.TrimEndingDirectorySeparator(d.Path)].Program,d.Path,d.LogicalBytes,known[Path.TrimEndingDirectorySeparator(d.Path)].Url)).ToArray();
    }
    public const string Chrome="https://support.google.com/chrome/answer/2392709?co=GENIE.Platform%3DDesktop";
    public const string Edge="https://support.microsoft.com/en-us/edge/view-and-delete-browser-history-in-microsoft-edge";
    public static void OpenStorage()=>Process.Start(new ProcessStartInfo("ms-settings:storagesense"){UseShellExecute=true});
    public static void OpenInstructions(string url)
    {
        if(url is not (Chrome or Edge or NuGet or Pip))throw new ArgumentException("Unsupported recommendation.");
        Process.Start(new ProcessStartInfo(url){UseShellExecute=true});
    }
    public static void OpenFolder(string path)
    {
        if(!Path.IsPathFullyQualified(path)||path.StartsWith(@"\\",StringComparison.Ordinal)||!Directory.Exists(path))throw new ArgumentException("Existing local folder required.");
        var start=new ProcessStartInfo("explorer.exe"){UseShellExecute=true};start.ArgumentList.Add(path);Process.Start(start);
    }
}
public sealed record CacheRecommendation(string Program,string Path,long LogicalBytes,string InstructionsUrl);
