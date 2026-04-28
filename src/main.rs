use scalable_cli::{human_error_message, run};

fn main() {
    if let Err(err) = run() {
        eprintln!("Error: {}", human_error_message(&err));
        std::process::exit(1);
    }
}
