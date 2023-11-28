use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    mem::take,
};

use crate::{
    component::{self, stack::DisplayMode, Button, FrameView, Stack, Text},
    utils::{Background, Color},
    view::{Alignment, BoxView, Edge, Frame, RendererBuilder, Size},
    BoxEvent, View,
};

pub struct HtmlRenderer {
    state: HtmlRenderState,
    renderer: RendererBuilder<HtmlRenderState, HtmlRendererLocalState, HtmlRendererMessage>,
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

struct HtmlRendererLocalState {
    tag: Tag,
}

#[derive(Default)]
struct HtmlRendererMessage {
    attributes: Vec<(&'static str, String)>,
}

impl HtmlRendererMessage {
    pub fn attributes(attributes: impl Into<Vec<(&'static str, String)>>) -> Self {
        Self {
            attributes: attributes.into(),
        }
    }
}

impl Default for HtmlRendererLocalState {
    fn default() -> Self {
        Self { tag: Tag::new("") }
    }
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
    attributes: HashMap<&'static str, String>,

    content: String,
}

impl Tag {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            attributes: HashMap::new(),

            content: String::new(),
        }
    }

    pub fn set_attribute<'a>(&mut self, key: &'static str, value: impl Into<Cow<'a, str>>) {
        let attribute = self.attributes.entry(key).or_default();
        let value = value.into();
        match key {
            "class" => {
                attribute.push_str(value.as_ref());
                attribute.push(' ');
            }
            "style" => {
                attribute.push_str(value.as_ref());
                attribute.push(';');
            }
            _ => *attribute = value.into_owned(),
        }
    }

    pub fn set_name(&mut self, name: &'static str) {
        self.name = name;
    }

    pub fn set_content(&mut self, content: String) {
        self.content = content;
    }

    pub fn extend_head(&mut self, buf: &mut String) {
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
        self.set_attribute("class", class);
    }

    pub fn add_style<'a>(&mut self, style: impl Into<Cow<'a, str>>) {
        self.set_attribute("style", style);
    }

    pub fn extend_tail(&self, buf: &mut String) {
        buf.push_str("</");
        buf.push_str(self.name);
        buf.push('>');
    }
}

impl<'a> Extend<&'a mut Tag> for String {
    fn extend<T: IntoIterator<Item = &'a mut Tag>>(&mut self, iter: T) {
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

fn frame_to_style(frame: Frame) -> String {
    let mut result = String::new();
    if frame.height != Size::default() {
        result += &format!("height:{};", size_to_css(frame.height));
    }
    if frame.width != Size::default() {
        result += &format!("width:{};", size_to_css(frame.width));
    }
    if frame.margin != Edge::default() {
        result += &format!(
            "margin:{} {} {} {};",
            size_to_css(frame.margin.top),
            size_to_css(frame.margin.right),
            size_to_css(frame.margin.bottom),
            size_to_css(frame.margin.left)
        );
    }
    result
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
        let mut renderer: RendererBuilder<
            HtmlRenderState,
            HtmlRendererLocalState,
            HtmlRendererMessage,
        > = RendererBuilder::new().global_hook_before(|state, local_state, message| {
            for (key, value) in &message.attributes {
                local_state.tag.set_attribute(key, value);
            }
        });

        renderer.add(|view: Text, state, mut local_state, message, render| {
            let tag = &mut local_state.tag;
            tag.set_name("p");
            tag.set_content(view.text.into_html());
            match view.alignment {
                Alignment::Leading => tag.add_class("water-text-leading"),
                Alignment::Center => tag.add_class("water-text-center"),
                Alignment::Trailing => tag.add_class("water-text-trailing"),
                _ => {}
            }
            if !view.selectable {
                tag.add_class("disable-select");
            }
            state.buf.extend([tag]);
        });

        renderer.add(|view: Button, state, local_state, message, renderer| {
            let tag = &mut local_state.tag;
            tag.set_name("button");
            padding_builder(tag, view.padding);
            background_builder(tag, view.background);

            tag.set_content(view.label.into_html());
            state.buf.extend([tag]);
        });

        renderer.add(
            |view: FrameView, state, local_state, message, mut renderer| {
                renderer.call_with_message(
                    view.content,
                    state,
                    HtmlRendererMessage::attributes([("style", frame_to_style(view.frame))]),
                );
            },
        );

        renderer.add(|view: Stack, state, local_state, message, renderer| {
            let tag = &mut local_state.tag;
            tag.set_name("div");
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

        Self {
            renderer,
            state: HtmlRenderState::new(),
        }
    }

    pub fn renderer(mut self, view: BoxView) -> String {
        self.renderer.call(view, &mut self.state);
        self.state.buf
    }
}

enum DOMEvent {
    OnClick,
}
