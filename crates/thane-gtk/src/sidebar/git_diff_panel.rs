use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;

use thane_core::git::{DiffFile, DiffHunk, DiffLineKind};
use gtk4::prelude::*;

// ──────────────────────────────────────────────────────────────
// Tree data structures
// ──────────────────────────────────────────────────────────────

/// A node in the directory tree shown in the git diff panel.
#[derive(Debug)]
enum TreeNode<'a> {
    Dir {
        /// Display name — may be a compacted path like "crates/thane-core/src".
        name: String,
        children: Vec<TreeNode<'a>>,
    },
    File {
        file: &'a DiffFile,
        /// The filename component (last segment of the path).
        name: String,
    },
}

/// Temporary mutable tree used while building the directory hierarchy.
#[derive(Default)]
struct TempDir<'a> {
    subdirs: BTreeMap<String, TempDir<'a>>,
    files: Vec<&'a DiffFile>,
}

impl<'a> TempDir<'a> {
    fn insert(&mut self, parts: &[&str], file: &'a DiffFile) {
        if parts.len() == 1 {
            self.files.push(file);
        } else {
            self.subdirs
                .entry(parts[0].to_string())
                .or_default()
                .insert(&parts[1..], file);
        }
    }

    /// Convert into a sorted list of `TreeNode`s.
    /// Directories come first (alphabetical), then files (alphabetical).
    fn into_tree_nodes(self) -> Vec<TreeNode<'a>> {
        let mut nodes = Vec::new();

        for (name, child) in self.subdirs {
            let children = child.into_tree_nodes();
            nodes.push(TreeNode::Dir { name, children });
        }

        let mut file_nodes: Vec<TreeNode<'a>> = self
            .files
            .into_iter()
            .map(|f| {
                let name = f
                    .path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&f.path)
                    .to_string();
                TreeNode::File { file: f, name }
            })
            .collect();
        file_nodes.sort_by(|a, b| {
            let a_name = match a {
                TreeNode::File { name, .. } => name,
                _ => unreachable!(),
            };
            let b_name = match b {
                TreeNode::File { name, .. } => name,
                _ => unreachable!(),
            };
            a_name.cmp(b_name)
        });
        nodes.extend(file_nodes);

        nodes
    }
}

/// Compact single-child directory chains: if a Dir has exactly one child that
/// is also a Dir, merge them into "parent/child".
fn compact_dirs(nodes: Vec<TreeNode<'_>>) -> Vec<TreeNode<'_>> {
    nodes
        .into_iter()
        .map(|node| match node {
            TreeNode::Dir { name, children } => {
                let children = compact_dirs(children);
                if children.len() == 1
                    && let TreeNode::Dir {
                        name: child_name,
                        children: grandchildren,
                    } = &children[0]
                {
                    return TreeNode::Dir {
                        name: format!("{name}/{child_name}"),
                        children: grandchildren.clone(),
                    };
                }
                TreeNode::Dir { name, children }
            }
            other => other,
        })
        .collect()
}

// We need Clone for compact_dirs to work when merging single-child chains.
impl Clone for TreeNode<'_> {
    fn clone(&self) -> Self {
        match self {
            TreeNode::Dir { name, children } => TreeNode::Dir {
                name: name.clone(),
                children: children.clone(),
            },
            TreeNode::File { file, name } => TreeNode::File {
                file,
                name: name.clone(),
            },
        }
    }
}

/// Build a nested directory tree from a flat list of diff files.
fn build_dir_tree(files: &[DiffFile]) -> Vec<TreeNode<'_>> {
    let mut root = TempDir::default();

    for file in files {
        let parts: Vec<&str> = file.path.split('/').collect();
        root.insert(&parts, file);
    }

    let nodes = root.into_tree_nodes();
    compact_dirs(nodes)
}

/// Count total added/removed lines across all files under a tree node.
fn count_tree_lines(nodes: &[TreeNode<'_>]) -> (usize, usize) {
    let mut plus = 0;
    let mut minus = 0;
    for node in nodes {
        match node {
            TreeNode::Dir { children, .. } => {
                let (p, m) = count_tree_lines(children);
                plus += p;
                minus += m;
            }
            TreeNode::File { file, .. } => {
                let (p, m) = count_diff_lines(file);
                plus += p;
                minus += m;
            }
        }
    }
    (plus, minus)
}

// ──────────────────────────────────────────────────────────────
// Panel widget
// ──────────────────────────────────────────────────────────────

/// A panel showing the git diff for a workspace or specific directory.
pub struct GitDiffPanel {
    container: gtk4::Box,
    content_box: gtk4::Box,
    status_label: gtk4::Label,
    subtitle_label: gtk4::Label,
    close_btn: gtk4::Button,
}

impl Default for GitDiffPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl GitDiffPanel {
    pub fn new() -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        container.add_css_class("git-diff-panel");
        container.set_width_request(420);

        // Header.
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        header.set_margin_start(12);
        header.set_margin_end(12);
        header.set_margin_top(8);
        header.set_margin_bottom(0);

        let title = gtk4::Label::new(Some("Git Changes"));
        title.add_css_class("workspace-title");
        title.set_hexpand(true);
        title.set_halign(gtk4::Align::Start);
        header.append(&title);

        let status_label = gtk4::Label::new(Some(""));
        status_label.add_css_class("git-diff-status");
        header.append(&status_label);

        let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        close_btn.add_css_class("flat");
        close_btn.set_tooltip_text(Some("Close"));
        header.append(&close_btn);

        container.append(&header);

        // Subtitle: shows the CWD being diffed.
        let subtitle_label = gtk4::Label::new(None);
        subtitle_label.add_css_class("git-diff-subtitle");
        subtitle_label.set_halign(gtk4::Align::Start);
        subtitle_label.set_margin_start(12);
        subtitle_label.set_margin_bottom(4);
        subtitle_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
        subtitle_label.set_visible(false);
        container.append(&subtitle_label);

        // Scrollable content area (replaces ListBox with a plain Box).
        let scrolled = gtk4::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_hscrollbar_policy(gtk4::PolicyType::Automatic);

        let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        content_box.add_css_class("git-diff-list");
        scrolled.set_child(Some(&content_box));

        container.append(&scrolled);

        Self {
            container,
            content_box,
            status_label,
            subtitle_label,
            close_btn,
        }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    /// Set the subtitle showing which directory is being diffed.
    pub fn set_subtitle(&self, path: Option<&str>) {
        match path {
            Some(p) if !p.is_empty() => {
                self.subtitle_label.set_text(p);
                self.subtitle_label.set_visible(true);
            }
            _ => {
                self.subtitle_label.set_visible(false);
            }
        }
    }

    /// Connect the close button callback.
    pub fn connect_close<F: Fn() + 'static>(&self, f: F) {
        self.close_btn.connect_clicked(move |_| f());
    }

    /// Refresh the diff panel with new data.
    pub fn refresh(&self, files: &[DiffFile]) {
        // Remove all existing children.
        while let Some(child) = self.content_box.first_child() {
            self.content_box.remove(&child);
        }

        if files.is_empty() {
            self.status_label.set_text("Clean");
            let label = gtk4::Label::new(Some("No changes"));
            label.add_css_class("dim-label");
            label.set_margin_top(16);
            label.set_margin_bottom(16);
            self.content_box.append(&label);
            return;
        }

        // Count stats.
        let added = files
            .iter()
            .filter(|f| f.status == 'A' || f.status == '?')
            .count();
        let modified = files.iter().filter(|f| f.status == 'M').count();
        let deleted = files.iter().filter(|f| f.status == 'D').count();
        let total = files.len();

        let mut parts = Vec::new();
        if added > 0 {
            parts.push(format!("+{added}"));
        }
        if modified > 0 {
            parts.push(format!("~{modified}"));
        }
        if deleted > 0 {
            parts.push(format!("-{deleted}"));
        }
        self.status_label
            .set_text(&format!("{total} files ({})", parts.join(" ")));

        // Build directory tree and render it.
        let tree = build_dir_tree(files);
        render_tree_into_box(&self.content_box, &tree, 0);
    }
}

// ──────────────────────────────────────────────────────────────
// Tree rendering
// ──────────────────────────────────────────────────────────────

/// Recursively render tree nodes into a GTK Box.
fn render_tree_into_box(parent: &gtk4::Box, nodes: &[TreeNode<'_>], depth: u32) {
    for node in nodes {
        match node {
            TreeNode::Dir { name, children } => {
                let dir_widget = create_dir_row(name, children, depth);
                parent.append(&dir_widget);
            }
            TreeNode::File { file, .. } => {
                let row = create_diff_file_row(file, depth);
                parent.append(&row);
            }
        }
    }
}

/// Create a collapsible directory row with chevron, folder name, aggregate
/// line counts, and a `Revealer` containing children.
fn create_dir_row(name: &str, children: &[TreeNode<'_>], depth: u32) -> gtk4::Box {
    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    // Directory header row.
    let dir_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    dir_header.add_css_class("git-diff-dir-row");
    dir_header.set_cursor_from_name(Some("pointer"));
    dir_header.set_margin_start((8 + depth * 16) as i32);
    dir_header.set_margin_end(8);
    dir_header.set_margin_top(4);
    dir_header.set_margin_bottom(2);

    // Chevron (expanded by default).
    let chevron = gtk4::Label::new(Some("\u{25BC}")); // down-pointing triangle
    chevron.add_css_class("git-diff-chevron");
    dir_header.append(&chevron);

    // Folder name.
    let name_label = gtk4::Label::new(Some(name));
    name_label.add_css_class("git-diff-dir-name");
    name_label.set_hexpand(true);
    name_label.set_halign(gtk4::Align::Start);
    name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    dir_header.append(&name_label);

    // Aggregate line counts.
    let (plus, minus) = count_tree_lines(children);
    if plus > 0 || minus > 0 {
        let summary_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
        if plus > 0 {
            let plus_label = gtk4::Label::new(Some(&format!("+{plus}")));
            plus_label.add_css_class("git-diff-line-count-plus");
            summary_box.append(&plus_label);
        }
        if minus > 0 {
            let minus_label = gtk4::Label::new(Some(&format!("-{minus}")));
            minus_label.add_css_class("git-diff-line-count-minus");
            summary_box.append(&minus_label);
        }
        dir_header.append(&summary_box);
    }

    outer.append(&dir_header);

    // Children inside a Revealer (expanded by default).
    let revealer = gtk4::Revealer::new();
    revealer.set_reveal_child(true);
    revealer.set_transition_type(gtk4::RevealerTransitionType::SlideDown);
    revealer.set_transition_duration(150);

    let children_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    render_tree_into_box(&children_box, children, depth + 1);
    revealer.set_child(Some(&children_box));
    outer.append(&revealer);

    // Click to toggle expand/collapse.
    let expanded = Rc::new(Cell::new(true));
    let gesture = gtk4::GestureClick::new();
    gesture.set_button(1);
    let chevron_ref = chevron.clone();
    let outer_ref = outer.clone();
    gesture.connect_released(move |gesture, _n, _x, _y| {
        gesture.set_state(gtk4::EventSequenceState::Claimed);
        let is_expanded = !expanded.get();
        expanded.set(is_expanded);
        revealer.set_reveal_child(is_expanded);
        if is_expanded {
            outer_ref.remove_css_class("git-diff-dir-collapsed");
            chevron_ref.set_text("\u{25BC}"); // down-pointing triangle
        } else {
            outer_ref.add_css_class("git-diff-dir-collapsed");
            chevron_ref.set_text("\u{25B6}"); // right-pointing triangle
        }
    });
    dir_header.add_controller(gesture);

    outer
}

// ──────────────────────────────────────────────────────────────
// File row (adapted with depth parameter)
// ──────────────────────────────────────────────────────────────

/// Split a file path into (filename, directory).
/// e.g. "src/components/button.rs" -> ("button.rs", "src/components/")
fn split_path(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(idx) => (&path[idx + 1..], &path[..=idx]),
        None => (path, ""),
    }
}

/// Parse hunk header to extract starting line numbers.
/// e.g. "@@ -10,5 +12,7 @@" -> (10, 12)
fn parse_hunk_start_lines(header: &str) -> (usize, usize) {
    // Format: @@ -OLD_START[,COUNT] +NEW_START[,COUNT] @@
    let mut old_start = 1usize;
    let mut new_start = 1usize;

    if let Some(rest) = header.strip_prefix("@@ -") {
        let parts: Vec<&str> = rest.splitn(2, ' ').collect();
        if let Some(old_part) = parts.first()
            && let Some(num) = old_part.split(',').next()
        {
            old_start = num.parse().unwrap_or(1);
        }
        if let Some(new_part) = parts.get(1)
            && let Some(rest) = new_part.strip_prefix('+')
            && let Some(num) = rest.split(',').next()
        {
            new_start = num.parse().unwrap_or(1);
        }
    }

    (old_start, new_start)
}

fn create_diff_file_row(file: &DiffFile, depth: u32) -> gtk4::Box {
    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    outer.add_css_class("git-diff-file");

    let (filename, _dirname) = split_path(&file.path);

    // Clickable file header row.
    let file_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    file_header.add_css_class("git-diff-file-row");
    file_header.set_cursor_from_name(Some("pointer"));
    file_header.set_margin_start((8 + depth * 16) as i32);
    file_header.set_margin_end(8);
    file_header.set_margin_top(4);
    file_header.set_margin_bottom(2);

    // Status badge.
    let status_text = match file.status {
        'M' => "M",
        'A' => "A",
        'D' => "D",
        'R' => "R",
        '?' => "U",
        _ => "?",
    };
    let status_badge = gtk4::Label::new(Some(status_text));
    status_badge.add_css_class("git-diff-status-badge");
    status_badge.add_css_class(match file.status {
        'M' => "git-diff-modified",
        'A' | '?' => "git-diff-added",
        'D' => "git-diff-deleted",
        'R' => "git-diff-renamed",
        _ => "git-diff-modified",
    });
    file_header.append(&status_badge);

    // File name only (tree provides directory context).
    let name_label = gtk4::Label::new(Some(filename));
    name_label.add_css_class("git-diff-filename");
    name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    name_label.set_hexpand(true);
    name_label.set_halign(gtk4::Align::Start);
    file_header.append(&name_label);

    // Line count summary with colored +/-.
    let (plus, minus) = count_diff_lines(file);
    if plus > 0 || minus > 0 {
        let summary_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
        if plus > 0 {
            let plus_label = gtk4::Label::new(Some(&format!("+{plus}")));
            plus_label.add_css_class("git-diff-line-count-plus");
            summary_box.append(&plus_label);
        }
        if minus > 0 {
            let minus_label = gtk4::Label::new(Some(&format!("-{minus}")));
            minus_label.add_css_class("git-diff-line-count-minus");
            summary_box.append(&minus_label);
        }
        file_header.append(&summary_box);
    }

    // Expand/collapse chevron.
    let chevron = gtk4::Label::new(Some("\u{25B6}")); // right-pointing triangle
    chevron.add_css_class("git-diff-chevron");
    file_header.append(&chevron);

    outer.append(&file_header);

    // Diff hunks inside a Revealer (collapsed by default).
    if !file.hunks.is_empty() {
        let revealer = gtk4::Revealer::new();
        revealer.set_reveal_child(false);
        revealer.set_transition_type(gtk4::RevealerTransitionType::SlideDown);
        revealer.set_transition_duration(150);

        let diff_view = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        diff_view.add_css_class("git-diff-hunks");
        diff_view.set_margin_start((8 + depth * 16) as i32);
        diff_view.set_margin_end(8);
        diff_view.set_margin_bottom(4);

        for hunk in &file.hunks {
            build_hunk_view(&diff_view, hunk);
        }

        revealer.set_child(Some(&diff_view));
        outer.append(&revealer);

        // Click on file header → toggle expand/collapse.
        let expanded = Rc::new(Cell::new(false));
        let gesture = gtk4::GestureClick::new();
        gesture.set_button(1);
        let outer_ref = outer.clone();
        let chevron_ref = chevron.clone();
        gesture.connect_released(move |gesture, _n, _x, _y| {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            let is_expanded = !expanded.get();
            expanded.set(is_expanded);
            revealer.set_reveal_child(is_expanded);
            if is_expanded {
                outer_ref.add_css_class("git-diff-file-expanded");
                chevron_ref.set_text("\u{25BC}"); // down-pointing triangle
            } else {
                outer_ref.remove_css_class("git-diff-file-expanded");
                chevron_ref.set_text("\u{25B6}"); // right-pointing triangle
            }
        });
        file_header.add_controller(gesture);
    }

    outer
}

fn build_hunk_view(diff_view: &gtk4::Box, hunk: &DiffHunk) {
    // Hunk header.
    let hunk_header = gtk4::Label::new(Some(&hunk.header));
    hunk_header.set_halign(gtk4::Align::Start);
    hunk_header.add_css_class("git-diff-hunk-header");
    hunk_header.set_wrap(false);
    diff_view.append(&hunk_header);

    // Parse starting line numbers from hunk header.
    let (mut old_line, mut new_line) = parse_hunk_start_lines(&hunk.header);

    // Diff lines with line number gutter (cap to avoid overwhelming the UI).
    for line in hunk.lines.iter().take(200) {
        let line_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        line_row.add_css_class("git-diff-line");
        line_row.add_css_class(match line.kind {
            DiffLineKind::Added => "git-diff-line-added",
            DiffLineKind::Removed => "git-diff-line-removed",
            DiffLineKind::Context => "git-diff-line-context",
        });

        // Old line number gutter.
        let old_num_text = match line.kind {
            DiffLineKind::Added => String::new(),
            _ => {
                let s = old_line.to_string();
                old_line += 1;
                s
            }
        };
        let old_gutter = gtk4::Label::new(Some(&old_num_text));
        old_gutter.add_css_class("git-diff-gutter");
        old_gutter.set_width_chars(4);
        old_gutter.set_xalign(1.0);
        line_row.append(&old_gutter);

        // New line number gutter.
        let new_num_text = match line.kind {
            DiffLineKind::Removed => String::new(),
            _ => {
                let s = new_line.to_string();
                new_line += 1;
                s
            }
        };
        let new_gutter = gtk4::Label::new(Some(&new_num_text));
        new_gutter.add_css_class("git-diff-gutter");
        new_gutter.set_width_chars(4);
        new_gutter.set_xalign(1.0);
        line_row.append(&new_gutter);

        // +/- prefix.
        let prefix = match line.kind {
            DiffLineKind::Added => "+",
            DiffLineKind::Removed => "-",
            DiffLineKind::Context => " ",
        };
        let prefix_label = gtk4::Label::new(Some(prefix));
        prefix_label.add_css_class("git-diff-prefix");
        prefix_label.set_width_chars(2);
        line_row.append(&prefix_label);

        // Content.
        let content_label = gtk4::Label::new(Some(&line.content));
        content_label.set_halign(gtk4::Align::Start);
        content_label.set_hexpand(true);
        content_label.set_wrap(false);
        content_label.set_selectable(true);
        content_label.add_css_class("git-diff-content");
        line_row.append(&content_label);

        diff_view.append(&line_row);
    }
}

fn count_diff_lines(file: &DiffFile) -> (usize, usize) {
    let mut plus = 0;
    let mut minus = 0;
    for hunk in &file.hunks {
        for line in &hunk.lines {
            match line.kind {
                DiffLineKind::Added => plus += 1,
                DiffLineKind::Removed => minus += 1,
                DiffLineKind::Context => {}
            }
        }
    }
    (plus, minus)
}

// ──────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_file(path: &str, status: char) -> DiffFile {
        DiffFile {
            path: path.to_string(),
            status,
            hunks: vec![],
        }
    }

    fn dir_names(nodes: &[TreeNode<'_>]) -> Vec<String> {
        nodes
            .iter()
            .filter_map(|n| match n {
                TreeNode::Dir { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    fn file_names(nodes: &[TreeNode<'_>]) -> Vec<String> {
        nodes
            .iter()
            .filter_map(|n| match n {
                TreeNode::File { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn empty_input() {
        let tree = build_dir_tree(&[]);
        assert!(tree.is_empty());
    }

    #[test]
    fn single_root_file() {
        let files = [make_file("README.md", 'M')];
        let tree = build_dir_tree(&files);
        assert_eq!(file_names(&tree), vec!["README.md"]);
        assert!(dir_names(&tree).is_empty());
    }

    #[test]
    fn files_in_one_directory() {
        let files = [
            make_file("src/main.rs", 'M'),
            make_file("src/lib.rs", 'A'),
        ];
        let tree = build_dir_tree(&files);
        assert_eq!(dir_names(&tree), vec!["src"]);
        if let TreeNode::Dir { children, .. } = &tree[0] {
            assert_eq!(file_names(children), vec!["lib.rs", "main.rs"]);
        } else {
            panic!("expected Dir");
        }
    }

    #[test]
    fn compact_single_child_dirs() {
        let files = [make_file("crates/thane-core/src/lib.rs", 'M')];
        let tree = build_dir_tree(&files);
        // Should compact "crates" -> "thane-core" -> "src" into one dir node.
        assert_eq!(dir_names(&tree), vec!["crates/thane-core/src"]);
        if let TreeNode::Dir { children, .. } = &tree[0] {
            assert_eq!(file_names(children), vec!["lib.rs"]);
        } else {
            panic!("expected Dir");
        }
    }

    #[test]
    fn no_compact_when_multiple_children() {
        let files = [
            make_file("src/main.rs", 'M'),
            make_file("src/lib.rs", 'A'),
            make_file("tests/test.rs", 'A'),
        ];
        let tree = build_dir_tree(&files);
        // "src" has 2 files, "tests" has 1 file — neither should compact further.
        assert_eq!(dir_names(&tree), vec!["src", "tests"]);
    }

    #[test]
    fn dirs_sorted_before_files() {
        let files = [
            make_file("Cargo.toml", 'M'),
            make_file("src/main.rs", 'M'),
        ];
        let tree = build_dir_tree(&files);
        assert!(matches!(tree[0], TreeNode::Dir { .. }));
        assert!(matches!(tree[1], TreeNode::File { .. }));
    }

    #[test]
    fn mixed_depths() {
        let files = [
            make_file("README.md", 'M'),
            make_file("src/main.rs", 'M'),
            make_file("src/utils/helpers.rs", 'A'),
            make_file("tests/integration.rs", 'A'),
        ];
        let tree = build_dir_tree(&files);
        // Top-level: src, tests dirs + README.md file
        assert_eq!(dir_names(&tree), vec!["src", "tests"]);
        assert_eq!(file_names(&tree), vec!["README.md"]);

        // src should have "utils" dir and "main.rs" file
        if let TreeNode::Dir { children, .. } = &tree[0] {
            assert_eq!(dir_names(children), vec!["utils"]);
            assert_eq!(file_names(children), vec!["main.rs"]);
        } else {
            panic!("expected Dir");
        }
    }

    #[test]
    fn compact_partial_chain() {
        // a/b/c.rs and a/b/d.rs — a/b should compact since "a" has single child "b"
        let files = [
            make_file("a/b/c.rs", 'M'),
            make_file("a/b/d.rs", 'A'),
        ];
        let tree = build_dir_tree(&files);
        assert_eq!(dir_names(&tree), vec!["a/b"]);
        if let TreeNode::Dir { children, .. } = &tree[0] {
            assert_eq!(file_names(children), vec!["c.rs", "d.rs"]);
        } else {
            panic!("expected Dir");
        }
    }

    #[test]
    fn split_path_basic() {
        assert_eq!(split_path("src/main.rs"), ("main.rs", "src/"));
        assert_eq!(split_path("README.md"), ("README.md", ""));
        assert_eq!(split_path("a/b/c.rs"), ("c.rs", "a/b/"));
    }

    #[test]
    fn parse_hunk_headers() {
        assert_eq!(parse_hunk_start_lines("@@ -10,5 +12,7 @@"), (10, 12));
        assert_eq!(parse_hunk_start_lines("@@ -1 +1 @@"), (1, 1));
        assert_eq!(
            parse_hunk_start_lines("@@ -100,20 +200,30 @@ fn foo()"),
            (100, 200)
        );
        assert_eq!(parse_hunk_start_lines("garbage"), (1, 1));
    }
}
