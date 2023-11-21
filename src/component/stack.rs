use crate::view::BoxView;

pub struct Stack {
    pub content: Vec<BoxView>,
}

#[macro_export]
macro_rules! stack {
    ($($view:expr),*) => {
        {
            let mut content=Vec::new();
            $(
                let view:Box<dyn crate::View>=Box::new($view);
                content.push(view);
            )*
            crate::component::Stack{content}
        }
    };
}

native_implement!(Stack);
