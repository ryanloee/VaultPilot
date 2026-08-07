using System.Collections.Generic;
using Xunit;
using VaultPilot.WinUI.Utils;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression tests for issue #3932: NoteRefs.FindNoteReferences short-circuited
/// on text.Length &lt; 4 while the cached-title filter kept titles with
/// t.Length &gt;= 2, so a 3-char text (e.g. "AI!") could never match a 2-char
/// title (e.g. "AI"). MainWindow.AppendNoteRefText had the same early return.
///
/// Fix: both thresholds lowered to text.Length &lt; 2 (a 1-char text can never
/// contain a 2-char-minimum title), and the boundary check now reads chars
/// from lowerText (the ToLowerInvariant copy that the match index comes from)
/// instead of the original text, so Unicode chars that expand on lowercasing
/// (e.g. U+0130 'İ' -> "i̇") cannot misalign the index.
///
/// NOTE: _sortedTitlesCache is a STATIC cache keyed by the titleMap REFERENCE.
/// It only rebuilds when the map instance changes, so every test passes its
/// own fresh Dictionary instance — otherwise stale titles from a previous
/// test would leak into this one.
/// </summary>
public class Issue3932NoteRefsShortTitleTests
{
    [Fact]
    public void Regression_3932_TwoCharTitle_InThreeCharText_IsDetected()
    {
        // The exact bug scenario: "AI!" (3 chars) previously returned early
        // (text.Length < 4) and never matched the 2-char title "AI".
        var refs = NoteRefs.FindNoteReferences("AI!", new Dictionary<string, string> { ["AI"] = "note-ai" });

        var single = Assert.Single(refs);
        Assert.Equal("AI", single.Title);
        Assert.Equal(0, single.Start);
        Assert.Equal(2, single.End);
    }

    [Fact]
    public void Regression_3932_TwoCharTitle_InTwoCharText_IsDetected()
    {
        var refs = NoteRefs.FindNoteReferences("AI", new Dictionary<string, string> { ["AI"] = "note-ai" });

        var single = Assert.Single(refs);
        Assert.Equal("AI", single.Title);
        Assert.Equal(0, single.Start);
        Assert.Equal(2, single.End);
    }

    [Fact]
    public void Regression_3932_OneCharText_ReturnsEmpty()
    {
        // Threshold: a 1-char text can never contain a 2-char title.
        var refs = NoteRefs.FindNoteReferences("A", new Dictionary<string, string> { ["AI"] = "note-ai" });

        Assert.Empty(refs);
    }

    [Fact]
    public void Regression_3932_ThreeCharTitle_InLongerText_StillMatches()
    {
        // Long-title path unchanged: a 3-char title still matches in a 5-char
        // text. (Leading space keeps the ASCII word-boundary check happy — a
        // Latin letter directly before the title would correctly reject it,
        // e.g. "xABC!" is treated as a word containing "ABC".)
        var refs = NoteRefs.FindNoteReferences(" ABC!", new Dictionary<string, string> { ["ABC"] = "note-abc" });

        var single = Assert.Single(refs);
        Assert.Equal("ABC", single.Title);
        Assert.Equal(1, single.Start);
        Assert.Equal(4, single.End);
    }

    [Fact]
    public void Regression_3932_AsciiTitle_NotMatched_InsideLatinWord()
    {
        // "React" must not match inside "Reactor" (Latin word boundary).
        var refs = NoteRefs.FindNoteReferences("Reactor", new Dictionary<string, string> { ["React"] = "note-react" });

        Assert.Empty(refs);
    }

    [Fact]
    public void Regression_3932_AsciiTitle_Matched_AtPunctuationBoundary()
    {
        var refs = NoteRefs.FindNoteReferences("(React)", new Dictionary<string, string> { ["React"] = "note-react" });

        var single = Assert.Single(refs);
        Assert.Equal("React", single.Title);
        Assert.Equal(1, single.Start);
        Assert.Equal(6, single.End);
    }

    [Fact]
    public void Regression_3932_EmptyText_ReturnsEmpty()
    {
        var refs = NoteRefs.FindNoteReferences("", new Dictionary<string, string> { ["AI"] = "note-ai" });

        Assert.Empty(refs);
    }

    [Fact]
    public void Regression_3932_EmptyMap_ReturnsEmpty()
    {
        // Guard behavior unchanged: a title map with no entries yields no refs.
        var refs = NoteRefs.FindNoteReferences("AI!", new Dictionary<string, string>());

        Assert.Empty(refs);
    }

    [Fact]
    public void Regression_3932_CjkTitle_Matches_WithRelaxedBoundary()
    {
        // Non-ASCII (CJK) titles skip the Latin word-boundary check entirely.
        var refs = NoteRefs.FindNoteReferences("看笔记!", new Dictionary<string, string> { ["笔记"] = "note-cn" });

        var single = Assert.Single(refs);
        Assert.Equal("笔记", single.Title);
        Assert.Equal(1, single.Start);
        Assert.Equal(3, single.End);
    }
}
