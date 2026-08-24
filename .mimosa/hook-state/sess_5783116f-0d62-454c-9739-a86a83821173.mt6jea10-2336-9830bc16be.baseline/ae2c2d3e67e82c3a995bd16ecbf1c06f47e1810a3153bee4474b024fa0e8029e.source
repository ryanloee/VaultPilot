// Regression test for #3512: parse_image_token_at handles URLs with parentheses.
//
// The bug: `parse_image_token_at` used `paren_start.find(')')` which finds
// the FIRST `)` — truncating URLs/filenames that contain `(` before the
// actual closing `)`, e.g. Windows screenshot filenames like
// "Screenshot (1).png".
//
// After fix: parenthesis depth tracking ensures only the *matching* closing
// `)` is recognized.

#[cfg(test)]
mod tests {
    use crate::file_parsing::collect_image_references;

    #[test]
    fn issue_3512_windows_screenshot_filename_with_parentheses() {
        // Windows default screenshot: "Screenshot (1).png"
        let md = "![alt text](Screenshot (1).png)";
        let refs = collect_image_references(md);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].url, "Screenshot (1).png");
        assert!(!refs[0].is_wikilink);
    }

    #[test]
    fn issue_3512_url_with_multiple_parentheses() {
        // URLs with nested parents are rare but possible
        let md = "![pic](https://example.com/path_(v2)/image.png)";
        let refs = collect_image_references(md);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].url, "https://example.com/path_(v2)/image.png");
    }

    #[test]
    fn issue_3512_wikilink_with_parentheses_unaffected() {
        // Wikilinks were never affected — verify they still work
        let md = "![[Screenshot (1).png]]";
        let refs = collect_image_references(md);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].url, "Screenshot (1).png");
        assert!(refs[0].is_wikilink);
    }

    #[test]
    fn issue_3512_multiple_images_with_parentheses() {
        let md = "![a](pic (1).png) and ![b](pic (2).png)";
        let refs = collect_image_references(md);
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].url, "pic (1).png");
        assert_eq!(refs[1].url, "pic (2).png");
    }

    #[test]
    fn issue_3512_standard_url_still_works() {
        // Regression: plain URL without parents still parses correctly
        let md = "![cat](assets/cat.png)";
        let refs = collect_image_references(md);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].url, "assets/cat.png");
    }
}
