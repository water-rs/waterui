#[derive(Debug, Clone)]
pub struct Blur {
    pub radius: f64,
}

impl Blur {
    pub fn new(radius: f64) -> Self {
        Self { radius }
    }
}
