pub struct SearchBar<C, F, V> {
    data: C,
    filter: F,
    content: V,
}

trait Query {
    fn query();
}
