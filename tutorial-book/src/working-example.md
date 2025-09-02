# Working Example

This is a simple working example to demonstrate that mdbook test works:

```rust
fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    assert_eq!(add(2, 3), 5);
    println!("Test passed!");
}
```

This example shows basic Rust syntax and testing patterns that work with mdbook test.

## Simple Data Structures

```rust
#[derive(Debug, PartialEq)]
struct Point {
    x: f64,
    y: f64,
}

impl Point {
    fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }
    
    fn distance_from_origin(&self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
}

fn main() {
    let point = Point::new(3.0, 4.0);
    assert_eq!(point.distance_from_origin(), 5.0);
    println!("Point: {:?}", point);
}
```

These examples compile and run correctly, demonstrating the basic patterns used throughout WaterUI.