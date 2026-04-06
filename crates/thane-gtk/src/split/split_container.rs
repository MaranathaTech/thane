use thane_core::pane::{Orientation, PaneId, SplitTree};
use thane_core::panel::PanelId;
use glib;
use gtk4::prelude::*;

/// A container that renders a SplitTree as nested gtk4::Paned widgets.
///
/// Each leaf in the tree becomes a terminal or browser widget.
/// Each split becomes a Paned with two children.
pub struct SplitContainer {
    /// Root widget that holds the entire split layout.
    root: gtk4::Box,
}

impl Default for SplitContainer {
    fn default() -> Self {
        Self::new()
    }
}

impl SplitContainer {
    pub fn new() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root.set_hexpand(true);
        root.set_vexpand(true);
        Self { root }
    }

    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Rebuild the split layout from a SplitTree.
    ///
    /// `widget_for_pane` is a callback that returns the GTK widget for a given pane.
    pub fn rebuild<F>(&self, tree: &SplitTree, widget_for_pane: &F)
    where
        F: Fn(PaneId, &[PanelId], PanelId) -> gtk4::Widget,
    {
        // Remove existing children.
        while let Some(child) = self.root.first_child() {
            self.root.remove(&child);
        }

        let widget = self.build_widget(tree, widget_for_pane);
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        self.root.append(&widget);
    }

    /// Collect current divider positions from the live GTK Paned widgets in depth-first order.
    /// Returns positions as fractions (0.0–1.0) matching the SplitTree traversal order.
    pub fn collect_divider_positions(&self) -> Vec<f64> {
        let mut positions = Vec::new();
        if let Some(child) = self.root.first_child() {
            Self::collect_positions_recursive(&child, &mut positions);
        }
        positions
    }

    fn collect_positions_recursive(widget: &gtk4::Widget, positions: &mut Vec<f64>) {
        if let Some(paned) = widget.downcast_ref::<gtk4::Paned>() {
            let pos = paned.position() as f64;
            let size = if paned.orientation() == gtk4::Orientation::Horizontal {
                paned.width() as f64
            } else {
                paned.height() as f64
            };
            let fraction = if size > 10.0 { pos / size } else { 0.5 };
            positions.push(fraction.clamp(0.0, 1.0));

            // Recurse into children (depth-first: first then second).
            if let Some(start) = paned.start_child() {
                Self::collect_positions_recursive(&start, positions);
            }
            if let Some(end) = paned.end_child() {
                Self::collect_positions_recursive(&end, positions);
            }
        }
        // Leaf widgets (terminals, browsers, boxes) are not Paned — nothing to collect.
    }

    fn build_widget<F>(&self, tree: &SplitTree, widget_for_pane: &F) -> gtk4::Widget
    where
        F: Fn(PaneId, &[PanelId], PanelId) -> gtk4::Widget,
    {
        match tree {
            SplitTree::Leaf {
                pane_id,
                panel_ids,
                selected_panel,
            } => widget_for_pane(*pane_id, panel_ids, *selected_panel),

            SplitTree::Split {
                orientation,
                divider_position,
                first,
                second,
            } => {
                let gtk_orientation = match orientation {
                    Orientation::Horizontal => gtk4::Orientation::Horizontal,
                    Orientation::Vertical => gtk4::Orientation::Vertical,
                };

                let paned = gtk4::Paned::new(gtk_orientation);
                paned.set_wide_handle(true);

                // Ensure both children can resize and neither collapses to zero.
                paned.set_resize_start_child(true);
                paned.set_resize_end_child(true);
                paned.set_shrink_start_child(false);
                paned.set_shrink_end_child(false);

                let first_widget = self.build_widget(first, widget_for_pane);
                let second_widget = self.build_widget(second, widget_for_pane);

                first_widget.set_hexpand(true);
                first_widget.set_vexpand(true);
                second_widget.set_hexpand(true);
                second_widget.set_vexpand(true);

                paned.set_start_child(Some(&first_widget));
                paned.set_end_child(Some(&second_widget));

                // Set divider position after the widget has been laid out.
                let position = *divider_position;
                let paned_weak = paned.downgrade();
                glib::idle_add_local_once(move || {
                    if let Some(p) = paned_weak.upgrade() {
                        let size = if gtk_orientation == gtk4::Orientation::Horizontal {
                            p.width()
                        } else {
                            p.height()
                        };
                        if size > 10 {
                            p.set_position((size as f64 * position) as i32);
                        }
                    }
                });

                paned.upcast()
            }
        }
    }
}
