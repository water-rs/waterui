// Simple test module to verify mdbook test functionality
// This demonstrates basic Rust patterns used in WaterUI without dependencies

#[cfg(test)]
mod tests {
    #[test]
    fn basic_function_test() {
        // Test basic function pattern
        fn welcome_message(name: &str) -> String {
            format!("Welcome, {}!", name)
        }
        
        let result = welcome_message("Alice");
        assert_eq!(result, "Welcome, Alice!");
    }

    #[test]
    fn struct_pattern_test() {
        // Test struct pattern similar to WaterUI components
        struct Counter {
            value: i32,
        }
        
        impl Counter {
            fn new(initial: i32) -> Self {
                Self { value: initial }
            }
            
            fn increment(&mut self) {
                self.value += 1;
            }
            
            fn get_value(&self) -> i32 {
                self.value
            }
        }
        
        let mut counter = Counter::new(0);
        counter.increment();
        assert_eq!(counter.get_value(), 1);
    }
}