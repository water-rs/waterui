//! Fragment list trait for zero-allocation shader fragment composition.
//!
//! Filters provide shader fragments as associated types implementing `FragmentList`.
//! This enables compile-time fusion of consecutive filter shaders without heap allocation.

/// Trait for shader fragment collections that can write to a shader string.
///
/// Implemented for `&'static str` and nested tuples `(A, B)`.
/// This design enables zero-allocation shader composition at compile time.
///
/// # Example
///
/// ```ignore
/// // Single filter fragment
/// let fragment: &'static str = "color = color * brightness;";
/// fragment.write_to(&mut shader);
///
/// // Chained filter fragments
/// let fragments: (&str, &str) = ("color = grayscale(color);", "color = 1.0 - color;");
/// fragments.write_to(&mut shader);
/// ```
pub trait FragmentList: Send {
    /// Write the shader fragment(s) to the provided shader string.
    fn write_to(&self, shader: &mut String);
}

impl FragmentList for &'static str {
    #[inline]
    fn write_to(&self, shader: &mut String) {
        shader.push_str(self);
        shader.push('\n');
    }
}

// Implementation for nested tuples (enables chaining without allocation)

impl<A: FragmentList, B: FragmentList> FragmentList for (A, B) {
    #[inline]
    fn write_to(&self, shader: &mut String) {
        self.0.write_to(shader);
        self.1.write_to(shader);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_fragment() {
        let fragment: &'static str = "color = adjust(color);";
        let mut shader = String::new();
        fragment.write_to(&mut shader);
        assert_eq!(shader, "color = adjust(color);\n");
    }

    #[test]
    fn test_tuple_fragments() {
        let fragments: (&str, &str) = ("// first", "// second");
        let mut shader = String::new();
        fragments.write_to(&mut shader);
        assert_eq!(shader, "// first\n// second\n");
    }

    #[test]
    fn test_nested_fragments() {
        // Simulates: A.then(B).then(C)
        let fragments: ((&str, &str), &str) = (("// A", "// B"), "// C");
        let mut shader = String::new();
        fragments.write_to(&mut shader);
        assert_eq!(shader, "// A\n// B\n// C\n");
    }
}
