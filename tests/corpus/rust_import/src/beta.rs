use crate::alpha::AlphaThing as Renamed;
use crate::{alpha::alpha_helper, nested::inner::deep_fn};

pub fn beta_uses_renamed() -> Renamed {
    let _ = alpha_helper();
    let _ = deep_fn();
    Renamed::default()
}
