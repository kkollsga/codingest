//! Standalone, libpython-free frontend over the shared `codingest` CLI library.

fn main() {
    if let Err(error) = codingest_cli::run(std::env::args_os()) {
        eprintln!("Error: {error:?}");
        // Not `anyhow::Result` from `main`: a `--require-fresh` refusal has to
        // reach CI as its own exit code, and `Termination` only ever gives 1.
        std::process::exit(codingest_cli::exit_code_for(&error));
    }
}
