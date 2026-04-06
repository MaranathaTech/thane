use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::CoreError;
use crate::panel::PanelId;

/// Unique identifier for a pane (a leaf in the split tree).
pub type PaneId = Uuid;

/// Orientation of a split.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    Horizontal,
    Vertical,
}

/// A binary tree representing the pane layout within a workspace.
///
/// Each leaf holds a pane with one or more panels (tabs within a pane).
/// Each internal node represents a split with two children.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SplitTree {
    Leaf {
        pane_id: PaneId,
        /// Panels stacked within this pane (like tabs).
        panel_ids: Vec<PanelId>,
        /// Which panel is currently visible.
        selected_panel: PanelId,
    },
    Split {
        orientation: Orientation,
        /// Position of the divider as a fraction (0.0 to 1.0).
        divider_position: f64,
        first: Box<SplitTree>,
        second: Box<SplitTree>,
    },
}

impl SplitTree {
    /// Create a new leaf node with a single panel.
    pub fn new_leaf(panel_id: PanelId) -> Self {
        let pane_id = Uuid::new_v4();
        SplitTree::Leaf {
            pane_id,
            panel_ids: vec![panel_id],
            selected_panel: panel_id,
        }
    }

    /// Find a leaf by pane ID and return a reference.
    pub fn find_pane(&self, target: PaneId) -> Option<&SplitTree> {
        match self {
            SplitTree::Leaf { pane_id, .. } if *pane_id == target => Some(self),
            SplitTree::Split { first, second, .. } => {
                first.find_pane(target).or_else(|| second.find_pane(target))
            }
            _ => None,
        }
    }

    /// Find a leaf by pane ID and return a mutable reference.
    pub fn find_pane_mut(&mut self, target: PaneId) -> Option<&mut SplitTree> {
        match self {
            SplitTree::Leaf { pane_id, .. } if *pane_id == target => Some(self),
            SplitTree::Split { first, second, .. } => {
                if first.find_pane(target).is_some() {
                    first.find_pane_mut(target)
                } else {
                    second.find_pane_mut(target)
                }
            }
            _ => None,
        }
    }

    /// Collect all pane IDs in depth-first order.
    pub fn pane_ids(&self) -> Vec<PaneId> {
        match self {
            SplitTree::Leaf { pane_id, .. } => vec![*pane_id],
            SplitTree::Split { first, second, .. } => {
                let mut ids = first.pane_ids();
                ids.extend(second.pane_ids());
                ids
            }
        }
    }

    /// Collect all panel IDs across all panes.
    pub fn all_panel_ids(&self) -> Vec<PanelId> {
        match self {
            SplitTree::Leaf { panel_ids, .. } => panel_ids.clone(),
            SplitTree::Split { first, second, .. } => {
                let mut ids = first.all_panel_ids();
                ids.extend(second.all_panel_ids());
                ids
            }
        }
    }

    /// Count the number of leaf panes.
    pub fn pane_count(&self) -> usize {
        match self {
            SplitTree::Leaf { .. } => 1,
            SplitTree::Split { first, second, .. } => first.pane_count() + second.pane_count(),
        }
    }

    /// Split a pane identified by `target` into two, placing the new panel
    /// in the second position (right or below).
    ///
    /// Returns the new pane's ID.
    pub fn split(
        &mut self,
        target: PaneId,
        new_panel_id: PanelId,
        orientation: Orientation,
    ) -> Result<PaneId, CoreError> {
        self.split_inner(target, new_panel_id, orientation)
            .ok_or(CoreError::CannotSplit(target))
    }

    fn split_inner(
        &mut self,
        target: PaneId,
        new_panel_id: PanelId,
        orientation: Orientation,
    ) -> Option<PaneId> {
        match self {
            SplitTree::Leaf { pane_id, .. } if *pane_id == target => {
                let new_leaf = SplitTree::new_leaf(new_panel_id);
                let new_pane_id = match &new_leaf {
                    SplitTree::Leaf { pane_id, .. } => *pane_id,
                    _ => unreachable!(),
                };

                // Replace self with a split containing the old leaf and new leaf.
                let old = std::mem::replace(
                    self,
                    SplitTree::Split {
                        orientation,
                        divider_position: 0.5,
                        first: Box::new(SplitTree::Leaf {
                            pane_id: Uuid::nil(),
                            panel_ids: vec![],
                            selected_panel: Uuid::nil(),
                        }),
                        second: Box::new(new_leaf),
                    },
                );
                if let SplitTree::Split { first, .. } = self {
                    **first = old;
                }
                Some(new_pane_id)
            }
            SplitTree::Split { first, second, .. } => first
                .split_inner(target, new_panel_id, orientation)
                .or_else(|| second.split_inner(target, new_panel_id, orientation)),
            _ => None,
        }
    }

    /// Close a pane, returning the sibling tree to replace the parent split.
    /// Returns an error if this is the last pane.
    pub fn close_pane(&mut self, target: PaneId) -> Result<(), CoreError> {
        if self.pane_count() <= 1 {
            return Err(CoreError::CannotCloseLastPane);
        }
        if !self.close_pane_inner(target) {
            return Err(CoreError::PaneNotFound(target));
        }
        Ok(())
    }

    fn close_pane_inner(&mut self, target: PaneId) -> bool {
        match self {
            SplitTree::Leaf { .. } => false,
            SplitTree::Split { first, second, .. } => {
                // Check if target is a direct child leaf.
                let first_is_target = matches!(first.as_ref(), SplitTree::Leaf { pane_id, .. } if *pane_id == target);
                let second_is_target = matches!(second.as_ref(), SplitTree::Leaf { pane_id, .. } if *pane_id == target);

                if first_is_target {
                    let replacement = *second.clone();
                    *self = replacement;
                    return true;
                }
                if second_is_target {
                    let replacement = *first.clone();
                    *self = replacement;
                    return true;
                }

                // Recurse into children.
                first.close_pane_inner(target) || second.close_pane_inner(target)
            }
        }
    }

    /// Get the next pane ID after the given one (cycling).
    pub fn next_pane(&self, current: PaneId) -> Option<PaneId> {
        let ids = self.pane_ids();
        let pos = ids.iter().position(|&id| id == current)?;
        let next_pos = (pos + 1) % ids.len();
        Some(ids[next_pos])
    }

    /// Get the previous pane ID before the given one (cycling).
    pub fn prev_pane(&self, current: PaneId) -> Option<PaneId> {
        let ids = self.pane_ids();
        let pos = ids.iter().position(|&id| id == current)?;
        let prev_pos = if pos == 0 { ids.len() - 1 } else { pos - 1 };
        Some(ids[prev_pos])
    }

    /// Add a panel to a specific pane, making it the selected panel.
    pub fn add_panel_to_pane(&mut self, target: PaneId, panel_id: PanelId) -> bool {
        if let Some(SplitTree::Leaf {
            panel_ids,
            selected_panel,
            ..
        }) = self.find_pane_mut(target)
        {
            panel_ids.push(panel_id);
            *selected_panel = panel_id;
            return true;
        }
        false
    }

    /// Remove a panel from a pane. Returns true if the pane is now empty.
    /// If the removed panel was selected, selects an adjacent panel.
    pub fn remove_panel_from_pane(&mut self, target: PaneId, panel_id: PanelId) -> Option<bool> {
        if let Some(SplitTree::Leaf {
            panel_ids,
            selected_panel,
            ..
        }) = self.find_pane_mut(target)
        {
            let pos = panel_ids.iter().position(|&id| id == panel_id)?;
            panel_ids.remove(pos);

            if panel_ids.is_empty() {
                return Some(true);
            }

            // If we removed the selected panel, select an adjacent one.
            if *selected_panel == panel_id {
                let new_idx = if pos >= panel_ids.len() {
                    panel_ids.len() - 1
                } else {
                    pos
                };
                *selected_panel = panel_ids[new_idx];
            }

            return Some(false);
        }
        None
    }

    /// Set the selected panel in a pane.
    pub fn select_panel_in_pane(&mut self, target: PaneId, panel_id: PanelId) -> bool {
        if let Some(SplitTree::Leaf {
            panel_ids,
            selected_panel,
            ..
        }) = self.find_pane_mut(target)
            && panel_ids.contains(&panel_id)
        {
            *selected_panel = panel_id;
            return true;
        }
        false
    }

    /// Reorder panels within a pane by moving a panel to a new index.
    pub fn reorder_panel_in_pane(&mut self, target: PaneId, panel_id: PanelId, new_index: usize) -> bool {
        if let Some(SplitTree::Leaf { panel_ids, .. }) = self.find_pane_mut(target) {
            if let Some(old_idx) = panel_ids.iter().position(|&id| id == panel_id) {
                panel_ids.remove(old_idx);
                let clamped = new_index.min(panel_ids.len());
                panel_ids.insert(clamped, panel_id);
                return true;
            }
        }
        false
    }

    /// Swap the positions of two panels within a pane.
    /// Returns `true` if both panels were found in the pane and swapped.
    pub fn swap_panels_in_pane(&mut self, target: PaneId, panel_a: PanelId, panel_b: PanelId) -> bool {
        if let Some(SplitTree::Leaf { panel_ids, .. }) = self.find_pane_mut(target) {
            let pos_a = panel_ids.iter().position(|&id| id == panel_a);
            let pos_b = panel_ids.iter().position(|&id| id == panel_b);
            if let (Some(a), Some(b)) = (pos_a, pos_b) {
                panel_ids.swap(a, b);
                return true;
            }
        }
        false
    }

    /// Get the next panel in a pane (cycling forward).
    pub fn next_panel_in_pane(&self, target: PaneId) -> Option<PanelId> {
        if let Some(SplitTree::Leaf {
            panel_ids,
            selected_panel,
            ..
        }) = self.find_pane(target)
        {
            if panel_ids.len() <= 1 {
                return Some(*selected_panel);
            }
            let pos = panel_ids
                .iter()
                .position(|&id| id == *selected_panel)
                .unwrap_or(0);
            let next_pos = (pos + 1) % panel_ids.len();
            Some(panel_ids[next_pos])
        } else {
            None
        }
    }

    /// Get the previous panel in a pane (cycling backward).
    pub fn prev_panel_in_pane(&self, target: PaneId) -> Option<PanelId> {
        if let Some(SplitTree::Leaf {
            panel_ids,
            selected_panel,
            ..
        }) = self.find_pane(target)
        {
            if panel_ids.len() <= 1 {
                return Some(*selected_panel);
            }
            let pos = panel_ids
                .iter()
                .position(|&id| id == *selected_panel)
                .unwrap_or(0);
            let prev_pos = if pos == 0 {
                panel_ids.len() - 1
            } else {
                pos - 1
            };
            Some(panel_ids[prev_pos])
        } else {
            None
        }
    }

    /// Update divider positions using a depth-first index.
    /// `positions` maps (depth-first split index) → fractional position (0.0–1.0).
    /// Returns the number of splits visited.
    pub fn update_divider_positions(&mut self, positions: &[f64]) -> usize {
        self.update_divider_positions_inner(positions, 0)
    }

    fn update_divider_positions_inner(&mut self, positions: &[f64], mut idx: usize) -> usize {
        match self {
            SplitTree::Leaf { .. } => idx,
            SplitTree::Split {
                divider_position,
                first,
                second,
                ..
            } => {
                if let Some(&pos) = positions.get(idx) {
                    *divider_position = pos;
                }
                idx += 1;
                idx = first.update_divider_positions_inner(positions, idx);
                idx = second.update_divider_positions_inner(positions, idx);
                idx
            }
        }
    }

    /// Collect divider positions in depth-first order.
    pub fn collect_divider_positions(&self) -> Vec<f64> {
        let mut out = Vec::new();
        self.collect_divider_positions_inner(&mut out);
        out
    }

    fn collect_divider_positions_inner(&self, out: &mut Vec<f64>) {
        match self {
            SplitTree::Leaf { .. } => {}
            SplitTree::Split {
                divider_position,
                first,
                second,
                ..
            } => {
                out.push(*divider_position);
                first.collect_divider_positions_inner(out);
                second.collect_divider_positions_inner(out);
            }
        }
    }

    /// Find which pane contains a given panel ID.
    pub fn pane_for_panel(&self, panel_id: PanelId) -> Option<PaneId> {
        match self {
            SplitTree::Leaf {
                pane_id,
                panel_ids,
                ..
            } => {
                if panel_ids.contains(&panel_id) {
                    Some(*pane_id)
                } else {
                    None
                }
            }
            SplitTree::Split { first, second, .. } => first
                .pane_for_panel(panel_id)
                .or_else(|| second.pane_for_panel(panel_id)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_leaf() {
        let panel_id = Uuid::new_v4();
        let tree = SplitTree::new_leaf(panel_id);
        assert_eq!(tree.pane_count(), 1);
        assert_eq!(tree.all_panel_ids(), vec![panel_id]);
    }

    #[test]
    fn test_split_horizontal() {
        let panel1 = Uuid::new_v4();
        let panel2 = Uuid::new_v4();
        let mut tree = SplitTree::new_leaf(panel1);
        let pane_id = tree.pane_ids()[0];

        let new_pane = tree.split(pane_id, panel2, Orientation::Horizontal).unwrap();
        assert_eq!(tree.pane_count(), 2);
        assert!(tree.find_pane(new_pane).is_some());
    }

    #[test]
    fn test_split_vertical() {
        let panel1 = Uuid::new_v4();
        let panel2 = Uuid::new_v4();
        let mut tree = SplitTree::new_leaf(panel1);
        let pane_id = tree.pane_ids()[0];

        let new_pane = tree.split(pane_id, panel2, Orientation::Vertical).unwrap();
        assert_eq!(tree.pane_count(), 2);
        assert!(tree.find_pane(new_pane).is_some());
    }

    #[test]
    fn test_close_pane() {
        let panel1 = Uuid::new_v4();
        let panel2 = Uuid::new_v4();
        let mut tree = SplitTree::new_leaf(panel1);
        let original_pane = tree.pane_ids()[0];

        let new_pane = tree
            .split(original_pane, panel2, Orientation::Horizontal)
            .unwrap();
        assert_eq!(tree.pane_count(), 2);

        tree.close_pane(new_pane).unwrap();
        assert_eq!(tree.pane_count(), 1);
    }

    #[test]
    fn test_cannot_close_last_pane() {
        let panel1 = Uuid::new_v4();
        let mut tree = SplitTree::new_leaf(panel1);
        let pane_id = tree.pane_ids()[0];

        let result = tree.close_pane(pane_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_next_prev_pane() {
        let panel1 = Uuid::new_v4();
        let panel2 = Uuid::new_v4();
        let panel3 = Uuid::new_v4();
        let mut tree = SplitTree::new_leaf(panel1);
        let pane1 = tree.pane_ids()[0];

        let pane2 = tree
            .split(pane1, panel2, Orientation::Horizontal)
            .unwrap();
        let pane3 = tree
            .split(pane2, panel3, Orientation::Vertical)
            .unwrap();

        // Next from pane1 should go to pane2
        assert_eq!(tree.next_pane(pane1), Some(pane2));
        // Next from pane3 should cycle to pane1
        assert_eq!(tree.next_pane(pane3), Some(pane1));
        // Prev from pane1 should cycle to pane3
        assert_eq!(tree.prev_pane(pane1), Some(pane3));
    }

    #[test]
    fn test_add_panel_to_pane() {
        let panel1 = Uuid::new_v4();
        let panel2 = Uuid::new_v4();
        let mut tree = SplitTree::new_leaf(panel1);
        let pane_id = tree.pane_ids()[0];

        assert!(tree.add_panel_to_pane(pane_id, panel2));
        if let SplitTree::Leaf {
            panel_ids,
            selected_panel,
            ..
        } = &tree
        {
            assert_eq!(panel_ids.len(), 2);
            assert_eq!(*selected_panel, panel2);
        } else {
            panic!("Expected leaf");
        }
    }

    #[test]
    fn test_remove_panel_from_pane() {
        let panel1 = Uuid::new_v4();
        let panel2 = Uuid::new_v4();
        let mut tree = SplitTree::new_leaf(panel1);
        let pane_id = tree.pane_ids()[0];
        tree.add_panel_to_pane(pane_id, panel2);

        // Remove panel2 (the selected one) — panel1 should become selected.
        let empty = tree.remove_panel_from_pane(pane_id, panel2);
        assert_eq!(empty, Some(false));
        if let SplitTree::Leaf {
            panel_ids,
            selected_panel,
            ..
        } = &tree
        {
            assert_eq!(panel_ids.len(), 1);
            assert_eq!(*selected_panel, panel1);
        }

        // Remove the last panel.
        let empty = tree.remove_panel_from_pane(pane_id, panel1);
        assert_eq!(empty, Some(true));
    }

    #[test]
    fn test_select_panel_in_pane() {
        let panel1 = Uuid::new_v4();
        let panel2 = Uuid::new_v4();
        let mut tree = SplitTree::new_leaf(panel1);
        let pane_id = tree.pane_ids()[0];
        tree.add_panel_to_pane(pane_id, panel2);

        assert!(tree.select_panel_in_pane(pane_id, panel1));
        if let SplitTree::Leaf {
            selected_panel, ..
        } = &tree
        {
            assert_eq!(*selected_panel, panel1);
        }

        // Selecting a non-existent panel should fail.
        assert!(!tree.select_panel_in_pane(pane_id, Uuid::new_v4()));
    }

    #[test]
    fn test_next_prev_panel_in_pane() {
        let panel1 = Uuid::new_v4();
        let panel2 = Uuid::new_v4();
        let panel3 = Uuid::new_v4();
        let mut tree = SplitTree::new_leaf(panel1);
        let pane_id = tree.pane_ids()[0];
        tree.add_panel_to_pane(pane_id, panel2);
        tree.add_panel_to_pane(pane_id, panel3);
        // Selected is panel3 (last added).
        tree.select_panel_in_pane(pane_id, panel1);

        // Next from panel1 should be panel2.
        assert_eq!(tree.next_panel_in_pane(pane_id), Some(panel2));
        // Set selected to panel3 and test wrap.
        tree.select_panel_in_pane(pane_id, panel3);
        assert_eq!(tree.next_panel_in_pane(pane_id), Some(panel1));
        // Prev from panel3 should be panel2.
        assert_eq!(tree.prev_panel_in_pane(pane_id), Some(panel2));
        // Prev from panel1 should wrap to panel3.
        tree.select_panel_in_pane(pane_id, panel1);
        assert_eq!(tree.prev_panel_in_pane(pane_id), Some(panel3));
    }

    #[test]
    fn test_pane_for_panel() {
        let panel1 = Uuid::new_v4();
        let panel2 = Uuid::new_v4();
        let mut tree = SplitTree::new_leaf(panel1);
        let pane1 = tree.pane_ids()[0];
        let pane2 = tree
            .split(pane1, panel2, Orientation::Horizontal)
            .unwrap();

        assert_eq!(tree.pane_for_panel(panel1), Some(pane1));
        assert_eq!(tree.pane_for_panel(panel2), Some(pane2));
        assert_eq!(tree.pane_for_panel(Uuid::new_v4()), None);
    }

    #[test]
    fn test_deep_split_and_close() {
        let panel1 = Uuid::new_v4();
        let panel2 = Uuid::new_v4();
        let panel3 = Uuid::new_v4();
        let panel4 = Uuid::new_v4();
        let mut tree = SplitTree::new_leaf(panel1);
        let pane1 = tree.pane_ids()[0];

        let pane2 = tree
            .split(pane1, panel2, Orientation::Horizontal)
            .unwrap();
        let pane3 = tree
            .split(pane2, panel3, Orientation::Vertical)
            .unwrap();
        let _pane4 = tree
            .split(pane3, panel4, Orientation::Horizontal)
            .unwrap();
        assert_eq!(tree.pane_count(), 4);

        // Close an inner pane
        tree.close_pane(pane3).unwrap();
        assert_eq!(tree.pane_count(), 3);
    }

    #[test]
    fn test_divider_position_roundtrip() {
        let panel1 = Uuid::new_v4();
        let panel2 = Uuid::new_v4();
        let panel3 = Uuid::new_v4();
        let mut tree = SplitTree::new_leaf(panel1);
        let pane1 = tree.pane_ids()[0];
        let pane2 = tree.split(pane1, panel2, Orientation::Horizontal).unwrap();
        tree.split(pane2, panel3, Orientation::Vertical).unwrap();

        // Set custom divider positions
        let positions = vec![0.3, 0.7];
        tree.update_divider_positions(&positions);

        // Collect them back
        let collected = tree.collect_divider_positions();
        assert_eq!(collected.len(), 2);
        assert!((collected[0] - 0.3).abs() < f64::EPSILON);
        assert!((collected[1] - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn test_split_nonexistent_pane_returns_error() {
        let panel1 = Uuid::new_v4();
        let panel2 = Uuid::new_v4();
        let mut tree = SplitTree::new_leaf(panel1);
        let nonexistent = Uuid::new_v4();

        let result = tree.split(nonexistent, panel2, Orientation::Horizontal);
        assert!(result.is_err());
    }

    #[test]
    fn test_single_panel_pane_cycling_returns_same() {
        let panel1 = Uuid::new_v4();
        let tree = SplitTree::new_leaf(panel1);
        let pane_id = tree.pane_ids()[0];

        // With only one panel, next/prev should return the same panel
        assert_eq!(tree.next_panel_in_pane(pane_id), Some(panel1));
        assert_eq!(tree.prev_panel_in_pane(pane_id), Some(panel1));
    }

    #[test]
    fn test_close_pane_nonexistent_returns_error() {
        let panel1 = Uuid::new_v4();
        let panel2 = Uuid::new_v4();
        let mut tree = SplitTree::new_leaf(panel1);
        let pane1 = tree.pane_ids()[0];
        tree.split(pane1, panel2, Orientation::Horizontal).unwrap();

        let result = tree.close_pane(Uuid::new_v4());
        assert!(result.is_err());
    }

    #[test]
    fn test_single_pane_next_prev_returns_self() {
        let panel1 = Uuid::new_v4();
        let tree = SplitTree::new_leaf(panel1);
        let pane_id = tree.pane_ids()[0];

        // Single pane: next/prev cycles to itself
        assert_eq!(tree.next_pane(pane_id), Some(pane_id));
        assert_eq!(tree.prev_pane(pane_id), Some(pane_id));
    }

    #[test]
    fn test_swap_panels_in_pane() {
        let panel1 = Uuid::new_v4();
        let panel2 = Uuid::new_v4();
        let panel3 = Uuid::new_v4();
        let mut tree = SplitTree::new_leaf(panel1);
        let pane_id = tree.pane_ids()[0];
        tree.add_panel_to_pane(pane_id, panel2);
        tree.add_panel_to_pane(pane_id, panel3);

        // Order: [panel1, panel2, panel3]. Swap panel1 and panel3.
        assert!(tree.swap_panels_in_pane(pane_id, panel1, panel3));
        if let SplitTree::Leaf { panel_ids, .. } = &tree {
            assert_eq!(panel_ids, &[panel3, panel2, panel1]);
        } else {
            panic!("Expected leaf");
        }

        // Swap with a non-existent panel should fail.
        assert!(!tree.swap_panels_in_pane(pane_id, panel1, Uuid::new_v4()));

        // Swap in a non-existent pane should fail.
        assert!(!tree.swap_panels_in_pane(Uuid::new_v4(), panel1, panel2));
    }
}
