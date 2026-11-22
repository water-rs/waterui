//! The text-input document of a GPU surface's `InputView` on `UIKit`.
//!
//! A GPU surface's content takes text as an event stream — committed text,
//! and a composition's start, updates and commit or cancel — not as a
//! document. [`GpuSurfaceTextClient`] adapts that stream to the
//! `UITextInput` document model: the only text it holds is the input
//! method's marked text, every edit `UIKit` makes becomes the matching
//! content event, and the content's own caret places the candidate UI.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use cocoa_ui::Rect;
use cocoa_ui::input::SurfaceEvent;
use cocoa_ui::input::{Code, Key, Modifiers, NamedKey};
use cocoa_ui::uikit::input_view::InputView;
use cocoa_ui::uikit::text_input::{TextInputClient, TextInputTraits, previous_grapheme};
use objc2::rc::Weak;

/// The composition the input method is marking, in UTF-16 units.
#[derive(Debug, Default)]
struct Marked {
    text: String,
    /// The selection within `text`.
    selected: Range<usize>,
}

/// The `UITextInput` document of a GPU surface: its marked text.
pub struct GpuSurfaceTextClient {
    deliver: Rc<dyn Fn(SurfaceEvent)>,
    caret: Box<dyn Fn() -> Option<Rect>>,
    view: Weak<InputView>,
    marked: RefCell<Option<Marked>>,
}

impl std::fmt::Debug for GpuSurfaceTextClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuSurfaceTextClient")
            .field("marked", &self.marked)
            .finish_non_exhaustive()
    }
}

impl GpuSurfaceTextClient {
    /// A client delivering the content events through `deliver`, placing
    /// the input method's UI at `caret` and, without one, at `view`'s
    /// bounds.
    pub fn new(
        deliver: Rc<dyn Fn(SurfaceEvent)>,
        caret: impl Fn() -> Option<Rect> + 'static,
        view: Weak<InputView>,
    ) -> Self {
        Self {
            deliver,
            caret: Box::new(caret),
            view,
            marked: RefCell::new(None),
        }
    }

    fn place(&self) -> Rect {
        (self.caret)().unwrap_or_else(|| {
            self.view.load().map_or(Rect::ZERO, |view| {
                let bounds = view.bounds();
                Rect::new(
                    bounds.origin.x,
                    bounds.origin.y,
                    bounds.size.width,
                    bounds.size.height,
                )
            })
        })
    }
}

/// `text` as UTF-16 code units, sliced to `range`.
fn utf16_slice(text: &str, range: Range<usize>) -> String {
    let units: Vec<u16> = text.encode_utf16().collect();
    String::from_utf16_lossy(&units[range.start.min(units.len())..range.end.min(units.len())])
}

/// The byte offset of `utf16_offset` code units into `text`.
fn byte_offset(text: &str, utf16_offset: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units >= utf16_offset {
            return byte;
        }
        units += ch.len_utf16();
    }
    text.len()
}

impl TextInputClient for GpuSurfaceTextClient {
    fn is_editing(&self) -> bool {
        // The content reads hardware keys itself.
        false
    }

    fn traits(&self) -> TextInputTraits {
        TextInputTraits::default()
    }

    fn len(&self) -> usize {
        self.marked
            .borrow()
            .as_ref()
            .map_or(0, |marked| marked.text.encode_utf16().count())
    }

    fn text(&self, range: Range<usize>) -> String {
        self.marked
            .borrow()
            .as_ref()
            .map_or_else(String::new, |marked| utf16_slice(&marked.text, range))
    }

    fn selection(&self) -> Range<usize> {
        self.marked
            .borrow()
            .as_ref()
            .map_or(0..0, |marked| marked.selected.clone())
    }

    fn set_selection(&self, range: Range<usize>) {
        if let Some(marked) = self.marked.borrow_mut().as_mut() {
            marked.selected = range;
        }
    }

    fn marked_range(&self) -> Option<Range<usize>> {
        (self.len() > 0).then(|| 0..self.len())
    }

    fn set_marked_text(&self, text: &str, selected: Range<usize>) {
        let started = self.marked.borrow().is_none();
        let caret = byte_offset(text, selected.start);
        self.marked.replace(Some(Marked {
            text: text.to_owned(),
            selected,
        }));
        if started {
            (self.deliver)(SurfaceEvent::CompositionStart);
        }
        (self.deliver)(SurfaceEvent::CompositionUpdate {
            text: text.to_owned(),
            caret: Some(caret),
        });
    }

    fn unmark_text(&self) {
        let marked = self.marked.take();
        if let Some(marked) = marked {
            (self.deliver)(SurfaceEvent::CompositionCommit(marked.text));
        }
    }

    fn replace(&self, _range: Range<usize>, text: &str) {
        let composing = self.marked.take().is_some();
        (self.deliver)(match (composing, text.is_empty()) {
            (true, true) => SurfaceEvent::CompositionCancel,
            (true, false) => SurfaceEvent::CompositionCommit(text.to_owned()),
            (false, _) => SurfaceEvent::TextInput(text.to_owned()),
        });
    }

    fn delete_backward(&self) {
        let Some(marked) = self
            .marked
            .borrow()
            .as_ref()
            .map(|marked| (marked.text.clone(), marked.selected.clone()))
        else {
            // Outside a composition the content owns the document: the
            // delete key is a Backspace press, as it always was.
            for pressed in [true, false] {
                (self.deliver)(SurfaceEvent::Key {
                    pressed,
                    key: Key::Named(NamedKey::Backspace),
                    code: Code::Backspace,
                    modifiers: Modifiers::empty(),
                    repeat: false,
                });
            }
            return;
        };
        let (text, selected) = marked;
        let range = if selected.is_empty() {
            if selected.start == 0 {
                return;
            }
            previous_grapheme(self, selected.start)
        } else {
            selected
        };
        let mut units: Vec<u16> = text.encode_utf16().collect();
        units.drain(range.clone());
        if units.is_empty() {
            self.marked.replace(None);
            (self.deliver)(SurfaceEvent::CompositionCancel);
            return;
        }
        let text = String::from_utf16_lossy(&units);
        let caret = byte_offset(&text, range.start);
        self.marked.replace(Some(Marked {
            text: text.clone(),
            selected: range.start..range.start,
        }));
        (self.deliver)(SurfaceEvent::CompositionUpdate {
            text,
            caret: Some(caret),
        });
    }

    fn first_rect(&self, _range: Range<usize>) -> Rect {
        self.place()
    }

    fn caret_rect(&self, _offset: usize) -> Rect {
        self.place()
    }

    fn selection_rects(&self, _range: Range<usize>) -> Vec<Rect> {
        Vec::new()
    }

    fn closest_offset(&self, _point: kurbo::Point) -> usize {
        self.selection().start
    }

    fn perform_return(&self) {
        (self.deliver)(SurfaceEvent::TextInput("\n".to_owned()));
    }
}
