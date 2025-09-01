//! Custom layout engine for SwiftUI-like behavior in GTK4 backend.

use std::collections::HashMap;

/// A unique identifier for layout nodes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayoutId(usize);

impl LayoutId {
    pub fn new(id: usize) -> Self {
        Self(id)
    }
}

/// Represents a 2D size with width and height
#[derive(Debug, Clone, Copy, Default)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    pub fn zero() -> Self {
        Self::default()
    }

    pub fn infinite() -> Self {
        Self {
            width: f32::INFINITY,
            height: f32::INFINITY,
        }
    }
}

/// Represents a 2D position
#[derive(Debug, Clone, Copy, Default)]
pub struct Position {
    pub x: f32,
    pub y: f32,
}

impl Position {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn zero() -> Self {
        Self::default()
    }
}

/// A rectangle defined by position and size
#[derive(Debug, Clone, Copy, Default)]
pub struct Rect {
    pub origin: Position,
    pub size: Size,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            origin: Position::new(x, y),
            size: Size::new(width, height),
        }
    }

    pub fn zero() -> Self {
        Self::default()
    }
}

/// Layout constraints similar to SwiftUI's proposed size
#[derive(Debug, Clone, Copy)]
pub struct LayoutConstraints {
    pub min_size: Size,
    pub max_size: Size,
}

impl LayoutConstraints {
    pub fn new(min_size: Size, max_size: Size) -> Self {
        Self { min_size, max_size }
    }

    pub fn fixed(size: Size) -> Self {
        Self {
            min_size: size,
            max_size: size,
        }
    }

    pub fn loose(max_size: Size) -> Self {
        Self {
            min_size: Size::zero(),
            max_size,
        }
    }

    pub fn tight(size: Size) -> Self {
        Self::fixed(size)
    }

    /// Check if a size satisfies these constraints
    pub fn satisfies(&self, size: Size) -> bool {
        size.width >= self.min_size.width
            && size.width <= self.max_size.width
            && size.height >= self.min_size.height
            && size.height <= self.max_size.height
    }

    /// Constrain a size to fit within these constraints
    pub fn constrain(&self, size: Size) -> Size {
        Size {
            width: size.width.clamp(self.min_size.width, self.max_size.width),
            height: size
                .height
                .clamp(self.min_size.height, self.max_size.height),
        }
    }
}

/// The type of layout behavior for a container
#[derive(Debug, Clone)]
pub enum LayoutBehavior {
    /// A vertical stack (VStack) - arranges children vertically
    VStack {
        alignment: HorizontalAlignment,
        spacing: f32,
    },
    /// A horizontal stack (HStack) - arranges children horizontally  
    HStack {
        alignment: VerticalAlignment,
        spacing: f32,
    },
    /// A layered stack (ZStack) - overlays children
    ZStack { alignment: Alignment },
    /// A single leaf node (text, image, etc.)
    Leaf,
    /// A scroll view container
    ScrollView,
}

/// Horizontal alignment options
#[derive(Debug, Clone, Copy)]
pub enum HorizontalAlignment {
    Leading,
    Center,
    Trailing,
}

/// Vertical alignment options
#[derive(Debug, Clone, Copy)]
pub enum VerticalAlignment {
    Top,
    Center,
    Bottom,
}

/// Combined alignment for 2D positioning
#[derive(Debug, Clone, Copy)]
pub struct Alignment {
    pub horizontal: HorizontalAlignment,
    pub vertical: VerticalAlignment,
}

impl Alignment {
    pub const CENTER: Self = Self {
        horizontal: HorizontalAlignment::Center,
        vertical: VerticalAlignment::Center,
    };

    pub const TOP_LEADING: Self = Self {
        horizontal: HorizontalAlignment::Leading,
        vertical: VerticalAlignment::Top,
    };
}

/// A node in the layout tree
#[derive(Debug)]
pub struct LayoutNode {
    pub id: LayoutId,
    pub behavior: LayoutBehavior,
    pub children: Vec<LayoutId>,
    pub parent: Option<LayoutId>,

    // Layout results
    pub constraints: Option<LayoutConstraints>,
    pub computed_size: Option<Size>,
    pub frame: Option<Rect>,
}

impl LayoutNode {
    pub fn new(id: LayoutId, behavior: LayoutBehavior) -> Self {
        Self {
            id,
            behavior,
            children: Vec::new(),
            parent: None,
            constraints: None,
            computed_size: None,
            frame: None,
        }
    }
}

/// The main layout engine that performs SwiftUI-like layout calculations
pub struct LayoutEngine {
    nodes: HashMap<LayoutId, LayoutNode>,
    next_id: usize,
    root: Option<LayoutId>,
}

impl LayoutEngine {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            next_id: 0,
            root: None,
        }
    }

    /// Create a new layout node and return its ID
    pub fn create_node(&mut self, behavior: LayoutBehavior) -> LayoutId {
        let id = LayoutId::new(self.next_id);
        self.next_id += 1;

        let node = LayoutNode::new(id, behavior);
        self.nodes.insert(id, node);
        id
    }

    /// Add a child to a parent node
    pub fn add_child(&mut self, parent_id: LayoutId, child_id: LayoutId) {
        if let Some(parent) = self.nodes.get_mut(&parent_id) {
            parent.children.push(child_id);
        }

        if let Some(child) = self.nodes.get_mut(&child_id) {
            child.parent = Some(parent_id);
        }
    }

    /// Set the root node
    pub fn set_root(&mut self, root_id: LayoutId) {
        self.root = Some(root_id);
    }

    /// Perform layout calculation starting from the root
    pub fn layout(&mut self, available_size: Size) -> Option<Size> {
        if let Some(root_id) = self.root {
            let constraints = LayoutConstraints::loose(available_size);
            self.layout_node(root_id, constraints)
        } else {
            None
        }
    }

    /// Layout a specific node with given constraints
    fn layout_node(&mut self, node_id: LayoutId, constraints: LayoutConstraints) -> Option<Size> {
        let behavior = self.nodes.get(&node_id)?.behavior.clone();
        let children = self.nodes.get(&node_id)?.children.clone();

        // Store constraints on the node
        if let Some(node) = self.nodes.get_mut(&node_id) {
            node.constraints = Some(constraints);
        }

        let size = match behavior {
            LayoutBehavior::VStack { alignment, spacing } => {
                self.layout_vstack(node_id, &children, alignment, spacing, constraints)
            }
            LayoutBehavior::HStack { alignment, spacing } => {
                self.layout_hstack(node_id, &children, alignment, spacing, constraints)
            }
            LayoutBehavior::ZStack { alignment } => {
                self.layout_zstack(node_id, &children, alignment, constraints)
            }
            LayoutBehavior::Leaf => {
                // For leaf nodes, we assume they want to be as small as possible
                // In a real implementation, this would query the widget's intrinsic size
                constraints.constrain(Size::new(100.0, 30.0))
            }
            LayoutBehavior::ScrollView => {
                // ScrollView takes all available space
                constraints.max_size
            }
        };

        // Store computed size
        if let Some(node) = self.nodes.get_mut(&node_id) {
            node.computed_size = Some(size);
        }

        Some(size)
    }

    /// Layout a vertical stack
    fn layout_vstack(
        &mut self,
        _parent_id: LayoutId,
        children: &[LayoutId],
        _alignment: HorizontalAlignment,
        spacing: f32,
        constraints: LayoutConstraints,
    ) -> Size {
        if children.is_empty() {
            return Size::zero();
        }

        let mut total_height = 0.0_f32;
        let mut max_width = 0.0_f32;
        let spacing_total = (children.len().saturating_sub(1) as f32) * spacing;

        // First pass: layout children with unconstrained height
        let available_height = (constraints.max_size.height - spacing_total).max(0.0);
        let child_height = available_height / children.len() as f32;

        for &child_id in children {
            let child_constraints = LayoutConstraints {
                min_size: Size::new(0.0, 0.0),
                max_size: Size::new(constraints.max_size.width, child_height),
            };

            if let Some(child_size) = self.layout_node(child_id, child_constraints) {
                total_height += child_size.height;
                max_width = max_width.max(child_size.width);
            }
        }

        total_height += spacing_total;

        constraints.constrain(Size::new(max_width, total_height))
    }

    /// Layout a horizontal stack
    fn layout_hstack(
        &mut self,
        _parent_id: LayoutId,
        children: &[LayoutId],
        _alignment: VerticalAlignment,
        spacing: f32,
        constraints: LayoutConstraints,
    ) -> Size {
        if children.is_empty() {
            return Size::zero();
        }

        let mut total_width = 0.0_f32;
        let mut max_height = 0.0_f32;
        let spacing_total = (children.len().saturating_sub(1) as f32) * spacing;

        // First pass: layout children with unconstrained width
        let available_width = (constraints.max_size.width - spacing_total).max(0.0);
        let child_width = available_width / children.len() as f32;

        for &child_id in children {
            let child_constraints = LayoutConstraints {
                min_size: Size::new(0.0, 0.0),
                max_size: Size::new(child_width, constraints.max_size.height),
            };

            if let Some(child_size) = self.layout_node(child_id, child_constraints) {
                total_width += child_size.width;
                max_height = max_height.max(child_size.height);
            }
        }

        total_width += spacing_total;

        constraints.constrain(Size::new(total_width, max_height))
    }

    /// Layout a layered stack  
    fn layout_zstack(
        &mut self,
        _parent_id: LayoutId,
        children: &[LayoutId],
        _alignment: Alignment,
        constraints: LayoutConstraints,
    ) -> Size {
        if children.is_empty() {
            return Size::zero();
        }

        let mut max_width = 0.0_f32;
        let mut max_height = 0.0_f32;

        // All children get the same constraints
        for &child_id in children {
            if let Some(child_size) = self.layout_node(child_id, constraints) {
                max_width = max_width.max(child_size.width);
                max_height = max_height.max(child_size.height);
            }
        }

        constraints.constrain(Size::new(max_width, max_height))
    }

    /// Position all nodes after layout is complete
    pub fn position(&mut self, container_rect: Rect) {
        if let Some(root_id) = self.root {
            self.position_node(root_id, container_rect);
        }
    }

    /// Position a specific node and its children
    fn position_node(&mut self, node_id: LayoutId, container: Rect) {
        let (behavior, children, size) = {
            let node = match self.nodes.get(&node_id) {
                Some(n) => n,
                None => return,
            };
            (
                node.behavior.clone(),
                node.children.clone(),
                node.computed_size.unwrap_or(Size::zero()),
            )
        };

        // Set this node's frame
        if let Some(node) = self.nodes.get_mut(&node_id) {
            node.frame = Some(Rect {
                origin: container.origin,
                size,
            });
        }

        // Position children based on layout behavior
        match behavior {
            LayoutBehavior::VStack { alignment, spacing } => {
                self.position_vstack_children(node_id, &children, alignment, spacing, container);
            }
            LayoutBehavior::HStack { alignment, spacing } => {
                self.position_hstack_children(node_id, &children, alignment, spacing, container);
            }
            LayoutBehavior::ZStack { alignment } => {
                self.position_zstack_children(node_id, &children, alignment, container);
            }
            _ => {} // Leaf nodes don't have children to position
        }
    }

    fn position_vstack_children(
        &mut self,
        _parent_id: LayoutId,
        children: &[LayoutId],
        alignment: HorizontalAlignment,
        spacing: f32,
        container: Rect,
    ) {
        let mut y = container.origin.y;

        for &child_id in children {
            let child_size = self
                .nodes
                .get(&child_id)
                .and_then(|n| n.computed_size)
                .unwrap_or(Size::zero());

            let x = match alignment {
                HorizontalAlignment::Leading => container.origin.x,
                HorizontalAlignment::Center => {
                    container.origin.x + (container.size.width - child_size.width) / 2.0
                }
                HorizontalAlignment::Trailing => {
                    container.origin.x + container.size.width - child_size.width
                }
            };

            let child_container = Rect::new(x, y, child_size.width, child_size.height);
            self.position_node(child_id, child_container);

            y += child_size.height + spacing;
        }
    }

    fn position_hstack_children(
        &mut self,
        _parent_id: LayoutId,
        children: &[LayoutId],
        alignment: VerticalAlignment,
        spacing: f32,
        container: Rect,
    ) {
        let mut x = container.origin.x;

        for &child_id in children {
            let child_size = self
                .nodes
                .get(&child_id)
                .and_then(|n| n.computed_size)
                .unwrap_or(Size::zero());

            let y = match alignment {
                VerticalAlignment::Top => container.origin.y,
                VerticalAlignment::Center => {
                    container.origin.y + (container.size.height - child_size.height) / 2.0
                }
                VerticalAlignment::Bottom => {
                    container.origin.y + container.size.height - child_size.height
                }
            };

            let child_container = Rect::new(x, y, child_size.width, child_size.height);
            self.position_node(child_id, child_container);

            x += child_size.width + spacing;
        }
    }

    fn position_zstack_children(
        &mut self,
        _parent_id: LayoutId,
        children: &[LayoutId],
        alignment: Alignment,
        container: Rect,
    ) {
        for &child_id in children {
            let child_size = self
                .nodes
                .get(&child_id)
                .and_then(|n| n.computed_size)
                .unwrap_or(Size::zero());

            let x = match alignment.horizontal {
                HorizontalAlignment::Leading => container.origin.x,
                HorizontalAlignment::Center => {
                    container.origin.x + (container.size.width - child_size.width) / 2.0
                }
                HorizontalAlignment::Trailing => {
                    container.origin.x + container.size.width - child_size.width
                }
            };

            let y = match alignment.vertical {
                VerticalAlignment::Top => container.origin.y,
                VerticalAlignment::Center => {
                    container.origin.y + (container.size.height - child_size.height) / 2.0
                }
                VerticalAlignment::Bottom => {
                    container.origin.y + container.size.height - child_size.height
                }
            };

            let child_container = Rect::new(x, y, child_size.width, child_size.height);
            self.position_node(child_id, child_container);
        }
    }

    /// Get the computed frame for a node
    pub fn get_frame(&self, node_id: LayoutId) -> Option<Rect> {
        self.nodes.get(&node_id)?.frame
    }

    /// Get a reference to a node
    pub fn get_node(&self, node_id: LayoutId) -> Option<&LayoutNode> {
        self.nodes.get(&node_id)
    }
}

impl Default for LayoutEngine {
    fn default() -> Self {
        Self::new()
    }
}
