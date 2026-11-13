//! Squeak daemon (`squeakd`) scaffold.

fn main() {
    println!("squeakd {} (scaffold)", env!("CARGO_PKG_VERSION"));
}

#[cfg(test)]
mod tests {
    #[test]
    fn daemon_version_present() {
        assert!(!env!("CARGO_PKG_VERSION").is_empty());
    }
}
