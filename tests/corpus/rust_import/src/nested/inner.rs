use self::private_helper as aliased_self;
use super::super::beta::beta_uses_renamed;

fn private_helper() -> u32 {
    1
}

pub fn deep_fn() -> u32 {
    let _ = beta_uses_renamed();
    aliased_self()
}
