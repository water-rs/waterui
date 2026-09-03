/// Greets every name once, keeping the order the caller gave.
pub fn greet(names: &[&str]) -> Vec<String> {
    let mut greetings = Vec::with_capacity(names.len());
    for (index, name) in names.iter().enumerate() {
        // Index from one: humans do not count from zero.
        greetings.push(format!("{}. Hello, {name}!", index + 1));
    }
    greetings
}
