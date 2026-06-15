using VaultPilot.WinUI.Views;

namespace VaultPilot.WinUI.Tests;

public class NotesViewUtilityTests
{
    // ── FormatRelativeTime ──

    [Fact]
    public void FormatRelativeTime_EmptyString_ReturnsEmpty()
    {
        Assert.Equal("", NotesView.FormatRelativeTime(""));
    }

    [Fact]
    public void FormatRelativeTime_NullString_ReturnsEmpty()
    {
        Assert.Equal("", NotesView.FormatRelativeTime(null!));
    }

    [Fact]
    public void FormatRelativeTime_InvalidDate_ReturnsOriginal()
    {
        Assert.Equal("not-a-date", NotesView.FormatRelativeTime("not-a-date"));
    }

    [Fact]
    public void FormatRelativeTime_RecentTime_Returns刚刚()
    {
        var recent = DateTimeOffset.Now.AddSeconds(-30).ToString("O");
        Assert.Equal("刚刚", NotesView.FormatRelativeTime(recent));
    }

    [Fact]
    public void FormatRelativeTime_MinutesAgo_ReturnsMinutes()
    {
        var minutesAgo = DateTimeOffset.Now.AddMinutes(-5).ToString("O");
        Assert.Equal("5分钟前", NotesView.FormatRelativeTime(minutesAgo));
    }

    [Fact]
    public void FormatRelativeTime_HoursAgo_ReturnsHours()
    {
        var hoursAgo = DateTimeOffset.Now.AddHours(-3).ToString("O");
        Assert.Equal("3小时前", NotesView.FormatRelativeTime(hoursAgo));
    }

    [Fact]
    public void FormatRelativeTime_DaysAgo_ReturnsDays()
    {
        var daysAgo = DateTimeOffset.Now.AddDays(-2).ToString("O");
        Assert.Equal("2天前", NotesView.FormatRelativeTime(daysAgo));
    }

    [Fact]
    public void FormatRelativeTime_OldDate_ReturnsFormattedDate()
    {
        var oldDate = new DateTimeOffset(2024, 1, 15, 10, 30, 0, TimeSpan.Zero).ToString("O");
        var result = NotesView.FormatRelativeTime(oldDate);
        Assert.Matches(@"\d{4}-\d{2}-\d{2}", result);
    }
}
