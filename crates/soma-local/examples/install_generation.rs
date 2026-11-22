//! Publishes a certified Generation already staged under the host's private prepared root.

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "install_generation/linux.rs"]
mod linux;

fn main() {
    if soma_version::version_requested(std::env::args_os()) {
        print!(
            "{}",
            soma_version::report(
                "install_generation",
                &soma_version::build!(),
                &soma_version::Speaks::default(),
            )
        );
        return;
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        let arguments: Vec<_> = std::env::args_os().skip(1).collect();
        let [root, stage, reference, generation] = arguments.as_slice() else {
            eprintln!(
                "usage: install_generation <prepared-root> <stage-name> <pinned-reference> <generation-id>"
            );
            std::process::exit(2);
        };
        match linux::install(root, stage, reference, generation) {
            Ok(path) => println!("{}", path.display()),
            Err(error) => {
                eprintln!("install_generation: {error}");
                std::process::exit(1);
            }
        }
    }

    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    {
        eprintln!("install_generation requires Linux x86_64");
        std::process::exit(2);
    }
}
