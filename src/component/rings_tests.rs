#[cfg(test)]
mod tests {
    use crate::component::rings::{ConcentricRings, RingData};
    use waterui_graphics::color::Color;

    #[test]
    fn test_rings_component_creation() {
        let data = vec![
            RingData {
                value: 0.5,
                color: Color::srgb(255, 0, 0),
            },
            RingData {
                value: 0.8,
                color: Color::srgb(0, 255, 0),
            },
        ];
        let rings = ConcentricRings::new(data);
        assert_eq!(rings.data.len(), 2);
        assert_eq!(rings.step, 40.0);
    }

    #[test]
    fn test_rings_component_empty_data() {
        let rings = ConcentricRings::new(vec![]);
        assert_eq!(rings.data.len(), 0);
    }

    #[test]
    fn test_rings_component_step_modifier() {
        let rings = ConcentricRings::new(vec![]).step(20.0);
        assert_eq!(rings.step, 20.0);
    }

    #[test]
    fn test_rings_negative_values() {
        let data = vec![RingData {
            value: -0.1,
            color: Color::srgb(255, 0, 0),
        }];
        let rings = ConcentricRings::new(data);
        assert_eq!(rings.data[0].value, -0.1);
    }
}
