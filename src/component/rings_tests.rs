#[cfg(test)]
mod tests {
    use crate::component::rings::ConcentricRings;

    #[test]
    fn test_rings_component_creation() {
        let rings = ConcentricRings::new(vec![0.5, 0.8]);
        assert_eq!(rings.values.len(), 2);
    }
}
