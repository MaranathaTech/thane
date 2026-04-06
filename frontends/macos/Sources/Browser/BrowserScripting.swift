import Foundation

/// JavaScript snippets for browser automation and Vimium-style navigation.
///
/// These mirror the cross-platform snippets from `thane_browser::scripting`.
enum BrowserScripting {

    // MARK: - Vimium link hints

    /// Inject hint labels on all visible clickable elements.
    /// Returns JSON array of `{label, selector}` objects.
    static let showHintsJS = """
    (function() {
        document.querySelectorAll('.__thane_hint').forEach(e => e.remove());
        const selectors = 'a[href], button, input, select, textarea, [role="button"], [role="link"], [onclick], [tabindex]:not([tabindex="-1"])';
        const elements = Array.from(document.querySelectorAll(selectors));
        const visible = elements.filter(el => {
            const rect = el.getBoundingClientRect();
            return rect.width > 0 && rect.height > 0 &&
                   rect.top >= 0 && rect.top < window.innerHeight &&
                   rect.left >= 0 && rect.left < window.innerWidth;
        });
        function genLabels(count) {
            const chars = 'abcdefghijklmnopqrstuvwxyz';
            let labels = [];
            if (count <= 26) {
                for (let i = 0; i < count; i++) labels.push(chars[i]);
            } else {
                for (let i = 0; i < count; i++) {
                    let l = '';
                    let n = i;
                    do {
                        l = chars[n % 26] + l;
                        n = Math.floor(n / 26) - 1;
                    } while (n >= 0);
                    labels.push(l);
                }
            }
            return labels;
        }
        const labels = genLabels(visible.length);
        const hints = [];
        visible.forEach((el, i) => {
            const rect = el.getBoundingClientRect();
            const hint = document.createElement('div');
            hint.className = '__thane_hint';
            hint.textContent = labels[i];
            hint.style.cssText = `
                position: fixed;
                left: ${rect.left}px;
                top: ${rect.top}px;
                background: #f7d94c;
                color: #000;
                font: bold 11px/14px monospace;
                padding: 1px 3px;
                border-radius: 3px;
                z-index: 999999;
                pointer-events: none;
                box-shadow: 0 1px 3px rgba(0,0,0,0.3);
            `;
            hint.dataset.label = labels[i];
            document.body.appendChild(hint);
            let selector = '';
            if (el.id) selector = '#' + el.id;
            else {
                let path = [];
                let cur = el;
                while (cur && cur !== document.body) {
                    let s = cur.tagName.toLowerCase();
                    if (cur.id) { path.unshift('#' + cur.id); break; }
                    let sib = cur, nth = 1;
                    while (sib = sib.previousElementSibling) {
                        if (sib.tagName === cur.tagName) nth++;
                    }
                    if (nth > 1) s += ':nth-of-type(' + nth + ')';
                    path.unshift(s);
                    cur = cur.parentElement;
                }
                selector = path.join(' > ');
            }
            hints.push({label: labels[i], selector: selector});
        });
        return JSON.stringify(hints);
    })()
    """

    /// JSON-encode a Swift string for safe interpolation into JavaScript.
    /// Uses Foundation's JSONSerialization to produce a properly escaped JSON string literal.
    private static func jsonEncode(_ value: String) -> String {
        guard let data = try? JSONSerialization.data(withJSONObject: value),
              let str = String(data: data, encoding: .utf8) else {
            // Fallback: empty string (should never happen for valid UTF-8)
            return "\"\""
        }
        return str  // Already includes surrounding quotes
    }

    /// Click the element under the hint with the given label, then remove all hints.
    static func clickHintJS(label: String) -> String {
        let safeLabel = jsonEncode(label)
        return """
        (function() {
            const label = \(safeLabel);
            const hint = document.querySelector('.__thane_hint[data-label="' + CSS.escape(label) + '"]');
            if (!hint) return 'hint not found';
            const rect = hint.getBoundingClientRect();
            hint.style.display = 'none';
            const el = document.elementFromPoint(rect.left + 1, rect.top + 1);
            hint.style.display = '';
            document.querySelectorAll('.__thane_hint').forEach(e => e.remove());
            if (el) {
                el.click();
                if (el.tagName === 'A' && el.href) return 'navigated';
                return 'clicked';
            }
            return 'element not found at hint position';
        })()
        """
    }

    /// Narrow visible hints to those matching prefix. Click if exactly one match.
    /// Returns "clicked", "navigated", match count as string, or "0".
    static func matchHintJS(prefix: String) -> String {
        let safePrefix = jsonEncode(prefix)
        return """
        (function() {
            const prefix = \(safePrefix);
            const hints = document.querySelectorAll('.__thane_hint');
            let matches = [];
            hints.forEach(h => {
                if (h.dataset.label && h.dataset.label.startsWith(prefix)) {
                    h.style.opacity = '1';
                    matches.push(h);
                } else {
                    h.style.opacity = '0.15';
                }
            });
            if (matches.length === 0) {
                hints.forEach(e => e.remove());
                return '0';
            }
            if (matches.length === 1) {
                const hint = matches[0];
                const rect = hint.getBoundingClientRect();
                hint.style.display = 'none';
                const el = document.elementFromPoint(rect.left + 1, rect.top + 1);
                hint.style.display = '';
                hints.forEach(e => e.remove());
                if (el) {
                    el.click();
                    if (el.tagName === 'A' && el.href) return 'navigated';
                    return 'clicked';
                }
                return '0';
            }
            return String(matches.length);
        })()
        """
    }

    /// Remove all hint overlays.
    static let clearHintsJS = """
    (function() {
        document.querySelectorAll('.__thane_hint').forEach(e => e.remove());
        return 'cleared';
    })()
    """

    // MARK: - Scroll

    static let scrollDownJS = "window.scrollBy(0, window.innerHeight * 0.7)"
    static let scrollUpJS = "window.scrollBy(0, -window.innerHeight * 0.7)"
    static let scrollTopJS = "window.scrollTo(0, 0)"
    static let scrollBottomJS = "window.scrollTo(0, document.body.scrollHeight)"

    // MARK: - Accessibility tree

    /// Extract a simplified accessibility tree as JSON.
    static let accessibilityTreeJS = """
    (function() {
        function getSelector(el) {
            if (el.id) return '#' + el.id;
            if (el === document.body) return 'body';
            let path = [];
            let current = el;
            while (current && current !== document.body) {
                let selector = current.tagName.toLowerCase();
                if (current.id) {
                    selector = '#' + current.id;
                    path.unshift(selector);
                    break;
                }
                let sibling = current;
                let nth = 1;
                while (sibling = sibling.previousElementSibling) {
                    if (sibling.tagName === current.tagName) nth++;
                }
                if (nth > 1) selector += ':nth-of-type(' + nth + ')';
                path.unshift(selector);
                current = current.parentElement;
            }
            return path.join(' > ');
        }
        function buildTree(el, depth) {
            if (depth > 5) return null;
            let node = {
                role: el.getAttribute('role') || el.tagName.toLowerCase(),
                name: el.getAttribute('aria-label') || el.textContent?.substring(0, 100)?.trim() || null,
                value: el.value || null,
                selector: getSelector(el),
                children: []
            };
            for (let child of el.children) {
                let childNode = buildTree(child, depth + 1);
                if (childNode) node.children.push(childNode);
            }
            return node;
        }
        return JSON.stringify(buildTree(document.body, 0));
    })()
    """

    // MARK: - Element interaction

    /// Click an element by CSS selector.
    static func clickElementJS(selector: String) -> String {
        let safeSelector = jsonEncode(selector)
        return """
        (function() {
            let el = document.querySelector(\(safeSelector));
            if (el) {
                el.click();
                return 'ok';
            } else {
                return 'element not found';
            }
        })()
        """
    }

    /// Type text into an element by CSS selector.
    static func typeTextJS(selector: String, text: String) -> String {
        let safeSelector = jsonEncode(selector)
        let safeText = jsonEncode(text)
        return """
        (function() {
            let el = document.querySelector(\(safeSelector));
            if (el) {
                el.focus();
                el.value = \(safeText);
                el.dispatchEvent(new Event('input', { bubbles: true }));
                return 'ok';
            } else {
                return 'element not found';
            }
        })()
        """
    }
}
