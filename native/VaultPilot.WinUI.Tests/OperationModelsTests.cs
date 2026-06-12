using System.Text.Json;
using VaultPilot.WinUI.Models;

namespace VaultPilot.WinUI.Tests;

public class OperationModelsTests
{
    [Fact]
    public void ImportResult_PropertiesPreserved()
    {
        var result = new ImportResult(
            Imported: 42,
            Skipped: 5,
            Errors: new List<string> { "file1.md: parse error", "file2.md: permission denied" });

        Assert.Equal(42UL, result.Imported);
        Assert.Equal(5UL, result.Skipped);
        Assert.Equal(2, result.Errors.Count);
    }

    [Fact]
    public void ImportResult_EmptyErrors()
    {
        var result = new ImportResult(
            Imported: 100,
            Skipped: 0,
            Errors: Array.Empty<string>());

        Assert.Equal(100UL, result.Imported);
        Assert.Equal(0UL, result.Skipped);
        Assert.Empty(result.Errors);
    }

    [Fact]
    public void ImportResult_JsonRoundTrip()
    {
        var original = new ImportResult(
            Imported: 10,
            Skipped: 3,
            Errors: new List<string> { "error1" });

        var json = JsonSerializer.Serialize(original);
        var deserialized = JsonSerializer.Deserialize<ImportResult>(json);

        Assert.NotNull(deserialized);
        Assert.Equal(original.Imported, deserialized.Imported);
        Assert.Equal(original.Skipped, deserialized.Skipped);
        Assert.Single(deserialized.Errors);
        Assert.Equal("error1", deserialized.Errors[0]);
    }

    [Fact]
    public void IndexStats_PropertiesPreserved()
    {
        var stats = new IndexStats(
            Scanned: 500,
            Indexed: 480,
            Removed: 10);

        Assert.Equal(500UL, stats.Scanned);
        Assert.Equal(480UL, stats.Indexed);
        Assert.Equal(10UL, stats.Removed);
    }

    [Fact]
    public void IndexStats_JsonRoundTrip()
    {
        var original = new IndexStats(Scanned: 1000, Indexed: 950, Removed: 25);

        var json = JsonSerializer.Serialize(original);
        var deserialized = JsonSerializer.Deserialize<IndexStats>(json);

        Assert.NotNull(deserialized);
        Assert.Equal(original, deserialized);
    }
}
