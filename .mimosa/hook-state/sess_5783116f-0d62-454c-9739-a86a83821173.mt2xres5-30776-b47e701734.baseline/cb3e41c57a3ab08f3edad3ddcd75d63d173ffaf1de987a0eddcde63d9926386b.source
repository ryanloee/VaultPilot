// VaultPilot Clipper — Content Script
// Runs on every page to extract article content when requested.

// Listen for messages from the popup or background script
chrome.runtime.onMessage.addListener((request, sender, sendResponse) => {
  if (request.action === 'extractContent') {
    const result = extractPageContent();
    sendResponse(result);
  }
  // Keep the message channel open for async response
  return true;
});

/**
 * Extract readable content from the current page.
 * Uses a simplified Readability-like approach:
 * 1. Try <article> element first
 * 2. Fall back to <main>, then <body>
 * 3. Remove script, style, nav, footer, header elements
 */
function extractPageContent() {
  const url = window.location.href;
  const title = document.title;
  const description = getMetaContent('description');
  const author = getMetaContent('author');
  const siteName = getMetaContent('og:site_name') || getMetaContent('twitter:site') || new URL(url).hostname;

  // Clone the document to avoid modifying the live DOM
  const clone = document.body.cloneNode(true);

  // Remove non-content elements
  const removals = clone.querySelectorAll(
    'script, style, noscript, iframe, svg, ' +
    'nav, footer, header, aside, ' +
    '.sidebar, .nav, .footer, .header, .menu, .ad, .advertisement, .ads, ' +
    '[role="navigation"], [role="banner"], [role="contentinfo"], ' +
    '.social-share, .comments, .comment'
  );
  removals.forEach(el => el.remove());

  // Try to find the main content area
  let content = null;
  const article = document.querySelector('article');
  const main = document.querySelector('main');
  const contentDiv = document.querySelector('[role="main"], .content, .post-content, .entry-content, .article-content');

  if (article) {
    content = article.cloneNode(true);
  } else if (main) {
    content = main.cloneNode(true);
  } else if (contentDiv) {
    content = contentDiv.cloneNode(true);
  } else {
    content = clone;
  }

  // Clean remaining non-content elements from the selected content
  const innerRemovals = content.querySelectorAll(
    'script, style, noscript, iframe, svg, button, ' +
    '.ad, .advertisement, .ads, .social-share, .comments'
  );
  innerRemovals.forEach(el => el.remove());

  // Extract text content
  const bodyText = content.textContent
    .replace(/\\s+/g, ' ')
    .replace(/\\n{3,}/g, '\\n\\n')
    .trim();

  // Truncate if too long
  const maxLength = 50000;
  const truncated = bodyText.length > maxLength
    ? bodyText.substring(0, maxLength) + '\\n\\n[内容已截断，原文超过 ' + maxLength + ' 字符]'
    : bodyText;

  return {
    title,
    url,
    description,
    author,
    siteName,
    bodyText: truncated,
    textLength: bodyText.length,
    extractedAt: new Date().toISOString()
  };
}

function getMetaContent(name) {
  // Try standard name and property attributes
  const selectors = [
    `meta[name="${name}"]`,
    `meta[property="${name}"]`,
    `meta[name="og:${name}"]`,
    `meta[property="og:${name}"]`,
    `meta[name="twitter:${name}"]`,
    `meta[property="twitter:${name}"]`
  ];

  for (const sel of selectors) {
    const el = document.querySelector(sel);
    if (el && el.content) return el.content;
  }
  return '';
}
