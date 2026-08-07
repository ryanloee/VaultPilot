using System.Collections.Generic;
using Xunit;
using VaultPilot.WinUI.Utils;

namespace VaultPilot.WinUI.Tests.Regression;

/// <summary>
/// Regression tests for issue #3936: FindNoteReferences mixed coordinate spaces.
///
/// Bug: #3932 moved the boundary check onto lowerText (the ToLowerInvariant copy)
/// but still emitted NoteRef with Start = lowerText index and End = Start +
/// title.Length (original title length). When a character earlier in the text
/// expands under ToLowerInvariant (e.g. U+0130 'İ' → "i̇"), the lowerText index
/// drifts past the original-text index, so Start/End no longer bound the title
/// in the ORIGINAL text — SplitLineByNoteRefs then slices the wrong span,
/// duplicates characters, and in multi-ref cases throws
/// ArgumentOutOfRangeException (cursor &gt; line.Length).
///
/// Fix: FindNoteReferences builds a lowerText→original offset map when the two
/// lengths differ and translates every match back to original-text coordinates
/// before constructing NoteRef(Start, End).
///
/// NOTE: _sortedTitlesCache is a STATIC cache keyed by the titleMap REFERENCE;
/// every test passes its own fresh Dictionary to avoid stale-title leakage.
/// </summary>
public class Issue3936NoteRefsCoordinateSpaceTests
{
    [Fact]
    public void Regression_3936_TitleAfterExpandingChar_StartEndBoundOriginalText()
    {
        // U+0130 'İ' lowercases to "i̇" (2 chars), so lowerText is 1 longer than text.
        // Original:  "İşte AI"  (len 7)   indices: İ=0 ş=1 t=2 e=3 (space)=4 A=5 I=6
        // lowerText: "i̇şte ai" (len 8)   indices: i=0 ̇=1 ş=2 t=3 e=4 (space)=5 a=6 i=7
        // "ai" matches lowerText at idx=6 → must translate back to original idx=5.
        // Pre-fix the code emitted Start=6 (lowerText idx used as text idx), which
        // under-sliced by one and End=8 (> text.Length=7) → slicing crashed.
        var text = "İşte AI";
        var refs = NoteRefs.FindNoteReferences(text,
            new Dictionary<string, string> { ["AI"] = "note-ai" });

        var single = Assert.Single(refs);
        Assert.Equal("AI", single.Title);
        // Start must be 5 in the ORIGINAL text, not 6 in lowerText.
        Assert.Equal(5, single.Start);
        Assert.Equal(7, single.End);
        // The emitted span must exactly bound the matched title in the original.
        Assert.Equal("AI", text[single.Start..single.End]);
    }

    [Fact]
    public void Regression_3936_SplitLineByNoteRefs_NoCharDuplication()
    {
        // #3936 symptom: SplitLineByNoteRefs used the wrong Start, so slicing
        // produced "İşte A" + hyperlinked "AI" where the 'A' was duplicated /
        // the tail was mis-split. Verify the round-trip reassembles the input.
        var text = "İşte AI";
        var refs = NoteRefs.FindNoteReferences(text,
            new Dictionary<string, string> { ["AI"] = "note-ai" });

        var segments = NoteRefs.SplitLineByNoteRefs(text, refs);

        // Reassemble non-ref text + ref titles; must equal the original text.
        var rebuilt = new System.Text.StringBuilder();
        foreach (var seg in segments)
        {
            rebuilt.Append(seg.IsNoteRef ? seg.Title : seg.Text);
        }
        Assert.Equal(text, rebuilt.ToString());
    }

    [Fact]
    public void Regression_3936_MultipleRefsAfterExpandingChar_NoOutOfRange()
    {
        // Two refs separated by an expanding char earlier in the text —
        // pre-fix the first ref's End could exceed line.Length, pushing the
        // cursor past the line and making line[cursor..ref.Start] throw.
        var text = "İAI ok BI";
        var refs = NoteRefs.FindNoteReferences(text,
            new Dictionary<string, string>
            {
                ["AI"] = "note-ai",
                ["BI"] = "note-bi",
            });

        // Both titles detected, ordered by position.
        Assert.Equal(2, refs.Count);
        Assert.Equal("AI", refs[0].Title);
        Assert.Equal("BI", refs[1].Title);

        // Every Start/End must be valid in the ORIGINAL text and bound the title.
        foreach (var r in refs)
        {
            Assert.InRange(r.Start, 0, text.Length);
            Assert.InRange(r.End, r.Start, text.Length);
            Assert.Equal(r.Title, text[r.Start..r.End]);
        }

        // Round-trip through the splitter without throwing.
        var segments = NoteRefs.SplitLineByNoteRefs(text, refs);
        var rebuilt = new System.Text.StringBuilder();
        foreach (var seg in segments)
        {
            rebuilt.Append(seg.IsNoteRef ? seg.Title : seg.Text);
        }
        Assert.Equal(text, rebuilt.ToString());
    }

    [Fact]
    public void Regression_3936_LigatureExpandingChar_TitleCoordinatesStable()
    {
        // U+FB00 'ﬀ' (LATIN SMALL LIGATURE FF) lowercases to "ff" (2 chars).
        // A title following it must still resolve to its original-text span.
        var text = "ﬀx AI";
        var refs = NoteRefs.FindNoteReferences(text,
            new Dictionary<string, string> { ["AI"] = "note-ai" });

        var single = Assert.Single(refs);
        Assert.Equal("AI", single.Title);
        Assert.Equal(text.IndexOf("AI", System.StringComparison.Ordinal), single.Start);
        Assert.Equal(single.Start + 2, single.End);
        Assert.Equal("AI", text[single.Start..single.End]);
    }

    [Fact]
    public void Regression_3936_NoExpandingChar_PathUnchanged()
    {
        // Sanity: when text has no expanding characters, lengths are equal and
        // the map is skipped (identity) — coordinates must match the pre-#3936
        // behaviour exactly.
        var refs = NoteRefs.FindNoteReferences("hello AI world",
            new Dictionary<string, string> { ["AI"] = "note-ai" });

        var single = Assert.Single(refs);
        Assert.Equal(6, single.Start);
        Assert.Equal(8, single.End);
    }
}
