use std::env;

const BASE_URL_ENV: &str = "MULTIMETER_BASE_URL";

fn main() {
    println!("cargo:rerun-if-env-changed={BASE_URL_ENV}");

    let base_url = env::var(BASE_URL_ENV).unwrap_or_else(|_| {
        panic!(
            "{BASE_URL_ENV} must be configured for multimeter builds, for example in .cargo/config.toml"
        )
    });

    if base_url.is_empty() {
        panic!("{BASE_URL_ENV} must not be empty");
    }
    if base_url.chars().any(char::is_whitespace) {
        panic!("{BASE_URL_ENV} must not contain whitespace");
    }
    if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
        panic!("{BASE_URL_ENV} must start with http:// or https://");
    }

    println!("cargo:rustc-env={BASE_URL_ENV}={base_url}");
}
