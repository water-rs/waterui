use waterui_task::task;

fn main() {
    task(hello());
}

pub async fn hello() {
    println!("Hello,world");
}
