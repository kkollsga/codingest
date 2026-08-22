pub fn helper() -> u32 {
    3
}

/// One inline level. `super::` here names THIS file's module, so both uses
/// stay inside `alpha` and must form no cross-file edge at all — before the
/// fix they were popped against the file and landed on `lib.rs`.
pub mod inner {
    use super::helper;
    use crate::beta::beta_entry;

    pub fn call_helper() -> u32 {
        helper() + beta_entry()
    }
}

/// One inline level, TWO supers: one is cancelled by `inner_escape`, the
/// remainder still pops the file's real parent, so this resolves to
/// `beta.rs` — the positive half of the fix.
pub mod inner_escape {
    use super::super::beta::beta_entry;

    pub fn escape() -> u32 {
        beta_entry()
    }
}

/// Two inline levels, two supers: back at `alpha` itself.
pub mod outer {
    pub mod deeper {
        use super::super::helper;

        pub fn deep_call() -> u32 {
            helper()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_is_three() {
        assert_eq!(helper(), 3);
    }
}
