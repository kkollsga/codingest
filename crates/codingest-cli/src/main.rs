//! Standalone, libpython-free frontend over the shared `codingest` CLI library.

fn main() -> anyhow::Result<()> {
    codingest_cli::run(std::env::args_os())
}
