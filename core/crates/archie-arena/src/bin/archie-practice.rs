//! Generate and run Stage-verified drawing drills for Photopea.
//!
//! ```text
//! archie-practice --socket <stage.sock> --key-file <key> --policy <apps.toml>
//!                 --out <practice-dir> [--app photopea] [--count 3] [--seed 1]
//! ```

fn main() -> std::process::ExitCode {
    archie_arena::practice::main_entry()
}
