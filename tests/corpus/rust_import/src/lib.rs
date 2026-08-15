pub mod alpha;
pub mod beta;
pub mod nested;

use crate::alpha::AlphaThing;

pub fn root_uses_alpha() -> AlphaThing {
    AlphaThing::default()
}
