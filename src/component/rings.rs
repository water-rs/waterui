use crate::component::progress::progress;
use crate::prelude::*;
use waterui_core::id::Identifiable;
use waterui_core::{Environment, View};
use waterui_graphics::color::Color;

#[derive(Debug, Clone)]
pub struct RingData {
    pub value: f64,
    pub color: Color,
}

#[derive(Debug)]
pub struct ConcentricRings {
    pub data: Vec<RingData>,
    pub step: f32,
}

impl ConcentricRings {
    pub fn new(data: Vec<RingData>) -> Self {
        Self { data, step: 40.0 }
    }

    #[must_use]
    pub fn step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }
}

#[derive(Clone)]
struct RingItem {
    idx: usize,
    data: RingData,
}

impl Identifiable for RingItem {
    type Id = usize;
    fn id(&self) -> Self::Id {
        self.idx
    }
}

impl View for ConcentricRings {
    fn body(self, _env: &Environment) -> impl View {
        let all_empty = !self.data.is_empty() && self.data.iter().all(|d| d.value <= 0.0);
        let step = self.step;

        let items: Vec<RingItem> = self
            .data
            .into_iter()
            .enumerate()
            .map(|(idx, data)| RingItem { idx, data })
            .collect();

        let rings = ZStack::for_each(items, move |item| {
            let ring_padding = (item.idx as f32) * step;
            let val = if item.data.value <= 0.0 {
                0.0
            } else {
                item.data.value
            };

            progress(val)
                .circular()
                .foreground(item.data.color)
                .padding_with(ring_padding)
                .anyview()
        });

        zstack((
            rings,
            if all_empty {
                text!("0%").anyview()
            } else {
                spacer().anyview()
            },
        ))
    }
}
