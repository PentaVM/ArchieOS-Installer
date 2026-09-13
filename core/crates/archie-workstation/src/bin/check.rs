use archie_workstation::check_component;

fn main() -> std::process::ExitCode {
    let component = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    let checks = check_component(&component);
    for check in &checks {
        println!("{:<10} {:<24} {}", check.status, check.name, check.detail);
    }
    if checks.iter().all(|check| check.status.is_healthy()) {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}
