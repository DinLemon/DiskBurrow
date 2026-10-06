using DiskBurrow.Windows.Cleanup;
using DiskBurrow.Windows.Files;

namespace DiskBurrow.Tests;

public sealed class RuleDiscoveryTests
{
    [Fact]
    public void StandardTempIsDiscoveredWithoutHintApproval()
    {
        using var fixture = new CleanupFixture();
        var temp = fixture.Tree.DirectoryAt("profile/AppData/Local/Temp");
        Assert.Contains(fixture.Discovery().Discover().Roots, rule => rule.RuleId == "UserTemp" && rule.Path == temp);
    }

    [Fact]
    public void BroadAndPrefixSiblingRootsAreRejected()
    {
        using var fixture = new CleanupFixture();
        foreach (var hint in new[] { Path.GetPathRoot(fixture.Tree.Root)!, fixture.Environment.KnownDirectories.UserProfile,
                     fixture.Environment.KnownDirectories.LocalAppData, fixture.Environment.KnownDirectories.Windows,
                     fixture.Tree.DirectoryAt("profile/AppData/Local/TempSibling"), fixture.Tree.DirectoryAt("profile/Documents") })
        {
            fixture.Environment.KnownDirectories = fixture.Environment.KnownDirectories with { TempHint = hint };
            var discovery = fixture.Discovery();
            Assert.DoesNotContain(discovery.Discover().Roots, rule => rule.Path == hint);
            if (hint.EndsWith("TempSibling", StringComparison.Ordinal)) continue; // explicit approval is a separate setting.
            Assert.Null(discovery.ApproveCustomTempRoot(hint));
            Assert.Contains(discovery.Discover().Warnings, warning => warning.RuleId == "UserTemp");
        }
    }

    [Fact]
    public void CustomTempRequiresMatchingHintAndExplicitApproval()
    {
        using var fixture = new CleanupFixture();
        var custom = fixture.Tree.DirectoryAt("scratch/Temp");
        fixture.Environment.KnownDirectories = fixture.Environment.KnownDirectories with { TempHint = custom };
        var discovery = fixture.Discovery();
        Assert.DoesNotContain(discovery.Discover().Roots, rule => rule.Path == custom);
        Assert.Null(discovery.ApproveCustomTempRoot(fixture.Tree.Root));
        var approved = Assert.IsType<ApprovedTempRoot>(discovery.ApproveCustomTempRoot(custom));
        Assert.Contains(fixture.Discovery(approved: [approved]).Discover().Roots, rule => rule.Path == custom);
        fixture.Environment.KnownDirectories = fixture.Environment.KnownDirectories with { TempHint = fixture.Tree.Root };
        Assert.DoesNotContain(fixture.Discovery(approved: [approved]).Discover().Roots, rule => rule.Path == custom);
    }

    [Fact]
    public void JunctionAncestorsAreRejected()
    {
        using var fixture = new CleanupFixture();
        var actual = fixture.Tree.DirectoryAt("actual/Local/Temp");
        var junction = fixture.Tree.Junction("redirect", fixture.Tree.DirectoryAt("actual"));
        fixture.Environment.KnownDirectories = fixture.Environment.KnownDirectories with
        { LocalAppData = Path.Combine(junction, "Local"), TempHint = Path.Combine(junction, "Local", "Temp") };
        var result = fixture.Discovery().Discover();
        Assert.Empty(result.Roots);
        Assert.Contains(result.Warnings, warning => warning.ReasonKey == "Cleanup.UnsafeRoot");
        Assert.True(Directory.Exists(actual));
    }

    [Fact]
    public void CloudAncestorsAreRejected()
    {
        using var fixture = new CleanupFixture();
        fixture.Tree.DirectoryAt("profile/AppData/Local/Temp");
        Assert.Empty(fixture.Discovery(new CloudAncestorApi(fixture.Environment.KnownDirectories.UserProfile)).Discover().Roots);
    }

    [Theory]
    [InlineData(OwnerProcessState.Running, "Cleanup.OwnerRunning")]
    [InlineData(OwnerProcessState.Unavailable, "Cleanup.OwnerUnavailable")]
    public void RunningBrowserDisablesItsRule(OwnerProcessState state, string reason)
    {
        using var fixture = new CleanupFixture();
        var cache = fixture.Tree.DirectoryAt("profile/AppData/Local/Google/Chrome/User Data/Default/Cache/Cache_Data");
        fixture.Environment.ProcessStates["chrome"] = state;
        var result = fixture.Discovery().Discover();
        Assert.DoesNotContain(result.Roots, rule => rule.Path == cache);
        Assert.Contains(result.Warnings, warning => warning.RuleId == "ChromeCache" && warning.ReasonKey == reason);
    }

    [Fact]
    public void BrowserDiscoveryUsesOnlyKnownProfilesAndExactCacheData()
    {
        using var fixture = new CleanupFixture();
        var expected = fixture.Tree.DirectoryAt("profile/AppData/Local/Microsoft/Edge/User Data/Profile 2/Cache/Cache_Data");
        fixture.Tree.DirectoryAt("profile/AppData/Local/Microsoft/Edge/User Data/Guest Profile/Cache/Cache_Data");
        fixture.Tree.DirectoryAt("profile/AppData/Local/Microsoft/Edge/User Data/Default/Code Cache");
        var root = Assert.Single(fixture.Discovery().Discover().Roots);
        Assert.Equal(expected, root.Path);
        Assert.Equal("msedge", root.OwnerProcessName);
    }

    [Fact]
    public async Task VolumeRootExclusionSuppressesEveryCandidate()
    {
        using var fixture = new CleanupFixture();
        fixture.Tree.FileAt("profile/AppData/Local/Google/Chrome/User Data/Default/Cache/Cache_Data/data", 10);
        var plan = await new WindowsCleanupPlanner(fixture.Discovery()).PreviewAsync(
            new HashSet<string> { Path.GetPathRoot(fixture.Tree.Root)! }, default);
        Assert.Empty(plan.Candidates);
    }

    [Fact]
    public void UnsafeApprovedTempIsRejectedAfterParentReplacement()
    {
        using var fixture = new CleanupFixture();
        var custom = fixture.Tree.DirectoryAt("scratch/Temp");
        fixture.Environment.KnownDirectories = fixture.Environment.KnownDirectories with { TempHint = custom };
        var approved = Assert.IsType<ApprovedTempRoot>(fixture.Discovery().ApproveCustomTempRoot(custom));
        Directory.Move(Path.Combine(fixture.Tree.Root, "scratch"), Path.Combine(fixture.Tree.Root, "moved"));
        fixture.Tree.Junction("scratch", Path.Combine(fixture.Tree.Root, "moved"));
        var result = fixture.Discovery(approved: [approved]).Discover();
        Assert.DoesNotContain(result.Roots, rule => rule.Path == custom);
        Assert.Contains(result.Warnings, warning => warning.RuleId == "UserTemp");
    }

    [Theory]
    [InlineData("Windows/scratch/Temp")]
    [InlineData("Program Files/scratch/Temp")]
    [InlineData("ProgramData/scratch/Temp")]
    public void ProtectedAreaDescendantsCannotBeApproved(string relative)
    {
        using var fixture = new CleanupFixture();
        var custom = fixture.Tree.DirectoryAt(relative);
        fixture.Environment.KnownDirectories = fixture.Environment.KnownDirectories with { TempHint = custom };
        Assert.Null(fixture.Discovery().ApproveCustomTempRoot(custom));
    }

    [Fact]
    public void DedicatedSubdirectoryOfLibraryCanBeExplicitlyApproved()
    {
        using var fixture = new CleanupFixture();
        var custom = fixture.Tree.DirectoryAt("profile/Documents/Scratch/Temp");
        fixture.Environment.KnownDirectories = fixture.Environment.KnownDirectories with { TempHint = custom };
        var approved = Assert.IsType<ApprovedTempRoot>(fixture.Discovery().ApproveCustomTempRoot(custom));
        Assert.Contains(fixture.Discovery(approved: [approved]).Discover().Roots, rule => rule.Path == custom);
    }

    [Fact]
    public void AlternateStreamAndNetworkRootCannotBeApproved()
    {
        using var fixture = new CleanupFixture();
        var custom = fixture.Tree.DirectoryAt("scratch/Temp");
        foreach (var path in new[] { custom + ":stream", @"\\localhost\share\Temp", Path.Combine(Path.GetPathRoot(custom)!, "Temp") })
        {
            fixture.Environment.KnownDirectories = fixture.Environment.KnownDirectories with { TempHint = path };
            Assert.Null(fixture.Discovery().ApproveCustomTempRoot(path));
        }
    }

    [Fact]
    public void RedirectedDownloadsCannotBecomeCustomTempRoot()
    {
        using var fixture = new CleanupFixture();
        var redirected = fixture.Tree.DirectoryAt("relocated/Downloads");
        var actualKnown = new WindowsRuleEnvironment(() => redirected).KnownDirectories;
        Assert.Contains(redirected, actualKnown.UserLibraryRoots);
        fixture.Environment.KnownDirectories = fixture.Environment.KnownDirectories with
        { TempHint = redirected, UserLibraryRoots = actualKnown.UserLibraryRoots, UserLibraryRootsVerified = actualKnown.UserLibraryRootsVerified };
        Assert.Null(fixture.Discovery().ApproveCustomTempRoot(redirected));
    }

    [Fact]
    public void DownloadsLookupFailureDisablesCustomApprovalButKeepsStandardTemp()
    {
        using var fixture = new CleanupFixture();
        var standard = fixture.Tree.DirectoryAt("profile/AppData/Local/Temp");
        var custom = fixture.Tree.DirectoryAt("scratch/Temp");
        var actualKnown = new WindowsRuleEnvironment(() => null).KnownDirectories;
        fixture.Environment.KnownDirectories = fixture.Environment.KnownDirectories with
        { TempHint = custom, UserLibraryRoots = actualKnown.UserLibraryRoots, UserLibraryRootsVerified = actualKnown.UserLibraryRootsVerified };
        var discovery = fixture.Discovery();
        Assert.Null(discovery.ApproveCustomTempRoot(custom));
        var result = discovery.Discover();
        Assert.Contains(result.Roots, root => root.Path == standard);
        Assert.Contains(result.Warnings, warning => warning.ReasonKey == "Cleanup.KnownFoldersUnavailable");
    }

    [Fact]
    public void NativeDownloadsLookupReturnsCurrentUserKnownFolder()
    {
        var known = new WindowsRuleEnvironment().KnownDirectories;
        Assert.True(known.UserLibraryRootsVerified);
        var expected = Assert.IsType<string>(Microsoft.Win32.Registry.GetValue(
            @"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders",
            "{374DE290-123F-4565-9164-39C4925E467B}", null));
        Assert.Contains(known.UserLibraryRoots, path => path.Equals(expected, StringComparison.OrdinalIgnoreCase));
    }

    [Theory]
    [InlineData(DriveType.Network)]
    [InlineData(DriveType.Unknown)]
    [InlineData(DriveType.NoRootDirectory)]
    [InlineData(DriveType.CDRom)]
    public void MappedOrUnconfirmedCustomVolumeIsRejectedBeforeMetadata(DriveType state)
    {
        using var fixture = new CleanupFixture();
        const string mappedTemp = @"Z:\scratch\Temp";
        fixture.Environment.KnownDirectories = fixture.Environment.KnownDirectories with { TempHint = mappedTemp };
        var files = new SyntheticDirectoryApi();
        var discovery = new RuleDiscovery(fixture.Environment, files, driveType: root => root == @"Z:\" ? state : DriveType.Fixed);
        Assert.Null(discovery.ApproveCustomTempRoot(mappedTemp));
        Assert.Empty(files.InspectedPaths);
    }

    [Theory]
    [InlineData(DriveType.Network)]
    [InlineData(DriveType.Unknown)]
    [InlineData(DriveType.NoRootDirectory)]
    [InlineData(DriveType.CDRom)]
    public async Task NonLocalKnownRootsAreRejectedBeforeMetadataOrTraversal(DriveType state)
    {
        using var fixture = new CleanupFixture();
        fixture.Tree.DirectoryAt("profile/AppData/Local/Temp");
        fixture.Tree.FileAt("profile/AppData/Local/Google/Chrome/User Data/Default/Cache/Cache_Data/data", 10);
        var files = new RecordingNativeApi();
        var discovery = fixture.Discovery(files, driveType: _ => state);
        var catalog = discovery.Discover();
        Assert.Empty(catalog.Roots);
        Assert.Contains(catalog.Warnings, warning => warning.ReasonKey == "Cleanup.NonLocalVolume");
        var plan = await new WindowsCleanupPlanner(discovery, files).PreviewAsync(new HashSet<string>(), default);
        Assert.Empty(plan.Candidates);
        Assert.Empty(files.InspectedPaths);
    }

    [Theory]
    [InlineData(DriveType.Fixed)]
    [InlineData(DriveType.Removable)]
    [InlineData(DriveType.Ram)]
    public void ConfirmedLocalCustomVolumesRemainEligible(DriveType state)
    {
        using var fixture = new CleanupFixture();
        var custom = fixture.Tree.DirectoryAt("scratch/Temp");
        fixture.Environment.KnownDirectories = fixture.Environment.KnownDirectories with { TempHint = custom };
        var discovery = fixture.Discovery(driveType: root =>
        {
            Assert.True(root.EndsWith('\\'));
            Assert.Equal(Path.GetPathRoot(custom), root);
            return state;
        });
        Assert.NotNull(discovery.ApproveCustomTempRoot(custom));
    }

    [Fact]
    public void ApprovalIsInvalidatedWhenVolumeBecomesRemote()
    {
        using var fixture = new CleanupFixture();
        var custom = fixture.Tree.DirectoryAt("scratch/Temp");
        fixture.Environment.KnownDirectories = fixture.Environment.KnownDirectories with { TempHint = custom };
        var state = DriveType.Fixed;
        var files = new RecordingNativeApi();
        var approved = Assert.IsType<ApprovedTempRoot>(fixture.Discovery(files, driveType: _ => state).ApproveCustomTempRoot(custom));
        files.InspectedPaths.Clear();
        state = DriveType.Network;
        Assert.Empty(fixture.Discovery(files, [approved], _ => state).Discover().Roots);
        Assert.Empty(files.InspectedPaths);
    }

    [Fact]
    public void LocalityLookupFailureIsFailClosedBeforeMetadata()
    {
        using var fixture = new CleanupFixture();
        var custom = fixture.Tree.DirectoryAt("scratch/Temp");
        fixture.Environment.KnownDirectories = fixture.Environment.KnownDirectories with { TempHint = custom };
        var files = new RecordingNativeApi();
        var discovery = fixture.Discovery(files, driveType: _ => throw new IOException("Volume lookup unavailable."));
        Assert.Null(discovery.ApproveCustomTempRoot(custom));
        Assert.Empty(discovery.Discover().Roots);
        Assert.Empty(files.InspectedPaths);
    }

    private sealed class RecordingNativeApi : NativeFileApi
    {
        public List<string> InspectedPaths { get; } = [];
        public override DiskBurrow.Core.Scanning.FileObservation Inspect(string path)
        {
            InspectedPaths.Add(path);
            return base.Inspect(path);
        }
    }
    private sealed class SyntheticDirectoryApi : NativeFileApi
    {
        public List<string> InspectedPaths { get; } = [];
        public override DiskBurrow.Core.Scanning.FileObservation Inspect(string path)
        {
            InspectedPaths.Add(path);
            return new(path, new(1, "directory"), 0, 0, DateTimeOffset.UtcNow, 1, FileAttributes.Directory);
        }
    }

    private sealed class CloudAncestorApi(string blockedPath) : NativeFileApi
    {
        protected override FileAttributes ReadAttributes(string path) => base.ReadAttributes(path) |
            (path.Equals(blockedPath, StringComparison.OrdinalIgnoreCase) ? FileAttributes.Offline : 0);
    }
}

internal sealed class CleanupFixture : IDisposable
{
    public TempTree Tree { get; } = new();
    public FixtureRuleEnvironment Environment { get; }
    public CleanupFixture()
    {
        Environment = new(new(Tree.DirectoryAt("profile"), Tree.DirectoryAt("profile/AppData/Local"),
            Tree.DirectoryAt("Windows"), Tree.DirectoryAt("Program Files"), Tree.DirectoryAt("Program Files (x86)"),
            Tree.DirectoryAt("ProgramData"), Path.Combine(Tree.Root, "profile", "AppData", "Local", "Temp"))
        { UserLibraryRoots = [Tree.DirectoryAt("profile/Documents")] });
    }
    public RuleDiscovery Discovery(NativeFileApi? files = null, IReadOnlyList<ApprovedTempRoot>? approved = null,
        Func<string, DriveType>? driveType = null) => new(Environment, files, approved, driveType);
    public void Dispose() => Tree.Dispose();
}

internal sealed class FixtureRuleEnvironment(CleanupKnownDirectories directories) : IRuleEnvironment
{
    public CleanupKnownDirectories KnownDirectories { get; set; } = directories;
    public Dictionary<string, OwnerProcessState> ProcessStates { get; } = [];
    public OwnerProcessState CheckOwnerProcess(string processName) => ProcessStates.GetValueOrDefault(processName, OwnerProcessState.Closed);
}
