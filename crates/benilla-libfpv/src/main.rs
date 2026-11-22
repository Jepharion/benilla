//! `benilla-libfpv`: WoW FPV, an FPV quad flying through the 1.12 world on libfpv, built on top of
//! benilla. Bare, it starts the full client through `benilla_app::run_with`; `--worldview` starts
//! the offline world viewer through `benilla_world::worldview::run_with`, with no server.
//!
//! ```text
//! benilla-libfpv --worldview                                   the viewer, with this crate's plugin
//! benilla-libfpv --worldview --roll-test <site>                fly a site's rolling path, live
//! benilla-libfpv --worldview --roll-test <site> --capture <dir>
//!                                                              photograph it flat and rolled, exit
//! ```
//!
//! Sites: `streets` (Stormwind's Trade District), `cathedral` (inside the Cathedral of Light) and
//! `open` (Elwynn, terrain and sky). `roll_test.rs` says what a capture writes.

use benilla_libfpv::roll_test;

use benilla_app::{AppExit, BuildId};

fn main() -> AppExit {
    let build = BuildId {
        version: env!("CARGO_PKG_VERSION"),
        describe: env!("BENILLA_GIT_DESCRIBE"),
        sha: env!("BENILLA_GIT_SHA"),
        short: env!("BENILLA_GIT_SHORT"),
        date: env!("BENILLA_GIT_DATE"),
        profile: env!("BENILLA_PROFILE"),
        project_dir: env!("BENILLA_PROJECT_DIR"),
        ..Default::default()
    };

    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = match Cli::parse(&args) {
        Ok(cli) => cli,
        Err(e) => {
            eprintln!("benilla-libfpv: {e}");
            return AppExit::error();
        }
    };

    if cli.worldview {
        benilla_world::worldview::run_with(build, move |app| {
            if let Some(test) = cli.roll_test {
                app.add_plugins(roll_test::RollTestPlugin(test));
            }
        })
    } else {
        // The full client; the FPV plugin joins it in M8.
        benilla_app::run_with(build, |_app| {})
    }
}

/// The command line, parsed by hand: a handful of flags.
struct Cli {
    worldview: bool,
    roll_test: Option<roll_test::RollTest>,
}

impl Cli {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut worldview = false;
        let mut site = None;
        let mut capture = None;
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--worldview" => worldview = true,
                "--roll-test" => {
                    let name = it.next().ok_or("--roll-test wants a site")?;
                    site = Some(
                        roll_test::Site::named(name)
                            .ok_or_else(|| format!("unknown site `{name}`"))?,
                    );
                }
                "--capture" => {
                    capture = Some(std::path::PathBuf::from(
                        it.next().ok_or("--capture wants a folder")?,
                    ));
                }
                other => return Err(format!("unknown argument `{other}`")),
            }
        }
        if site.is_some() && !worldview {
            return Err("--roll-test runs in the --worldview only".into());
        }
        if capture.is_some() && site.is_none() {
            return Err("--capture wants a --roll-test site".into());
        }
        Ok(Self {
            worldview,
            roll_test: site.map(|site| roll_test::RollTest { site, capture }),
        })
    }
}
