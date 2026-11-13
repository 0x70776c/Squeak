//! Shared domain models and data-driven device definitions for Squeak.

/// Returns the crate name for smoke-test verification.
#[must_use]
pub const fn crate_name() -> &'static str {
    "squeak-core"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke() {
        assert_eq!(crate_name(), "squeak-core");
    }
}
