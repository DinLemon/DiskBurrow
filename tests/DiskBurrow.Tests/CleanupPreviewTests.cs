using System.Security.Cryptography;
using System.Text;
using DiskBurrow.Windows.Cleanup;
using DiskBurrow.Windows.Files;

namespace DiskBurrow.Tests;

public sealed class CleanupPreviewTests
{
    private static readonly DateTimeOffset Now = new(2026, 10, 5, 12, 0, 0, TimeSpan.Zero);

    [Fact]
    public async Task OnlyFilesOlderThanSevenDaysAreProposed()
    {
        using var fixture = new CleanupFixture();
        var old = Aged(fixture, "profile/AppData/Local/Temp/old.bin", 8);
        var exactlySevenDaysPath = Aged(fixture, "profile/AppData/Local/Temp/exact.bin", 7);
        Aged(fixture, "profile/AppData/Local/Temp/recent.bin", 6);
        var plan = await Planner(fixture).PreviewAsync(new HashSet<string>(), default);
        Assert.DoesNotContain(plan.Candidates, c => c.File.Path == exactlySevenDaysPath);
        var candidate = Assert.Single(plan.Candidates);
        Assert.Equal(old, candidate.File.Path);
        Assert.NotEqual(Guid.Empty, candidate.Id);
        Assert.NotNull(candidate.File.Identity);
        Assert.Equal(13, candidate.File.LogicalBytes);
        Assert.Equal(Now.AddDays(-8), candidate.File.ModifiedUtc);
        Assert.Equal(Now, plan.CreatedUtc);
        Assert.Equal("Cleanup.OldTemp", candidate.ReasonKey);
        Assert.Equal(13, plan.EstimatedDataBytes);
    }

    [Fact]
    public async Task CrashDumpRuleRequiresDmpExtension()
    {
        using var fixture = new CleanupFixture();
        var dump = Aged(fixture, "profile/AppData/Local/CrashDumps/app.DMP", 8);
        Aged(fixture, "profile/AppData/Local/CrashDumps/notes.txt", 8);
        Aged(fixture, "profile/AppData/Local/CrashDumps/exact.dmp", 7);
        Assert.Equal(dump, Assert.Single((await Planner(fixture).PreviewAsync(new HashSet<string>(), default)).Candidates).File.Path);
    }

    [Fact]
    public async Task BrowserRuleKeepsCookiesHistoryAndPasswords()
    {
        using var fixture = new CleanupFixture();
        const string profile = "profile/AppData/Local/Google/Chrome/User Data/Default/";
        var cache = fixture.Tree.FileAt(profile + "Cache/Cache_Data/data_0", 31);
        var cookiesPath = fixture.Tree.FileAt(profile + "Network/Cookies", 43);
        fixture.Tree.FileAt(profile + "History", 59);
        fixture.Tree.FileAt(profile + "Login Data", 67);
        fixture.Tree.FileAt(profile + "Extensions/id/content", 73);
        var plan = await Planner(fixture).PreviewAsync(new HashSet<string>(), default);
        Assert.DoesNotContain(plan.Candidates, c => c.File.Path == cookiesPath);
        Assert.Equal(cache, Assert.Single(plan.Candidates).File.Path);
    }

    [Fact]
    public async Task ExclusionsApplyToDescendants()
    {
        using var fixture = new CleanupFixture();
        Aged(fixture, "profile/AppData/Local/Temp/keep/nested/one.bin", 8);
        var sibling = Aged(fixture, "profile/AppData/Local/Temp/keeper/two.bin", 8);
        var exact = Aged(fixture, "profile/AppData/Local/Temp/exact.bin", 8);
        var excluded = Path.Combine(fixture.Environment.KnownDirectories.LocalAppData, "Temp", "KEEP", @"..\KEEP\");
        var plan = await Planner(fixture).PreviewAsync(new HashSet<string> { excluded, exact }, default);
        Assert.Equal(sibling, Assert.Single(plan.Candidates).File.Path);
    }

    [Fact]
    public async Task PreviewNeverMutatesFilesystem()
    {
        using var fixture = new CleanupFixture();
        Aged(fixture, "profile/AppData/Local/Temp/nested/file.bin", 8);
        var beforeTreeHash = HashTree(fixture.Tree.Root);
        var plan = await Planner(fixture).PreviewAsync(new HashSet<string>(), default);
        var afterTreeHash = HashTree(fixture.Tree.Root);
        Assert.Equal(beforeTreeHash, afterTreeHash);
        Assert.Single(plan.Candidates);
    }

    [Fact]
    public async Task NoCandidateIsInitiallySelected()
    {
        using var fixture = new CleanupFixture();
        Aged(fixture, "profile/AppData/Local/Temp/file.bin", 8);
        var plan = await Planner(fixture).PreviewAsync(new HashSet<string>(), default);
        Assert.Single(plan.Candidates);
        Assert.Empty(plan.SelectedIds);
    }

    [Fact]
    public async Task UnsafeEntriesAndAlternateStreamsAreIgnored()
    {
        using var fixture = new CleanupFixture();
        var safe = Aged(fixture, "profile/AppData/Local/Temp/safe.bin", 8);
        var readOnly = Aged(fixture, "profile/AppData/Local/Temp/readonly.bin", 8);
        File.SetAttributes(readOnly, FileAttributes.ReadOnly);
        var cloud = Aged(fixture, "profile/AppData/Local/Temp/cloud.bin", 8);
        fixture.Tree.DirectoryAt("profile/AppData/Local/Temp/empty");
        Aged(fixture, "outside/secret.bin", 8);
        fixture.Tree.Junction("profile/AppData/Local/Temp/link", fixture.Tree.DirectoryAt("outside"));
        File.WriteAllText(safe + ":extra", "stream content");
        File.SetLastWriteTimeUtc(safe, Now.AddDays(-8).UtcDateTime);
        try
        {
            var plan = await Planner(fixture, new CloudFileApi(cloud)).PreviewAsync(new HashSet<string>(), default);
            Assert.Equal(safe, Assert.Single(plan.Candidates).File.Path);
        }
        finally { File.SetAttributes(readOnly, FileAttributes.Normal); }
    }

    [Fact]
    public async Task PreviewChecksBrowserStateAgain()
    {
        using var fixture = new CleanupFixture();
        fixture.Tree.FileAt("profile/AppData/Local/Google/Chrome/User Data/Default/Cache/Cache_Data/data", 10);
        var planner = Planner(fixture);
        Assert.Single((await planner.PreviewAsync(new HashSet<string>(), default)).Candidates);
        fixture.Environment.ProcessStates["chrome"] = OwnerProcessState.Running;
        var second = await planner.PreviewAsync(new HashSet<string>(), default);
        Assert.Empty(second.Candidates);
        Assert.Contains(second.Warnings, warning => warning.ReasonKey == "Cleanup.OwnerRunning");
    }

    private static WindowsCleanupPlanner Planner(CleanupFixture fixture, NativeFileApi? files = null) =>
        new(fixture.Discovery(files), files, new FixedClock());
    private static string Aged(CleanupFixture fixture, string relative, int days)
    {
        var path = fixture.Tree.FileAt(relative, 13);
        File.SetLastWriteTimeUtc(path, Now.AddDays(-days).UtcDateTime);
        return path;
    }
    private static string HashTree(string root)
    {
        var rows = Directory.GetFileSystemEntries(root, "*", SearchOption.AllDirectories).Order(StringComparer.Ordinal)
            .Select(path => Directory.Exists(path) ? $"D:{Path.GetRelativePath(root, path)}" :
                $"F:{Path.GetRelativePath(root, path)}:{File.GetAttributes(path)}:{File.GetLastWriteTimeUtc(path):O}:{Convert.ToHexString(SHA256.HashData(File.ReadAllBytes(path)))}");
        return Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(string.Join('\n', rows))));
    }
    private sealed class FixedClock : TimeProvider { public override DateTimeOffset GetUtcNow() => Now; }
    private sealed class CloudFileApi(string cloud) : NativeFileApi
    {
        protected override FileAttributes ReadAttributes(string path) => base.ReadAttributes(path) |
            (path.Equals(cloud, StringComparison.OrdinalIgnoreCase) ? FileAttributes.Offline : 0);
    }
}
