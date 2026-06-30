using System;
using System.Collections.Generic;
using System.Linq;

namespace VaultPilot.WinUI.Utils;

/// <summary>
/// Note reference detection utility for Chat-Note bidirectional reference (#2035).
/// Detects both [[wikilink]] patterns and auto-detected note titles in text,
/// enabling clickable note references in AI chat responses.
/// </summary>
public static class NoteRefs
{
    /// <summary>
    /// A detected note reference in text with position info.
    /// </summary>
    public readonly record struct NoteRef(string Title, string NoteId, int Start, int End);

    /// <summary>
    /// A text segment split around note references.
    /// </summary>
    public readonly record struct Segment(string Text, bool IsNoteRef, string? NoteId, string? Title);

    /// <summary>
    /// Finds [[wikilink]] patterns in text.
    /// Returns list of (Title, Start, End) for each wikilink in order of appearance.
    /// </summary>
    public static List<(string Title, int Start, int End)> FindWikilinks(string text)
    {
        var results = new List<(string Title, int Start, int End)>();
        if (string.IsNullOrEmpty(text))
            return results;

        var index = 0;
        while (index < text.Length)
        {
            var open = text.IndexOf("[[", index, StringComparison.Ordinal);
            if (open < 0)
                break;

            var close = text.IndexOf("]]", open + 2, StringComparison.Ordinal);
            if (close < 0)
                break;

            var title = text[(open + 2)..close].Trim();
            if (!string.IsNullOrEmpty(title))
            {
                results.Add((title, open, close + 2));
            }

            index = close + 2;
        }

        return results;
    }

    /// <summary>
    /// Find auto-detected note title references in text using greedy longest-match.
    /// Titles are checked longest-first to avoid substring false positives.
    /// Boundary checks prevent matching inside Latin words (e.g. "React" != "Reactor").
    /// </summary>
    /// <param name="text">Raw text to scan</param>
    /// <param name="titleMap">title -> noteId dictionary (from LoadNoteTitleMapAsync)</param>
    /// <returns>Sorted list of NoteRef, empty if none</returns>
    public static List<NoteRef> FindNoteReferences(string text, Dictionary<string, string> titleMap)
    {
        var refs = new List<NoteRef>();
        if (titleMap is null || titleMap.Count == 0 || string.IsNullOrEmpty(text))
            return refs;

        var lowerText = text.ToLowerInvariant();

        // Sort titles by length descending for greedy longest-match
        var sortedTitles = titleMap.Keys
            .Where(t => !string.IsNullOrWhiteSpace(t))
            .OrderByDescending(t => t.Length)
            .ToArray();

        foreach (var title in sortedTitles)
        {
            var noteId = titleMap[title];
            var lowerTitle = title.ToLowerInvariant();
            var searchFrom = 0;

            while (true)
            {
                var idx = lowerText.IndexOf(lowerTitle, searchFrom, StringComparison.Ordinal);
                if (idx < 0)
                    break;

                // Boundary check: for ASCII-only titles, ensure the match is not
                // in the middle of a Latin word (e.g. "React" in "Reactor" -> no match).
                // For non-ASCII (CJK etc.) titles, boundary check is relaxed because
                // CJK characters don't form compound words like Latin does.
                var isAsciiOnly = title.All(c => c <= 0x7f);
                var afterIdx = idx + title.Length;
                var afterChar = afterIdx < text.Length ? text[afterIdx] : ' ';
                var validAfter = !isAsciiOnly || (!char.IsLetterOrDigit(afterChar) && afterChar != '_');
                if (!validAfter)
                {
                    searchFrom = idx + 1;
                    continue;
                }
                var beforeIdx = idx - 1;
                var beforeChar = beforeIdx >= 0 ? text[beforeIdx] : ' ';
                var validBefore = !isAsciiOnly || (!char.IsLetterOrDigit(beforeChar) && beforeChar != '_');
                if (!validBefore)
                {
                    searchFrom = idx + 1;
                    continue;
                }

                // Avoid overlapping with already-found refs
                var overlaps = false;
                foreach (var existing in refs)
                {
                    if ((idx >= existing.Start && idx < existing.End) ||
                        (existing.Start >= idx && existing.Start < idx + title.Length))
                    {
                        overlaps = true;
                        break;
                    }
                }

                if (!overlaps)
                {
                    refs.Add(new NoteRef(title, noteId, idx, idx + title.Length));
                }

                searchFrom = idx + 1;
            }
        }

        refs.Sort((a, b) => a.Start.CompareTo(b.Start));
        return refs;
    }

    /// <summary>
    /// Splits a line of text into segments around note references.
    /// Non-ref segments return IsNoteRef=false; ref segments return IsNoteRef=true
    /// with the noteId and title populated.
    /// </summary>
    public static List<Segment> SplitLineByNoteRefs(string line, List<NoteRef> refs)
    {
        var segments = new List<Segment>();
        if (refs is null || refs.Count == 0)
        {
            segments.Add(new Segment(line, false, null, null));
            return segments;
        }

        var cursor = 0;
        foreach (var ref_ in refs)
        {
            if (ref_.Start > cursor)
            {
                segments.Add(new Segment(line[cursor..ref_.Start], false, null, null));
            }

            segments.Add(new Segment(ref_.Title, true, ref_.NoteId, ref_.Title));
            cursor = ref_.End;
        }

        if (cursor < line.Length)
        {
            segments.Add(new Segment(line[cursor..], false, null, null));
        }

        return segments;
    }
}
