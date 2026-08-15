pub mod inner;

use super::alpha::AlphaThing;

pub fn nested_uses_super() -> AlphaThing {
    AlphaThing::default()
}
