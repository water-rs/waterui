use crate::component::{Button, DatePicker, Image, Stack, Text};

pub enum Widget {
    Text(Text),
    Button(Button),
    Image(Image),
    Stack(Stack),
}
