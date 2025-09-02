use waterui::prelude::*;
use nami::*;

pub fn counter_view() -> impl View {
    let count = signal(0);
    
    vstack((
        text!("Count: {}", count.get()),
        hstack((
            button("Increment").on_click(move || {
                count.set(count.get() + 1);
            }),
            button("Decrement").on_click(move || {
                count.set(count.get() - 1);
            }),
        )),
    ))
}

pub fn app() -> impl View {
    vstack((
        text!("Welcome to WaterUI!"),
        counter_view(),
    ))
    .padding(20)
}