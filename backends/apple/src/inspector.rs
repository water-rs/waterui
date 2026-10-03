//! Native inspection gestures and accessibility-tree publication.

use std::rc::Rc;

use cocoa_ui::PlatformView;
use objc2::rc::Weak;
use waterui::{
    Environment,
    inspector::{
        InspectorRuntime, TreeRecorder,
        protocol::{Bounds, NodeId, NodeState, TreeNode},
    },
};

use crate::contract::KeepAlive;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

struct Inspection {
    env: Environment,
    root: Weak<PlatformView>,
    #[cfg(target_os = "ios")]
    gesture: std::cell::RefCell<Option<cocoa_ui::uikit::gesture::GestureAttachment>>,
}

fn identifier(view: &PlatformView) -> NodeId {
    NodeId(core::ptr::from_ref(view).addr() as u64)
}

impl Inspection {
    fn publish(&self, root: &PlatformView) {
        let recorder = self
            .env
            .get::<TreeRecorder>()
            .expect("inspector installs its tree recorder");
        if !recorder.is_active() {
            return;
        }
        let mut nodes = Vec::new();
        walk(root, &mut nodes);
        recorder.record_snapshot(identifier(root), None, nodes);
    }

    fn inspect(&self, view: &PlatformView, root: &PlatformView) {
        self.publish(root);
        self.env
            .get::<InspectorRuntime>()
            .expect("inspection is installed only with an inspector runtime")
            .inspect_node(identifier(view));
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "inspector bounds use f32 platform points"
)]
fn walk(view: &PlatformView, nodes: &mut Vec<TreeNode>) {
    let children = view.subviews();
    let frame = view.convertRect_toView(view.bounds(), None);
    let (role, label, enabled) = accessibility(view);
    nodes.push(TreeNode {
        id: identifier(view),
        role,
        label: label.filter(|label| !label.is_empty()),
        value: None,
        bounds: Some(Bounds {
            x: frame.origin.x as f32,
            y: frame.origin.y as f32,
            width: frame.size.width as f32,
            height: frame.size.height as f32,
        }),
        state: NodeState {
            enabled,
            hidden: view.isHidden(),
            ..NodeState::default()
        },
        children: children.iter().map(|child| identifier(&child)).collect(),
    });
    for child in children {
        walk(&child, nodes);
    }
}

#[cfg(target_os = "macos")]
fn accessibility(view: &PlatformView) -> (String, Option<String>, bool) {
    use cocoa_ui::objc2_app_kit::NSAccessibility;
    use objc2::runtime::ProtocolObject;
    let view = ProtocolObject::<dyn NSAccessibility>::from_ref(view);
    (
        view.accessibilityRole()
            .map_or_else(|| "group".into(), |role| role.to_string().to_lowercase()),
        view.accessibilityLabel().map(|label| label.to_string()),
        view.isAccessibilityEnabled(),
    )
}

#[cfg(target_os = "ios")]
fn accessibility(view: &PlatformView) -> (String, Option<String>, bool) {
    use cocoa_ui::objc2_ui_kit::NSObjectUIAccessibility;
    use objc2::MainThreadOnly;
    (
        "group".into(),
        view.accessibilityLabel(view.mtm())
            .map(|label| label.to_string()),
        view.isUserInteractionEnabled(),
    )
}

/// Installs interactions for the root's lifetime. Only an inspect action publishes
/// a tree, using the same root that supplied the selected node.
pub fn install(root: &HostView, env: &Environment, keepalive: &mut KeepAlive) {
    if env.get::<InspectorRuntime>().is_none() {
        return;
    }
    let inspection = Rc::new(Inspection {
        env: env.clone(),
        root: Weak::new(root.as_ref()),
        #[cfg(target_os = "ios")]
        gesture: std::cell::RefCell::default(),
    });
    install_interaction(root, &inspection);
    keepalive.keep(inspection);
}

#[cfg(target_os = "macos")]
fn install_interaction(root: &HostView, inspection: &Rc<Inspection>) {
    use cocoa_ui::{
        appkit::ContextMenu,
        menu::{Command, MenuTreeNode},
    };
    use objc2::MainThreadOnly;
    let weak = Rc::downgrade(inspection);
    root.set_right_mouse_handler(move |host, event| {
        let Some(inspection) = weak.upgrade() else {
            return;
        };
        let point = event.locationInWindow();
        let action = Rc::downgrade(&inspection);
        let menu = ContextMenu::new(
            host.mtm(),
            &[MenuTreeNode::Command(
                Command {
                    label: "Inspect Element".into(),
                    enabled: true,
                    ..Command::default()
                },
                Rc::new(move || {
                    let Some(inspection) = action.upgrade() else {
                        return;
                    };
                    let Some(host) = inspection.root.load() else {
                        return;
                    };
                    let Some(root) = host.window().and_then(|window| window.contentView()) else {
                        return;
                    };
                    // SAFETY: the retained root is a live NSView accessed on
                    // AppKit's main thread for the duration of this menu action.
                    let parent = unsafe { root.superview() };
                    let point =
                        parent.map_or(point, |parent| parent.convertPoint_fromView(point, None));
                    let hit = root.hitTest(point).unwrap_or_else(|| root.clone());
                    inspection.inspect(&hit, &root);
                }),
            )],
            || {},
            || {},
        );
        menu.pop_up(host, event);
    });
}

#[cfg(target_os = "ios")]
fn install_interaction(root: &HostView, inspection: &Rc<Inspection>) {
    use cocoa_ui::{
        gesture::{ButtonMask, GestureState},
        objc2_ui_kit::{UIGestureRecognizer, UILongPressGestureRecognizer},
        uikit::gesture,
    };
    use objc2::MainThreadOnly;
    let weak = Rc::downgrade(inspection);
    let recognizer = Rc::new(std::cell::OnceCell::<Weak<UIGestureRecognizer>>::new());
    let observed = recognizer.clone();
    // Ask UIKit for its default threshold, matching an unconfigured recognizer.
    let duration = UILongPressGestureRecognizer::new(root.mtm()).minimumPressDuration();
    let gesture = gesture::long_press(root, duration, ButtonMask::PRIMARY, move |state| {
        if state != GestureState::Began {
            return;
        }
        let Some(inspection) = weak.upgrade() else {
            return;
        };
        let Some(root) = inspection.root.load() else {
            return;
        };
        let Some(recognizer) = observed.get().and_then(Weak::load) else {
            return;
        };
        let hit = root
            .hitTest_withEvent(recognizer.locationInView(Some(&root)), None)
            .unwrap_or_else(|| root.clone());
        inspection.inspect(&hit, &root);
    });
    let native = gesture
        .recognizer()
        .expect("new gesture remains attached to the root");
    // This touch chord has no pointer-button restriction, matching UIKit's
    // unfiltered long-press recognizer. The attachment still owns its target.
    native.setDelegate(None);
    native
        .downcast_ref::<UILongPressGestureRecognizer>()
        .expect("long_press installs a long-press recognizer")
        .setNumberOfTouchesRequired(2);
    native.setCancelsTouchesInView(false);
    recognizer
        .set(Weak::new(&native))
        .expect("recognizer is assigned once");
    *inspection.gesture.borrow_mut() = Some(gesture);
}

#[cfg(target_os = "ios")]
impl Drop for Inspection {
    fn drop(&mut self) {
        if let Some(gesture) = self.gesture.get_mut().take()
            && let (Some(root), Some(recognizer)) = (self.root.load(), gesture.recognizer())
        {
            root.removeGestureRecognizer(&recognizer);
        }
    }
}
