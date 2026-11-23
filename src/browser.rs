use std::io;

/// Opens a URL with the operating system's configured browser.
pub fn launch(url: &str) -> io::Result<()> {
    open::that_detached(url)
}
