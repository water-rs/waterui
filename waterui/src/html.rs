use std::{collections::HashMap, mem::take};

use crate::{
    component::{self, stack::DisplayMode},
    utils::{Background, Color},
    view::{Alignment, BoxView, Edge, Frame, Renderer, Size},
    BoxEvent, View,
};

pub struct HtmlRenderer {
    state: HtmlRenderState,
    renderer: Renderer<HtmlRenderState>,
}

struct Manager {
    event: HashMap<usize, (DOMEvent, BoxEvent)>,
}

impl Manager {
    pub fn new() -> Self {
        Self {
            event: HashMap::new(),
        }
    }
}

impl Manager {
    pub fn on_click(&mut self, id: usize, event: BoxEvent) {
        self.event.insert(id, (DOMEvent::OnClick, event));
    }
}

struct HtmlRenderState {
    id: usize,
    buf: String,
    manager: Manager,
}

impl HtmlRenderState {
    pub fn new() -> Self {
        Self {
            id: 0,
            buf: String::new(),
            manager: Manager::new(),
        }
    }

    pub fn get_id(&mut self) -> usize {
        self.id += 1;
        self.id
    }
}

struct Tag {
    name: &'static str,
    attributes: Vec<(&'static str, String)>,
    classes: Vec<&'static str>,
    style: String,
    content: String,
}

impl Tag {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            attributes: Vec::new(),
            classes: Vec::new(),
            style: String::new(),
            content: String::new(),
        }
    }

    pub fn set_attribute(&mut self, key: &'static str, value: impl Into<String>) {
        self.attributes.push((key, value.into()));
    }

    pub fn set_content(&mut self, content: String) {
        self.content = content;
    }

    pub fn extend_head(&mut self, buf: &mut String) {
        if !self.classes.is_empty() {
            self.set_attribute("class", self.classes.join(" "));
        }

        let style = take(&mut self.style);

        if !style.is_empty() {
            self.set_attribute("style", style);
        }
        buf.push('<');
        buf.push_str(self.name);
        for (key, value) in self.attributes.iter() {
            buf.push(' ');
            buf.push_str(key);
            buf.push_str("=\"");
            buf.push_str(value.as_str());
            buf.push_str("\"");
        }
        buf.push('>');
    }

    pub fn add_class(&mut self, class: &'static str) {
        self.classes.push(class);
    }

    pub fn add_style(&mut self, style: &str) {
        self.style.push_str(style);
    }

    pub fn extend_tail(&self, buf: &mut String) {
        buf.push_str("</");
        buf.push_str(self.name);
        buf.push('>');
    }
}

impl Extend<Tag> for String {
    fn extend<T: IntoIterator<Item = Tag>>(&mut self, iter: T) {
        for mut tag in iter {
            tag.extend_head(self);
            self.push_str(tag.content.as_str());
            tag.extend_tail(self);
        }
    }
}

fn size_to_css(size: Size) -> String {
    match size {
        Size::Default => "inherit".into(),
        Size::Px(px) => format!("{px}px"),
        Size::Percent(_) => todo!(),
        Size::Maximum(_) => todo!(),
        Size::Minimum(_) => todo!(),
    }
}

fn frame_builder(tag: &mut Tag, frame: Frame) {
    if frame.height != Size::default() {
        tag.add_style(&format!("height:{};", size_to_css(frame.height)));
    }
    if frame.width != Size::default() {
        tag.add_style(&format!("width:{};", size_to_css(frame.width)));
    }
    if frame.margin != Edge::default() {
        tag.add_style(&format!(
            "margin:{} {} {} {};",
            size_to_css(frame.margin.top),
            size_to_css(frame.margin.right),
            size_to_css(frame.margin.bottom),
            size_to_css(frame.margin.left)
        ));
    }
}

fn padding_builder(tag: &mut Tag, edge: Edge) {
    if edge != Edge::default() {
        tag.add_style(&format!(
            "padding:{} {} {} {};",
            size_to_css(edge.top),
            size_to_css(edge.right),
            size_to_css(edge.bottom),
            size_to_css(edge.left)
        ));
    }
}

fn background_builder(tag: &mut Tag, background: Background) {
    match background {
        Background::Default => {}
        Background::Image(url) => todo!(),
        Background::Color(color) => tag.add_style(&format!(
            "background:rgba({},{},{},{})",
            color.red, color.blue, color.green, color.opacity
        )),
    }
}

impl HtmlRenderer {
    pub fn new() -> Self {
        let mut renderer: Renderer<HtmlRenderState> = Renderer::new();
        let state = HtmlRenderState::new();
        renderer.add(|state, _renderer, view: component::Text| {
            let mut tag = Tag::new("p");
            frame_builder(&mut tag, view.frame());
            tag.set_content(view.text.into_html());
            match view.alignment {
                Alignment::Leading => tag.add_class("water-text-leading"),
                Alignment::Center => tag.add_class("water-text-center"),
                Alignment::Trailing => tag.add_class("water-text-trailing"),
                _ => {}
            }
            state.buf.extend([tag]);
        });

        renderer.add(|state, _renderer, view: component::Button| {
            let mut tag = Tag::new("button");
            frame_builder(&mut tag, view.frame());
            padding_builder(&mut tag, view.padding);
            background_builder(&mut tag, view.background);

            tag.set_content(view.label.into_html());
            state.buf.extend([tag]);
        });

        renderer.add(|state, renderer, view: component::Stack| {
            let mut tag = Tag::new("div");
            frame_builder(&mut tag, view.frame());

            match view.mode {
                DisplayMode::Vertical => {}
                DisplayMode::Horizontal => tag.add_class("water-horizontal"),
            }

            match view.alignment {
                Alignment::Leading => tag.add_class("water-leading"),
                Alignment::Center => tag.add_class("water-center"),
                Alignment::Trailing => tag.add_class("water-trailing"),
                _ => {}
            }

            tag.extend_head(&mut state.buf);

            for view in view.content {
                renderer.call(view, state);
            }
            tag.extend_tail(&mut state.buf);
        });

        renderer.add(|state, renderer, view: component::TapGesture| {
            state.buf.push_str("<div id=\"water-");
            let id = state.get_id();
            //tag.set_attribute("id", format!("wui-{id}"));

            state.buf.push_str(id.to_string().as_str());
            state.buf.push_str("\">");
            state.manager.on_click(id, view.event);
            renderer.call(view.view, state);
            state.buf.push_str("</div>");
        });

        Self { renderer, state }
    }

    pub fn renderer(mut self, view: BoxView) -> String {
        self.renderer.call(view, &mut self.state);
        self.state.buf
    }
}

enum DOMEvent {
    OnClick,
}
