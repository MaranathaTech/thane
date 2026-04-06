use serde::{Deserialize, Serialize};

/// An element in the browser's accessibility tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessibilityNode {
    pub role: String,
    pub name: Option<String>,
    pub value: Option<String>,
    pub selector: String,
    pub children: Vec<AccessibilityNode>,
}

/// JavaScript snippet to extract a simplified accessibility tree.
pub const ACCESSIBILITY_TREE_JS: &str = r#"
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
"#;

/// JavaScript snippet to click an element by CSS selector.
pub fn click_element_js(selector: &str) -> String {
    let escaped = selector.replace('\\', "\\\\").replace('\'', "\\'");
    format!(
        r#"
        (function() {{
            let el = document.querySelector('{escaped}');
            if (el) {{
                el.click();
                return 'ok';
            }} else {{
                return 'element not found';
            }}
        }})()
        "#
    )
}

/// JavaScript snippet to show Vimium-style link hints on all clickable elements.
/// Returns JSON array of {label, selector, rect} objects.
pub const VIMIUM_SHOW_HINTS_JS: &str = r#"
(function() {
    // Remove any existing hints.
    document.querySelectorAll('.__thane_hint').forEach(e => e.remove());

    // Find all clickable/focusable elements.
    const selectors = 'a[href], button, input, select, textarea, [role="button"], [role="link"], [onclick], [tabindex]:not([tabindex="-1"])';
    const elements = Array.from(document.querySelectorAll(selectors));
    const visible = elements.filter(el => {
        const rect = el.getBoundingClientRect();
        return rect.width > 0 && rect.height > 0 &&
               rect.top >= 0 && rect.top < window.innerHeight &&
               rect.left >= 0 && rect.left < window.innerWidth;
    });

    // Generate short labels (a, b, ..., z, aa, ab, ...).
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
"#;

/// JavaScript snippet to click the element with the given hint label.
pub fn vimium_click_hint_js(label: &str) -> String {
    let escaped = label.replace('\\', "\\\\").replace('\'', "\\'");
    format!(
        r#"
        (function() {{
            const hint = document.querySelector('.__thane_hint[data-label="{escaped}"]');
            if (!hint) return 'hint not found';
            // Find the element at the hint's position.
            const rect = hint.getBoundingClientRect();
            hint.style.display = 'none';
            const el = document.elementFromPoint(rect.left + 1, rect.top + 1);
            hint.style.display = '';
            // Remove all hints.
            document.querySelectorAll('.__thane_hint').forEach(e => e.remove());
            if (el) {{
                el.click();
                if (el.tagName === 'A' && el.href) return 'navigated';
                return 'clicked';
            }}
            return 'element not found at hint position';
        }})()
        "#
    )
}

/// JavaScript snippet to remove all link hints.
pub const VIMIUM_CLEAR_HINTS_JS: &str = r#"
(function() {
    document.querySelectorAll('.__thane_hint').forEach(e => e.remove());
    return 'cleared';
})()
"#;

/// JavaScript snippet to scroll down by a page.
pub const VIMIUM_SCROLL_DOWN_JS: &str = "window.scrollBy(0, window.innerHeight * 0.7)";

/// JavaScript snippet to scroll up by a page.
pub const VIMIUM_SCROLL_UP_JS: &str = "window.scrollBy(0, -window.innerHeight * 0.7)";

/// JavaScript snippet to scroll to the top.
pub const VIMIUM_SCROLL_TOP_JS: &str = "window.scrollTo(0, 0)";

/// JavaScript snippet to scroll to the bottom.
pub const VIMIUM_SCROLL_BOTTOM_JS: &str = "window.scrollTo(0, document.body.scrollHeight)";

/// JavaScript snippet to type text into an element.
pub fn type_text_js(selector: &str, text: &str) -> String {
    let escaped_sel = selector.replace('\\', "\\\\").replace('\'', "\\'");
    let escaped_text = text.replace('\\', "\\\\").replace('\'', "\\'");
    format!(
        r#"
        (function() {{
            let el = document.querySelector('{escaped_sel}');
            if (el) {{
                el.focus();
                el.value = '{escaped_text}';
                el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                return 'ok';
            }} else {{
                return 'element not found';
            }}
        }})()
        "#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_click_element_js_simple() {
        let js = click_element_js("#submit");
        assert!(js.contains("document.querySelector('#submit')"));
        assert!(js.contains("el.click()"));
    }

    #[test]
    fn test_click_element_js_escapes_single_quotes() {
        let js = click_element_js("input[name='email']");
        // Single quotes in the selector are escaped with backslash.
        assert!(js.contains(r"input[name=\'email\']"));
    }

    #[test]
    fn test_click_element_js_escapes_backslashes() {
        let js = click_element_js(r"div.foo\.bar");
        assert!(js.contains(r"div.foo\\.bar"));
    }

    #[test]
    fn test_vimium_click_hint_js_simple() {
        let js = vimium_click_hint_js("a");
        assert!(js.contains(r#"data-label="a""#));
        assert!(js.contains("el.click()"));
    }

    #[test]
    fn test_vimium_click_hint_js_escapes() {
        let js = vimium_click_hint_js("a'b");
        assert!(js.contains(r"a\'b"));
    }

    #[test]
    fn test_type_text_js_simple() {
        let js = type_text_js("#input", "hello");
        assert!(js.contains("document.querySelector('#input')"));
        assert!(js.contains("el.value = 'hello'"));
        assert!(js.contains("el.focus()"));
    }

    #[test]
    fn test_type_text_js_escapes_quotes() {
        let js = type_text_js("input[name='q']", "it's a test");
        assert!(js.contains(r"input[name=\'q\']"));
        assert!(js.contains(r"it\'s a test"));
    }

    #[test]
    fn test_type_text_js_escapes_backslashes() {
        let js = type_text_js("#field", r"path\to\file");
        assert!(js.contains(r"path\\to\\file"));
    }
}
