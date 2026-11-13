//! Squeak command-line interface (`squeak`) scaffold.

fn main() {
    println!("squeak {} (scaffold)", env!("CARGO_PKG_VERSION"));
}

#[cfg(test)]
mod tests {
    #[test]
    fn cli_version_present() {
        assert!(!env!("CARGO_PKG_VERSION").is_empty());
    }
}
