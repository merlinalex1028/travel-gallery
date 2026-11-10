fn main() {
    if travel_gallery::quality_worker::run().is_err() {
        std::process::exit(1);
    }
}
