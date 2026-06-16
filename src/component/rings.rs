use crate::component::progress::progress;
use crate::prelude::*;
use waterui_core::id::Identifiable;
use waterui_core::{Environment, View};

pub struct ConcentricRings {
    pub values: Vec<f64>,
}

impl ConcentricRings {
    pub fn new(values: Vec<f64>) -> Self {
        Self { values }
    }
}

#[derive(Clone)]
struct RingItem {
    idx: usize,
    value: f64,
}

impl Identifiable for RingItem {
    type Id = usize;
    fn id(&self) -> Self::Id {
        self.idx
    }
}

impl View for ConcentricRings {
    fn body(self, _env: &Environment) -> impl View {
        let items: Vec<RingItem> = self
            .values
            .into_iter()
            .enumerate()
            .map(|(idx, value)| RingItem { idx, value })
            .collect();

        ZStack::for_each(items, |item| {
            // Scale radii for concentric rings
            // Apply increasing padding for each outer ring, or decreasing for inner
            let ring_padding = (item.idx as f32) * 40.0; // 40pt step between rings

            if item.value <= 0.0 {
                // 100% exhausted quota: countdown / completed text
                text!("0%").padding_with(ring_padding).anyview()
            } else {
                progress(item.value)
                    .circular()
                    .padding_with(ring_padding)
                    .anyview()
            }
        })
    }
}
