using System.Diagnostics;
using System.IO.Compression;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using DiskBurrow.App.Services;

namespace DiskBurrow.Tests;

public sealed class PackageTests
{
    internal static string Repo
    {
        get
        {
            for (var directory = new DirectoryInfo(AppContext.BaseDirectory); directory is not null; directory = directory.Parent)
                if (File.Exists(Path.Combine(directory.FullName, "DiskBurrow.slnx"))) return directory.FullName;
            throw new InvalidOperationException("Package tests require a repository ancestor.");
        }
    }
    private static readonly Lazy<string> Package = new(BuildPackage);
    private static string BuildPackage()
    {
        var script = Path.Combine(Repo, "scripts", "publish.ps1");
        Assert.True(File.Exists(script), "Portable publisher is missing.");
        var output = Path.Combine(Repo, "work", "package-tests", Guid.NewGuid().ToString("N"));
        var result = RunPublisher("0.1.0", output);
        Assert.True(result.Code == 0, result.Output);
        return output;
    }
    private static (int Code, string Output) RunPublisher(string version, string output)
    {
        var start = new ProcessStartInfo("pwsh") { UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true, RedirectStandardError = true, WorkingDirectory = Repo };
        foreach (var arg in new[] { "-NoProfile", "-File", Path.Combine(Repo, "scripts", "publish.ps1"), "-Version", version, "-OutputDirectory", output }) start.ArgumentList.Add(arg);
        using var process = Process.Start(start)!;
        var stdout = process.StandardOutput.ReadToEndAsync();
        var stderr = process.StandardError.ReadToEndAsync();
        Assert.True(process.WaitForExit(240000), "Publisher timeout.");
        return (process.ExitCode, stdout.GetAwaiter().GetResult() + stderr.GetAwaiter().GetResult());
    }
    private static string Archive => Path.Combine(Package.Value, "DiskBurrow-0.1.0-win-x64.zip");
    [Fact]
    public void PackageContainsOnlyPublishedFiles()
    {
        using var zip = ZipFile.OpenRead(Archive);
        var entries = zip.Entries.Select(e => e.FullName).Order(StringComparer.Ordinal).ToArray();
        var published = File.ReadAllLines(Path.Combine(Package.Value, "publish-outputs.txt"));
        Assert.Equal(published.Concat(["LICENSE", "GETTING-STARTED.txt", "PACKAGE-MANIFEST.json"]).Order(StringComparer.Ordinal), entries);
        Assert.Contains("DiskBurrow.exe", entries);
        Assert.Contains("coreclr.dll", entries);
        Assert.Contains("hostpolicy.dll", entries);
        Assert.Contains("e_sqlite3.dll", entries);
        Assert.Contains("PresentationFramework.dll", entries);
        Assert.Contains("DiskBurrow.dll", entries);
        using var manifest = JsonDocument.Parse(zip.GetEntry("PACKAGE-MANIFEST.json")!.Open());
        Assert.Equal("10.0.12", manifest.RootElement.GetProperty("runtimeVersion").GetString());
        Assert.Equal(published.Order(StringComparer.Ordinal), manifest.RootElement.GetProperty("publishedFiles").EnumerateArray().Select(e => e.GetString()!).Order(StringComparer.Ordinal));
    }
    [Fact]
    public void PackageExcludesLocalHistoryReportsAndTestTrees()
    {
        using var zip = ZipFile.OpenRead(Archive);
        var entries = zip.Entries.Select(e => e.FullName).ToArray();
        Assert.DoesNotContain(entries, e => e.EndsWith(".db", StringComparison.OrdinalIgnoreCase));
        Assert.DoesNotContain(entries, e => e.Contains("scan-report", StringComparison.OrdinalIgnoreCase));
        Assert.DoesNotContain(entries, e => e.EndsWith(".pdb", StringComparison.OrdinalIgnoreCase));
        Assert.DoesNotContain(entries, e => new[] { "work/", "tests/", ".git/", ".superpowers/", "obj/", "bin/" }.Any(p => e.StartsWith(p, StringComparison.OrdinalIgnoreCase)));
        foreach (var entry in zip.Entries)
        {
            using var stream = entry.Open(); using var buffer = new MemoryStream(); stream.CopyTo(buffer);
            var bytes = buffer.ToArray();
            foreach (var forbidden in new[] { Repo, "C:\\Users\\", "C:/Users/", "E:\\CodexWork\\" })
                foreach (var encoding in new[] { Encoding.UTF8, Encoding.Unicode })
                    Assert.True(bytes.AsSpan().IndexOf(encoding.GetBytes(forbidden)) < 0, $"Local binary metadata in {entry.FullName}.");
            foreach (var text in new[] { Encoding.UTF8.GetString(buffer.ToArray()), Encoding.Unicode.GetString(buffer.ToArray()) })
            {
                Assert.DoesNotContain(Repo, text, StringComparison.OrdinalIgnoreCase);
                Assert.DoesNotContain("C:\\Users\\", text, StringComparison.OrdinalIgnoreCase);
                Assert.DoesNotContain("E:\\CodexWork\\", text, StringComparison.OrdinalIgnoreCase);
                Assert.DoesNotContain("C:/Users/", text, StringComparison.OrdinalIgnoreCase);
            }
        }
    }
    [Fact]
    public void ChecksumMatchesArchive()
    {
        using var stream = File.OpenRead(Archive);
        var actualSha256 = Convert.ToHexString(SHA256.HashData(stream)).ToLowerInvariant();
        var parts = File.ReadAllText(Archive + ".sha256").Trim().Split("  ", StringSplitOptions.None);
        Assert.Equal(actualSha256, parts[0]);
        Assert.Equal(Path.GetFileName(Archive), parts[1]);
    }
    [Fact]
    public void ReadmeDescribesActualRules()
    {
        foreach (var name in new[] { "README.md", "README.ru.md" })
        {
            var path = Path.Combine(Repo, name);
            Assert.True(File.Exists(path), "Bilingual release instructions are missing.");
            var text = File.ReadAllText(path);
            foreach (var required in new[] { "DiskBurrow.exe", "7", "Cache_Data", "250", "30", "100", "15", "5", "6", "https://github.com/DinLemon/DiskBurrow/releases", "docs/verification.md" }) Assert.Contains(required, text);
        }
    }
    [Fact]
    public void PublisherRejectsExistingDirectoryAndVersionAsCode()
    {
        Assert.True(File.Exists(Path.Combine(Repo, "scripts", "publish.ps1")), "Portable publisher is missing.");
        using var fixture = new TempTree();
        var sentinel = fixture.FileAt("keep.txt", 12);
        var result = RunPublisher("0.1.0", fixture.Root);
        Assert.NotEqual(0, result.Code);
        Assert.Equal(12, new FileInfo(sentinel).Length);
        var destination = Path.Combine(fixture.Root, "bad-version");
        result = RunPublisher("0.1.0;Write-Output injected", destination);
        Assert.NotEqual(0, result.Code);
        Assert.False(Directory.Exists(destination));
    }
    [Fact]
    public async Task PublisherResolvesRelativeOutputUsingChangedPowerShellLocation()
    {
        using var fixture=new TempTree();
        var relative=$"work/package-tests/relative-{Guid.NewGuid():N}";
        var destination=fixture.DirectoryAt(relative);var sentinel=Path.Combine(destination,"keep.txt");File.WriteAllText(sentinel,"owned fixture sentinel");
        var harness=Path.Combine(fixture.Root,"changed-location.ps1");
        static string Quote(string text)=>"'"+text.Replace("'","''")+"'";
        File.WriteAllText(harness,$"Set-Location -LiteralPath {Quote(fixture.Root)}\n& {Quote(Path.Combine(Repo,"scripts","publish.ps1"))} -Version 0.1.0 -OutputDirectory {Quote(relative)}\n");
        var start=new ProcessStartInfo("pwsh"){UseShellExecute=false,CreateNoWindow=true,RedirectStandardOutput=true,RedirectStandardError=true,WorkingDirectory=Repo};
        foreach(var argument in new[]{"-NoProfile","-File",harness})start.ArgumentList.Add(argument);
        using var process=Process.Start(start)!;var stdout=process.StandardOutput.ReadToEndAsync();var stderr=process.StandardError.ReadToEndAsync();
        await process.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(60));Assert.NotEqual(0,process.ExitCode);
        Assert.Contains("Output directory must be new",(await stdout)+(await stderr));
        Assert.Equal("owned fixture sentinel",File.ReadAllText(sentinel));Assert.False(Directory.Exists(Path.Combine(Repo,relative)));
    }
    [Fact]
    public void PackageResourcesAndApprovedLinksAreAvailable()
    {
        Assert.Contains("DiskBurrow.App.Resources.Strings.en.xaml", typeof(LocalizationService).Assembly.GetManifestResourceNames());
        Assert.Contains("Link.Repository", LocalizationService.ReadKeys("ru"));
        Assert.Contains("Link.Releases", LocalizationService.ReadKeys("en"));
        Assert.Equal("https://github.com/DinLemon/DiskBurrow", ProjectLinks.Repository);
        Assert.Equal("https://github.com/DinLemon/DiskBurrow/releases", ProjectLinks.Releases);
    }
}
