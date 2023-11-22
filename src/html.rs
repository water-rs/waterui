use std::collections::HashMap;

use crate::{
    component,
    reactive::{RawRef, Ref},
    view::{BoxView, Renderer},
    BoxEvent, View,
};

pub struct HtmlRenderer {
    state: HtmlRenderState,
    renderer: Renderer<HtmlRenderState>,
}

struct Manager {
    updater: HashMap<usize, Updater>,
    event: HashMap<usize, (DOMEvent, BoxEvent)>,
}

impl Manager {
    pub fn new() -> Self {
        Self {
            updater: HashMap::new(),
            event: HashMap::new(),
        }
    }
}

impl Manager {
    pub fn on_click(&mut self, id: usize, event: BoxEvent) {
        self.event.insert(id, (DOMEvent::OnClick, event));
    }
}

enum Updater {
    Content(Ref<String>),
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

impl HtmlRenderer {
    pub fn new() -> Self {
        let mut renderer: Renderer<HtmlRenderState> = Renderer::new();
        let state = HtmlRenderState::new();
        renderer.add(|state, renderer, view: component::Text| {
            state.buf.push_str("<p id=\"wui-");
            let id = state.get_id();
            state.buf.push_str(id.to_string().as_str());
            state.buf.push_str("\">");

            state.buf.push_str(view.text.get().as_ref());
            view.text.watch(|r: &RawRef<_>| {});

            state.buf.push_str("</p>");
        });

        renderer.add(|state, renderer, view: component::Button| {
            state.buf.push_str("<button id=\"wui-");
            let id = state.get_id();
            state.buf.push_str(id.to_string().as_str());
            state.buf.push_str("\">");
            state.buf.push_str(view.label.get().as_ref());
            state.buf.push_str("</button>");
        });

        renderer.add(|state, renderer, view: component::Stack| {
            state.buf.push_str("<div id=\"wui-");
            let id = state.get_id();
            state.buf.push_str(id.to_string().as_str());
            state.buf.push_str("\">");
            for view in view.content {
                renderer.call(view, state);
            }
            state.buf.push_str("</div>");
        });

        renderer.add(|state, renderer, view: component::TapGesture| {
            state.buf.push_str("<div id=\"wui-");
            let id = state.get_id();
            state.buf.push_str(id.to_string().as_str());
            state.buf.push_str("\">");
            state.manager.on_click(id, view.event);
            renderer.call(view.view, state);
            state.buf.push_str("</div>");
        });

        renderer
            .add(|state, renderer, view: component::ReactiveView| renderer.call(view.view, state));

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

mod test {
    use super::HtmlRenderer;
    use crate::{
        component::{text, Text},
        view::ViewExt,
        vstack,
    };

    #[test]
    fn test() {
        let renderer = HtmlRenderer::new();
        let view = vstack![text("233"), text("233"), text("233")];
        let view = view.on_tap(|| {});
        let html = renderer.renderer(Box::new(view));

        println!("{html}");
    }
}
