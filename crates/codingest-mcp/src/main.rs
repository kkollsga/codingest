//! Thin standalone process wrapper for [`codingest_mcp::run`].

fn main() -> anyhow::Result<()> {
    codingest_mcp::run(std::env::args_os())
}
