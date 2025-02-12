use waterui_reactive::collection::Collection;

pub struct SearchBar<C, F, V> {
    data: C,
    filter: F,
    content: V,
}

trait Query {
    fn query();
}
