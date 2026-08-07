mod dispatch;
mod hosts;
mod registry;
mod transport;

fn main() {
    if let Err(error) = dispatch::run() {
        eprintln!("proteus-reference-worker: {error:#}");
        std::process::exit(1);
    }
}
