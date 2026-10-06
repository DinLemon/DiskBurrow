namespace DiskBurrow.App.Services;
public static class ProjectLinks
{
    public const string Repository = "https://github.com/DinLemon/DiskBurrow";
    public const string Releases = "https://github.com/DinLemon/DiskBurrow/releases";
    public static void Open(bool releases)
        => System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo(releases ? Releases : Repository) { UseShellExecute = true });
}
