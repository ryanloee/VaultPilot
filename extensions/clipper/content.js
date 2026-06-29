/**
 * VaultPilot Clipper — Content Script
 *
 * Injected into every page. Listens for "getPageContent" messages from the
 * background service worker, extracts article content, converts to Markdown,
 * and returns the result.
 */
/**
 * Minimal HTML-to-Markdown converter (standalone, no external deps).
 */
function stripTags(html) {
    return html.replace(/<[^>]*>/g, '')
        .replace(/&amp;/g, '&')
        .replace(/&lt;/g, '<')
        .replace(/&gt;/g, '>')
        .replace(/&quot;/g, '"')
        .replace(/&#39;/g, "'")
        .replace(/&nbsp;/g, ' ')
        .trim();
}
function basicHtmlToMarkdown(html) {
    let text = html
        // Remove scripts and styles
        .replace(/<script[^>]*>[\s\S]*?<\/script>/gi, '')
        .replace(/<style[^>]*>[\s\S]*?<\/style>/gi, '')
        // Headers
        .replace(/<h1[^>]*>[\s\S]*?<\/h1>/gi, (m) => `# ${stripTags(m)}\n\n`)
        .replace(/<h2[^>]*>[\s\S]*?<\/h2>/gi, (m) => `## ${stripTags(m)}\n\n`)
        .replace(/<h3[^>]*>[\s\S]*?<\/h3>/gi, (m) => `### ${stripTags(m)}\n\n`)
        .replace(/<h4[^>]*>[\s\S]*?<\/h4>/gi, (m) => `#### ${stripTags(m)}\n\n`)
        // Bold / italic
        .replace(/<strong[^>]*>([\s\S]*?)<\/strong>/gi, '**$1**')
        .replace(/<b[^>]*>([\s\S]*?)<\/b>/gi, '**$1**')
        .replace(/<em[^>]*>([\s\S]*?)<\/em>/gi, '*$1*')
        .replace(/<i[^>]*>([\s\S]*?)<\/i>/gi, '*$1*')
        // Links
        .replace(/<a[^>]*href="([^"]*)"[^>]*>([\s\S]*?)<\/a>/gi, '[$2]($1)')
        // Images
        .replace(/<img[^>]*src="([^"]*)"[^>]*alt="([^"]*)"[^>]*>/gi, '![$2]($1)')
        .replace(/<img[^>]*src="([^"]*)"[^>]*>/gi, '![]($1)')
        // Code blocks
        .replace(/<pre[^>]*>[\s\S]*?<\/pre>/gi, (m) => `\`\`\`\n${stripTags(m)}\n\`\`\`\n\n`)
        .replace(/<code[^>]*>([\s\S]*?)<\/code>/gi, '`$1`')
        // Blockquotes
        .replace(/<blockquote[^>]*>([\s\S]*?)<\/blockquote>/gi, (m) => {
        const inner = stripTags(m);
        return inner.split('\n').map((l) => `> ${l}`).join('\n') + '\n\n';
    })
        // Lists
        .replace(/<li[^>]*>([\s\S]*?)<\/li>/gi, '- $1\n')
        // Paragraphs
        .replace(/<p[^>]*>([\s\S]*?)<\/p>/gi, '$1\n\n')
        // Horizontal rules
        .replace(/<hr[^>]*>/gi, '---\n\n')
        // Line breaks
        .replace(/<br\s*\/?>/gi, '\n')
        // Remove remaining HTML tags
        .replace(/<[^>]*>/g, '')
        // Decode common entities
        .replace(/&amp;/g, '&')
        .replace(/&lt;/g, '<')
        .replace(/&gt;/g, '>')
        .replace(/&quot;/g, '"')
        .replace(/&#39;/g, "'")
        .replace(/&nbsp;/g, ' ')
        // Clean up excessive whitespace
        .replace(/\n{3,}/g, '\n\n')
        .trim();
    return text;
}
/**
 * Extract page content: try semantic containers, fallback to body minus noise.
 */
function extractPageContent() {
    const title = document.title || window.location.hostname;
    const url = window.location.href;
    // Try common article containers
    const articleEl = document.querySelector('article') ||
        document.querySelector('[role="main"]') ||
        document.querySelector('main') ||
        document.querySelector('.post-content') ||
        document.querySelector('.entry-content') ||
        document.querySelector('.article-content');
    let contentHtml;
    if (articleEl) {
        contentHtml = articleEl.innerHTML;
    }
    else {
        // Fallback: use body content, excluding nav, footer, header, aside
        const bodyClone = document.body.cloneNode(true);
        const excludes = bodyClone.querySelectorAll('nav, footer, header, aside, .sidebar, .nav, .footer, .header, script, style, noscript');
        excludes.forEach(el => el.remove());
        contentHtml = bodyClone.innerHTML;
    }
    const markdown = basicHtmlToMarkdown(contentHtml);
    const excerpt = stripTags(contentHtml).substring(0, 200).replace(/\s+/g, ' ').trim();
    return { title, url, content: markdown, excerpt };
}
// Listen for messages from the background service worker
chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
    if (message.action === 'getPageContent') {
        try {
            const content = extractPageContent();
            sendResponse(content);
        }
        catch (error) {
            sendResponse({ error: String(error) });
        }
    }
    return true;
});
// Cache content on page load so popup can retrieve it quickly
const csCachedContent = extractPageContent();
chrome.runtime.sendMessage({ action: 'cacheContent', data: csCachedContent }).catch(() => {
    // Background may not be ready yet; that's fine
});
export {};
