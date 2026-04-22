using Forge.Gui.Core.Models;
using Forge.Gui.Mock;
using Xunit;

namespace Forge.Gui.Tests;

public class MockClientTests
{
    [Fact]
    public async Task StatusReturnsFixtures()
    {
        var client = new MockForgeClient();
        var status = await client.GetStatusAsync();
        Assert.NotEmpty(status.Changes);
        Assert.Contains(status.Changes, c => c.Kind == FileChangeKind.StagedModified);
    }

    [Fact]
    public async Task CommitMovesStagedToLog()
    {
        var client = new MockForgeClient();
        var before = (await client.GetLogAsync(10, null)).Count;
        await client.CommitAsync("test commit");
        var after = (await client.GetLogAsync(10, null)).Count;
        Assert.Equal(before + 1, after);

        var status = await client.GetStatusAsync();
        Assert.DoesNotContain(status.Changes, c =>
            c.Kind is FileChangeKind.StagedNew or FileChangeKind.StagedModified or FileChangeKind.StagedDeleted);
    }

    [Fact]
    public async Task LockEventFiresOnAcquire()
    {
        var client = new MockForgeClient();
        using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(1));

        var task = Task.Run(async () =>
        {
            await foreach (var evt in client.SubscribeLockEventsAsync(cts.Token))
                return evt;
            return null;
        });

        await client.LockAsync("Content/Test.uasset", "unit test");
        var evt = await task;
        Assert.NotNull(evt);
        Assert.Equal(LockEventKind.Acquired, evt!.Kind);
        Assert.Equal("Content/Test.uasset", evt.Lock.Path);
    }
}
