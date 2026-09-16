//! What one application declares about its own controls.
//!
//! ```text
//! declared <pid>
//! ```
//!
//! For seeing what Archie sees on the accessibility bus, and for telling "the application
//! says nothing" apart from "Archie is asking the wrong window".

#[cfg(unix)]
#[tokio::main(flavor = "current_thread")]
async fn main() -> std::process::ExitCode {
    let Some(pid) = std::env::args().nth(1).and_then(|a| a.parse::<u32>().ok()) else {
        eprintln!("declared: give the process id of the application");
        return std::process::ExitCode::FAILURE;
    };
    let bus = match archie_a11y::Accessibility::connect().await {
        Ok(bus) => bus,
        Err(error) => {
            eprintln!("declared: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    // Every window of it: this is for looking, and a window size would narrow it down.
    match bus.controls_of(pid, None).await {
        Ok(controls) => {
            for control in &controls {
                println!(
                    "{:?} {:?} at {},{} {}x{}  ({})",
                    control.kind,
                    control.name,
                    control.bounds.x,
                    control.bounds.y,
                    control.bounds.width,
                    control.bounds.height,
                    control.path
                );
            }
            println!("{} controls", controls.len());
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("declared: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!("the accessibility bus is a Linux thing");
}
