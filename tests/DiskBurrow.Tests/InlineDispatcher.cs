using DiskBurrow.App.Services;
namespace DiskBurrow.Tests;
internal sealed class InlineDispatcher : IUiDispatcher { public Task InvokeAsync(Action action) { action();return Task.CompletedTask; } }
