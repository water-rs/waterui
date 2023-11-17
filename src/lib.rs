use std::{
    any::{type_name, TypeId},
    ops::Deref,
};

use reactive::{Binding, IntoBinding};

pub mod ffi;
pub mod reactive;

pub struct Text {
    pub text: Binding<String>,
}

pub trait Event<T>: 'static {}

pub struct Button {
    pub label: String,
    pub event: ButtonEvent,
}

pub struct ButtonEvent {
    pub on_click: Box<dyn Event<()>>,
}

impl Text {
    pub fn new(text: impl IntoBinding<String>) -> Self {
        Self {
            text: text.into_binding(),
        }
    }
}

impl Button {
    pub fn on_click(mut self, event: impl Event<()>) -> Self {
        self.event.on_click = Box::new(event);
        self
    }
}

macro_rules! native_implement {
    ($ty:ty) => {
        impl View for $ty {
            fn view(&self) -> Box<dyn View> {
                panic!("[Native implement]");
            }
        }
    };
}

struct TextField {
    pub text: Binding<String>,
}

native_implement!(Stack);
native_implement!(Text);

pub struct Stack {
    pub content: Vec<BoxView>,
}

macro_rules! stack {
    ($($view:expr),*) => {
        {
            let mut content=Vec::new();
            $(
                let view:Box<dyn View>=Box::new($view);
                content.push(view);
            )*
            Stack{content}
        }
    };
}

impl TextField {
    pub fn new(text: impl IntoBinding<String>) -> Self {
        Self {
            text: text.into_binding(),
        }
    }
}

pub trait View: 'static {
    fn view(&self) -> Box<dyn View>;
    fn name(&self) -> &'static str {
        type_name::<Self>()
    }

    #[doc(hidden)]
    fn type_id(&self, _sealed: sealed::Sealed) -> TypeId {
        TypeId::of::<Self>()
    }
}

mod sealed {
    pub struct Sealed;
}

impl dyn View {
    pub fn is<T: View>(&self) -> bool {
        self.type_id(sealed::Sealed) == TypeId::of::<T>()
    }

    pub fn downcast_ref<T: View>(&self) -> Option<&T> {
        if self.is::<T>() {
            unsafe { Some(&*(self as *const dyn View as *const T)) }
        } else {
            None
        }
    }

    pub fn downcast<T: View>(self: Box<Self>) -> Result<Box<T>, Box<dyn View>> {
        if self.is::<T>() {
            unsafe {
                let raw: *mut dyn View = Box::into_raw(self);
                Ok(Box::from_raw(raw as *mut T))
            }
        } else {
            Err(self)
        }
    }
}

impl View for Box<dyn View> {
    fn view(&self) -> Box<dyn View> {
        self.deref().view()
    }
}

impl View for TextField {
    fn view(&self) -> Box<dyn View> {
        panic!("[Native implement]");
    }
}

type BoxView = Box<dyn View>;

struct HtmlRenderer;

impl HtmlRenderer {
    pub fn render(view: impl View) -> String {
        let view = Box::new(view);
        let mut buf = String::new();
        HtmlRenderer::render_inner(&mut buf, view);
        buf
    }
    fn render_inner(buf: &mut String, mut view: Box<dyn View>) {
        match view.downcast::<Stack>() {
            Ok(stack) => {
                buf.push_str("<div>");

                for view in stack.content {
                    HtmlRenderer::render_inner(buf, view);
                }

                buf.push_str("</div>");
                return;
            }
            Err(v) => view = v,
        }

        match view.downcast::<Text>() {
            Ok(text) => {
                buf.push_str("<p>");

                buf.push_str(&text.text);
                buf.push_str("</p>");

                return;
            }
            Err(v) => view = v,
        }

        HtmlRenderer::render_inner(buf, view.view());
    }
}

#[test]
fn test() {
    let view = stack![Text::new("Hello!"), Text::new("Water UI!")];

    let string = HtmlRenderer::render(view);
    println!("{string}");
}
