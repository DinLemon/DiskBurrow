using DiskBurrow.App.Services;
namespace DiskBurrow.Tests;
public class SingleInstanceTests
{
    [Fact] public async Task SecondInstanceActivatesFirst()
    {
        var activated = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var name = "DiskBurrow-test-" + Guid.NewGuid().ToString("N");
        await using var first = new SingleInstanceService(name, () => activated.TrySetResult());
        await using var second = new SingleInstanceService(name, () => {});
        Assert.True(await first.TryBecomePrimaryAsync()); Assert.False(await second.TryBecomePrimaryAsync());
        await activated.Task.WaitAsync(TimeSpan.FromSeconds(5));
    }
}
