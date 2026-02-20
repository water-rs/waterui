//! View renderer that dispatches `WaterUI` views to GTK widgets.

use gtk4::Widget;
use gtk4::prelude::*;
use waterui::component::list::ListConfig;
use waterui::component::progress::ProgressConfig;
use waterui::prelude::Divider;
use waterui_backend_core::ViewDispatcher;
use waterui_controls::button::ButtonConfig;
use waterui_controls::slider::SliderConfig;
use waterui_controls::stepper::StepperConfig;
use waterui_controls::text_field::TextFieldConfig;
use waterui_controls::toggle::ToggleConfig;
use waterui_core::dynamic::Dynamic;
use waterui_core::metadata::MetadataKey;
use waterui_core::{AnyView, Environment, Native, View};
use waterui_core::{IgnorableMetadata, Metadata, Retain, Str};
use waterui_form::picker::PickerConfig;
use waterui_form::secure::SecureFieldConfig;
use waterui_graphics::gpu_surface::GpuSurface;
use waterui_layout::container::{FixedContainer, LazyContainer};
use waterui_layout::padding::Padding;
use waterui_layout::scroll::ScrollView;
use waterui_layout::spacer::Spacer;
use waterui_navigation::tab::Tabs;
use waterui_navigation::{NavigationStack, NavigationView};
use waterui_text::TextConfig;
use waterui_webview::WebView;

use crate::component::GtkComponent;

/// Context passed to component renderers.
#[derive(Debug, Clone)]
pub struct RenderContext {
    /// Reference to the renderer for recursive rendering.
    /// This is a raw pointer because we can't have self-referential borrows.
    renderer_ptr: *mut GtkRenderer,
}

impl Default for RenderContext {
    fn default() -> Self {
        Self {
            renderer_ptr: std::ptr::null_mut(),
        }
    }
}

impl RenderContext {
    /// Creates a context with a renderer reference.
    fn with_renderer(renderer: &mut GtkRenderer) -> Self {
        Self {
            renderer_ptr: renderer as *mut GtkRenderer,
        }
    }

    /// Gets a mutable reference to the renderer.
    ///
    /// # Safety
    ///
    /// The caller must ensure the renderer pointer is valid.
    pub(crate) unsafe fn renderer(&self) -> Option<&mut GtkRenderer> {
        if self.renderer_ptr.is_null() {
            None
        } else {
            // SAFETY: Caller ensures the pointer is valid for the duration of the borrow.
            Some(unsafe { &mut *self.renderer_ptr })
        }
    }
}

/// GTK renderer that converts `WaterUI` views to GTK widgets.
#[derive(Debug)]
pub struct GtkRenderer {
    dispatcher: ViewDispatcher<(), RenderContext, Widget>,
}

impl GtkRenderer {
    /// Creates a new GTK renderer with all component handlers registered.
    #[must_use]
    pub fn new() -> Self {
        let mut dispatcher = ViewDispatcher::new();

        // Register component handlers
        Self::register_components(&mut dispatcher);

        Self { dispatcher }
    }

    /// Renders a view to a GTK widget.
    pub fn render<V: View>(&mut self, view: V, env: &Environment) -> Widget {
        let ctx = RenderContext::with_renderer(self);
        self.dispatcher.dispatch(view, env, ctx)
    }

    /// Renders an `AnyView` to a GTK widget.
    pub fn render_any(&mut self, view: AnyView, env: &Environment) -> Widget {
        let ctx = RenderContext::with_renderer(self);
        self.dispatcher.dispatch_any(view, env, ctx)
    }

    fn register_components(dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>) {
        // Register Native<T> wrapped components
        Self::register_native::<TextConfig>(dispatcher);
        Self::register_native::<Spacer>(dispatcher);
        Self::register_native::<FixedContainer>(dispatcher);
        Self::register_native::<LazyContainer>(dispatcher);
        Self::register_native::<ButtonConfig>(dispatcher);
        Self::register_native::<ToggleConfig>(dispatcher);
        Self::register_native::<SliderConfig>(dispatcher);
        Self::register_native::<TextFieldConfig>(dispatcher);
        Self::register_native::<ProgressConfig>(dispatcher);
        Self::register_native::<StepperConfig>(dispatcher);
        Self::register_native::<ScrollView>(dispatcher);
        Self::register_native::<Tabs>(dispatcher);
        Self::register_native::<ListConfig>(dispatcher);
        Self::register_native::<SecureFieldConfig>(dispatcher);
        Self::register_native::<PickerConfig>(dispatcher);
        Self::register_native::<WebView>(dispatcher);

        // Register Dynamic for reactive content
        Self::register::<Native<Dynamic>>(dispatcher);

        // Register GPU surface (used by waterui-graphics and waterui-media)
        Self::register_native::<GpuSurface>(dispatcher);

        // Register views that implement View directly
        Self::register::<Divider>(dispatcher);
        Self::register::<NavigationView>(dispatcher);
        Self::register::<NavigationStack<(), ()>>(dispatcher);

        // Register metadata handlers
        Self::register_metadata_handlers(dispatcher);

        // Register Padding to apply margins (before it becomes FixedContainer)
        Self::register_padding_handler(dispatcher);

        // Register Str directly (before it wraps into Native<Str>)
        Self::register_str_handler(dispatcher);

        // Register unit type () as empty widget
        Self::register_unit_handler(dispatcher);
    }

    /// Registers a handler for `Padding` that applies GTK margins.
    fn register_padding_handler(dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>) {
        dispatcher.register::<Padding>(|_state, ctx, padding, env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            let (edges, content) = padding.into_inner();

            // Render the content
            let widget = renderer.render_any(content, env);

            // Apply margins from EdgeInsets
            #[allow(clippy::cast_possible_truncation)]
            {
                widget.set_margin_top(edges.top() as i32);
                widget.set_margin_bottom(edges.bottom() as i32);
                widget.set_margin_start(edges.leading() as i32);
                widget.set_margin_end(edges.trailing() as i32);
            }

            widget
        });
    }

    /// Registers a handler for `Str` that renders it as a GTK Label.
    fn register_str_handler(dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>) {
        dispatcher.register::<Str>(|_state, _ctx, s, _env| {
            let label = gtk4::Label::new(Some(s.as_str()));
            // Let text maintain natural width - layout system handles sizing
            label.upcast()
        });
    }

    /// Registers a handler for unit type `()` as an empty widget.
    fn register_unit_handler(dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>) {
        dispatcher.register::<Native<()>>(|_state, _ctx, _unit, _env| {
            // Return an empty widget (invisible box with no children)
            let empty = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            empty.set_visible(false);
            empty.upcast()
        });
    }

    /// Registers handlers for metadata wrapper views.
    fn register_metadata_handlers(dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>) {
        use waterui::accessibility::{AccessibilityLabel, AccessibilityRole};
        use waterui::component::focus::Focused;
        use waterui::gesture::GestureObserver;
        use waterui::metadata::secure::{HighDynamicRange, Secure, StandardDynamicRange};
        use waterui::style::Shadow;
        use waterui_core::event::OnEvent;
        use waterui_layout::safe_area::IgnoreSafeArea;

        // Metadata<Environment> - use provided environment for subtree
        dispatcher.register::<Metadata<Environment>>(|_state, ctx, metadata, _env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            renderer.render_any(metadata.content, &metadata.value)
        });

        // Metadata<Retain> - just render content (the value stays alive in the struct)
        dispatcher.register::<Metadata<Retain>>(|_state, ctx, metadata, env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            renderer.render_any(metadata.content, env)
        });

        // Metadata<Shadow> - apply shadow and render content
        dispatcher.register::<Metadata<Shadow>>(|_state, ctx, metadata, env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            let widget = renderer.render_any(metadata.content, env);
            // TODO: Apply shadow styling (GTK CSS or custom drawing)
            widget
        });

        // Metadata<Focused> - handle focus state
        dispatcher.register::<Metadata<Focused>>(|_state, ctx, metadata, env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            let widget = renderer.render_any(metadata.content, env);
            // TODO: Set up focus handling
            widget
        });

        // Metadata<Secure> - mark content as secure (e.g., password fields)
        dispatcher.register::<Metadata<Secure>>(|_state, ctx, metadata, env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            renderer.render_any(metadata.content, env)
        });

        // Metadata<StandardDynamicRange> - render content with SDR color handling
        dispatcher.register::<Metadata<StandardDynamicRange>>(|_state, ctx, metadata, env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            renderer.render_any(metadata.content, env)
        });

        // Metadata<HighDynamicRange> - render content with HDR color handling
        dispatcher.register::<Metadata<HighDynamicRange>>(|_state, ctx, metadata, env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            renderer.render_any(metadata.content, env)
        });

        // Metadata<OnEvent> - handle events
        dispatcher.register::<Metadata<OnEvent>>(|_state, ctx, metadata, env| {
            use std::cell::RefCell;
            use std::rc::Rc;
            use waterui_core::event::Event;

            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            let widget = renderer.render_any(metadata.content, env);
            widget.set_can_target(true);

            let expected = metadata.value.event();
            let handler = Rc::new(RefCell::new(metadata.value));
            let env = env.clone();
            let motion = gtk4::EventControllerMotion::new();

            match expected {
                Event::HoverEnter => {
                    let env = env.clone();
                    let handler = handler.clone();
                    motion.connect_enter(move |_, _, _| {
                        if let Ok(mut on_event) = handler.try_borrow_mut() {
                            on_event.handle(&env);
                        }
                    });
                }
                Event::HoverExit => {
                    let env = env.clone();
                    let handler = handler.clone();
                    motion.connect_leave(move |_| {
                        if let Ok(mut on_event) = handler.try_borrow_mut() {
                            on_event.handle(&env);
                        }
                    });
                }
                _ => panic!("unsupported OnEvent variant on GTK backend"),
            }

            widget.add_controller(motion);
            widget
        });

        // Metadata<GestureObserver> - handle gestures
        dispatcher.register::<Metadata<GestureObserver>>(|_state, ctx, metadata, env| {
            use std::cell::RefCell;
            use std::rc::Rc;
            use waterui::gesture::{
                DragEvent, Gesture, GesturePhase, GesturePoint, LongPressEvent, MagnificationEvent,
                TapEvent,
            };

            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            let widget = renderer.render_any(metadata.content, env);

            let gesture = metadata.value.gesture;
            // Wrap action in Rc<RefCell> so it can be shared across closures
            let action = Rc::new(RefCell::new(metadata.value.action));
            let env = env.clone();

            // Make sure widget can receive pointer events
            widget.set_can_target(true);

            match gesture {
                Gesture::Tap(tap) => {
                    let click = gtk4::GestureClick::new();
                    click.set_button(1); // Left mouse button
                    click.set_propagation_phase(gtk4::PropagationPhase::Capture);

                    let env = env.clone();
                    let action = action.clone();
                    let required_count = tap.count;

                    click.connect_pressed(move |gesture, n_press, x, y| {
                        tracing::debug!(
                            "[GestureObserver] Tap pressed: n_press={}, required={}",
                            n_press,
                            required_count
                        );
                        if n_press as u32 >= required_count {
                            let tap_event = TapEvent {
                                location: GesturePoint::new(x as f32, y as f32),
                                count: n_press as u32,
                            };

                            let mut env = env.clone();
                            env.insert(tap_event);

                            if let Ok(mut handler) = action.try_borrow_mut() {
                                (&mut **handler)(&env);
                            }
                            gesture.set_state(gtk4::EventSequenceState::Claimed);
                        }
                    });

                    widget.add_controller(click);
                }

                Gesture::LongPress(long_press) => {
                    let press = gtk4::GestureLongPress::new();
                    // delay_factor is a multiplier: 1.0 = 500ms default, 2.0 = 1000ms
                    press.set_delay_factor(long_press.duration as f64 / 500.0);
                    press.set_propagation_phase(gtk4::PropagationPhase::Capture);

                    let env = env.clone();
                    let action = action.clone();
                    let duration = long_press.duration;

                    press.connect_pressed(move |gesture, x, y| {
                        tracing::debug!("[GestureObserver] Long press triggered");
                        let event = LongPressEvent {
                            location: GesturePoint::new(x as f32, y as f32),
                            duration: duration as f32,
                        };

                        let mut env = env.clone();
                        env.insert(event);

                        if let Ok(mut handler) = action.try_borrow_mut() {
                            (&mut **handler)(&env);
                        }
                        gesture.set_state(gtk4::EventSequenceState::Claimed);
                    });

                    widget.add_controller(press);
                }

                Gesture::Drag(drag) => {
                    let drag_gesture = gtk4::GestureDrag::new();
                    drag_gesture.set_propagation_phase(gtk4::PropagationPhase::Capture);
                    let min_distance = drag.min_distance;

                    let drag_started = Rc::new(RefCell::new(false));

                    // Drag begin
                    {
                        let env = env.clone();
                        let action = action.clone();
                        let drag_started = drag_started.clone();

                        drag_gesture.connect_drag_begin(move |gesture, x, y| {
                            *drag_started.borrow_mut() = false;

                            let event = DragEvent {
                                phase: GesturePhase::Started,
                                location: GesturePoint::new(x as f32, y as f32),
                                translation: GesturePoint::new(0.0, 0.0),
                                velocity: GesturePoint::new(0.0, 0.0),
                            };

                            let mut env = env.clone();
                            env.insert(event);

                            if let Ok(mut handler) = action.try_borrow_mut() {
                                (&mut **handler)(&env);
                            }
                            gesture.set_state(gtk4::EventSequenceState::Claimed);
                        });
                    }

                    // Drag update
                    {
                        let env = env.clone();
                        let action = action.clone();
                        let drag_started = drag_started.clone();

                        drag_gesture.connect_drag_update(move |gesture, offset_x, offset_y| {
                            let distance =
                                (offset_x * offset_x + offset_y * offset_y).sqrt() as f32;
                            if distance < min_distance && !*drag_started.borrow() {
                                return;
                            }
                            *drag_started.borrow_mut() = true;

                            let start = gesture.start_point().unwrap_or((0.0, 0.0));

                            let event = DragEvent {
                                phase: GesturePhase::Updated,
                                location: GesturePoint::new(
                                    (start.0 + offset_x) as f32,
                                    (start.1 + offset_y) as f32,
                                ),
                                translation: GesturePoint::new(offset_x as f32, offset_y as f32),
                                velocity: GesturePoint::new(0.0, 0.0),
                            };

                            let mut env = env.clone();
                            env.insert(event);

                            if let Ok(mut handler) = action.try_borrow_mut() {
                                (&mut **handler)(&env);
                            }
                        });
                    }

                    // Drag end
                    {
                        let env = env.clone();
                        let action = action.clone();
                        let drag_started = drag_started.clone();

                        drag_gesture.connect_drag_end(move |gesture, offset_x, offset_y| {
                            if !*drag_started.borrow() {
                                return;
                            }

                            let start = gesture.start_point().unwrap_or((0.0, 0.0));

                            let event = DragEvent {
                                phase: GesturePhase::Ended,
                                location: GesturePoint::new(
                                    (start.0 + offset_x) as f32,
                                    (start.1 + offset_y) as f32,
                                ),
                                translation: GesturePoint::new(offset_x as f32, offset_y as f32),
                                velocity: GesturePoint::new(0.0, 0.0),
                            };

                            let mut env = env.clone();
                            env.insert(event);

                            if let Ok(mut handler) = action.try_borrow_mut() {
                                (&mut **handler)(&env);
                            }
                        });
                    }

                    widget.add_controller(drag_gesture);
                }

                Gesture::Magnification(_magnify) => {
                    let zoom = gtk4::GestureZoom::new();

                    let env = env.clone();
                    let action = action.clone();

                    zoom.connect_scale_changed(move |gesture, scale| {
                        let bbox = gesture.bounding_box();
                        let center = bbox
                            .map(|b| GesturePoint::new(b.x() as f32, b.y() as f32))
                            .unwrap_or(GesturePoint::new(0.0, 0.0));

                        let event = MagnificationEvent {
                            phase: GesturePhase::Updated,
                            center,
                            scale: scale as f32,
                            velocity: 0.0,
                        };

                        let mut env = env.clone();
                        env.insert(event);

                        if let Ok(mut handler) = action.try_borrow_mut() {
                            (&mut **handler)(&env);
                        }
                        gesture.set_state(gtk4::EventSequenceState::Claimed);
                    });

                    widget.add_controller(zoom);
                }

                Gesture::Rotation(_rotate) => {
                    // GTK4 rotation gesture - no RotationEvent type defined yet
                    let rotate = gtk4::GestureRotate::new();

                    let env = env.clone();
                    let action = action.clone();

                    rotate.connect_angle_changed(move |gesture, _angle, _delta| {
                        // Just call the handler without a specific event type
                        let env = env.clone();

                        if let Ok(mut handler) = action.try_borrow_mut() {
                            (&mut **handler)(&env);
                        }
                        gesture.set_state(gtk4::EventSequenceState::Claimed);
                    });

                    widget.add_controller(rotate);
                }

                Gesture::Then(_then) => {
                    tracing::warn!(
                        "[GestureObserver] Sequential gestures (Then) not fully implemented"
                    );
                }

                _ => {
                    tracing::warn!("[GestureObserver] Unhandled gesture type");
                }
            }

            widget
        });

        // Metadata<IgnoreSafeArea> - ignore safe area insets
        dispatcher.register::<Metadata<IgnoreSafeArea>>(|_state, ctx, metadata, env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            renderer.render_any(metadata.content, env)
        });

        // IgnorableMetadata handlers - these can safely just render the content
        Self::register_ignorable_metadata::<AccessibilityLabel>(dispatcher);
        Self::register_ignorable_metadata::<AccessibilityRole>(dispatcher);
    }

    /// Registers a handler for `IgnorableMetadata<T>` that just renders the content.
    fn register_ignorable_metadata<T: MetadataKey>(
        dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>,
    ) {
        dispatcher.register::<IgnorableMetadata<T>>(|_state, ctx, metadata, env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            renderer.render_any(metadata.content, env)
        });
    }

    /// Registers a `Native<T>` wrapped component with the dispatcher.
    fn register_native<T: waterui_core::NativeView + 'static>(
        dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>,
    ) where
        Native<T>: GtkComponent,
    {
        Self::register::<Native<T>>(dispatcher);
    }

    /// Registers a `GtkComponent` view type with the dispatcher.
    fn register<V: GtkComponent>(dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>) {
        dispatcher.register::<V>(|_state, ctx, view, env| {
            let renderer = unsafe { ctx.renderer() }.expect("renderer required");
            view.render(env, renderer)
        });
    }
}

impl Default for GtkRenderer {
    fn default() -> Self {
        Self::new()
    }
}
