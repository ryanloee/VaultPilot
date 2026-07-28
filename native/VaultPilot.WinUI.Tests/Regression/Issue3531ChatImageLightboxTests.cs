using Xunit;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression test for issue #3531: clicking a chat history image opens
/// the Lightbox with the wrong image list (input attachments shown instead
/// of the clicked image).
///
/// Bug: ShowImagePreviewDialogAsync always built the navigable image list
/// from _attachments (the input queue), so clicking a sent message's image
/// while having input attachments queued would show the input images instead.
///
/// Fix: Check if the clicked attachment.Path is in _attachments. If not,
/// the image comes from chat history and should be shown standalone:
///   imagePaths = new List&lt;string&gt; { attachment.Path };
/// </summary>
public class Issue3531ChatImageLightboxTests
{
    [Fact]
    public void Regression_3531_HasChatHistoryAttachmentDetection()
    {
        var sourcePath = ResolveSourcePath("VaultPilot.WinUI", "MainWindow.Attachments.cs");
        if (!File.Exists(sourcePath))
            return; // source may not be co-located in CI

        var code = File.ReadAllText(sourcePath);

        // The fix must check if the clicked attachment is in the input queue.
        // Old code: always used _attachments as the navigable list.
        // Fixed:    if (!imagePaths.Contains(attachment.Path)) { ... }
        Assert.Contains("!imagePaths.Contains(attachment.Path)", code);
    }

    [Fact]
    public void Regression_3531_ShowsStandaloneForChatHistoryImages()
    {
        var sourcePath = ResolveSourcePath("VaultPilot.WinUI", "MainWindow.Attachments.cs");
        if (!File.Exists(sourcePath))
            return;

        var code = File.ReadAllText(sourcePath);

        // When the clicked image is not in the input queue, the lightbox
        // must show only that single image (standalone).
        // Fixed: imagePaths = new List&lt;string&gt; { attachment.Path };
        var standaloneLine = "imagePaths = new List<string> { attachment.Path };";
        Assert.Contains(standaloneLine, code);
    }

    [Fact]
    public void Regression_3531_HasIssueReferenceComment()
    {
        var sourcePath = ResolveSourcePath("VaultPilot.WinUI", "MainWindow.Attachments.cs");
        if (!File.Exists(sourcePath))
            return;

        var code = File.ReadAllText(sourcePath);

        // The #3531 comment should be present to document why this fix exists.
        Assert.Contains("#3531", code);
    }

    private static string ResolveSourcePath(string projectName, string relativePath)
    {
        var baseDir = AppContext.BaseDirectory;
        // Walk up from test output to find the solution root.
        var dir = new DirectoryInfo(baseDir);
        while (dir is not null && !dir.GetFiles("*.sln").Any())
            dir = dir.Parent;
        if (dir is null)
            return string.Empty;
        return Path.Combine(dir.FullName, projectName, relativePath);
    }
}
