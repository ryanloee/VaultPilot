using Xunit;
using System.Reflection;
using VaultPilot.WinUI.Backend;

namespace VaultPilot.WinUI.Tests;

public class BackendClientTests
{
    /// <summary>
    /// Tests for the static GetBackoffDelay method via reflection.
    /// The method computes exponential backoff: 5s, 10s, 20s, 40s, 60s, 60s...
    /// </summary>
    [Theory]
    [InlineData(1, 5)]   // attempt 1: 5s
    [InlineData(2, 10)]  // attempt 2: 10s
    [InlineData(3, 20)]  // attempt 3: 20s
    [InlineData(4, 40)]  // attempt 4: 40s
    [InlineData(5, 60)]  // attempt 5: 60s (capped at MaxBackoff)
    [InlineData(6, 60)]  // attempt 6: 60s (capped at MaxBackoff)
    public void GetBackoffDelay_ReturnsExpectedDelay(int attempt, int expectedSeconds)
    {
        var method = typeof(BackendClient).GetMethod(
            "GetBackoffDelay",
            BindingFlags.NonPublic | BindingFlags.Static);

        Assert.NotNull(method);

        var result = (TimeSpan)method.Invoke(null, new object[] { attempt })!;
        Assert.Equal(TimeSpan.FromSeconds(expectedSeconds), result);
    }

    [Fact]
    public async Task NewBackendClient_IsNotConnected()
    {
        await using var client = new BackendClient();
        Assert.False(client.IsConnected);
    }

    [Fact]
    public async Task GetStderrTail_EmptyClient_ReturnsEmpty()
    {
        await using var client = new BackendClient();
        var tail = client.GetStderrTail();
        Assert.Equal(string.Empty, tail);
    }
}
