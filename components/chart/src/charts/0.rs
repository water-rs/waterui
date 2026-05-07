
/// Convenience constructor for [`ScatterChart`].
///
/// Equivalent to [`ScatterChart::new`]; matches WaterUI's "general constructor +
/// ergonomic free function" convention.
#[must_use]
pub fn 0_chart<S: Signal<Output = Vec<DataPoint>>>(data: S) -> ScatterChart<S> {
    ScatterChart::new(data)
}
