//! `benilla-libfpv`: WoW FPV, an FPV quad flying through the 1.12 world on libfpv, built on top of
//! benilla. Bare, it starts the full client through `benilla_app::run_with`; `--worldview` starts
//! the offline world viewer through `benilla_world::worldview::run_with`, with no server.
//!
//! ```text
//! benilla-libfpv --worldview                                   the viewer, with this crate's plugin
//! benilla-libfpv --worldview --roll-test <site>                fly a site's rolling path, live
//! benilla-libfpv --worldview --roll-test <site> --capture <dir>
//!                                                              photograph it flat and rolled, exit
//! benilla-libfpv --worldview --flight-test <park|wall> [--capture <dir>]
//!                                                              a headless libfpv flight, PASS/FAIL
//! ```
//!
//! Roll sites: `streets` (Stormwind's Trade District), `cathedral` (inside the Cathedral of
//! Light), `open` (Westfall) and `over-city`. `roll_test.rs` and `flight_test.rs` say what each
//! does and writes.

use benilla_libfpv::{flight_test, roll_test};

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
        benilla_world::worldview::run_with(build, move |app| match cli.test {
            Some(Test::Roll(t)) => {
                app.add_plugins(roll_test::RollTestPlugin(t));
            }
            Some(Test::Flight(t)) => {
                app.add_plugins(flight_test::FlightTestPlugin(t));
            }
            None => {}
        })
    } else {
        // The full client; the FPV plugin joins it in M8.
        benilla_app::run_with(build, |_app| {})
    }
}

enum Test {
    Roll(roll_test::RollTest),
    Flight(flight_test::FlightTest),
}

/// The command line, parsed by hand: a handful of flags.
struct Cli {
    worldview: bool,
    test: Option<Test>,
}

impl Cli {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut worldview = false;
        let mut site = None;
        let mut scenario = None;
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
                "--flight-test" => {
                    let name = it.next().ok_or("--flight-test wants a scenario")?;
                    scenario = Some(
                        flight_test::Scenario::named(name)
                            .ok_or_else(|| format!("unknown scenario `{name}`"))?,
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
        let test = match (site, scenario) {
            (Some(_), Some(_)) => return Err("one test at a time".into()),
            (Some(site), None) => Some(Test::Roll(roll_test::RollTest { site, capture })),
            (None, Some(scenario)) => {
                Some(Test::Flight(flight_test::FlightTest { scenario, capture }))
            }
            (None, None) if capture.is_some() => {
                return Err("--capture wants a --roll-test or --flight-test".into())
            }
            (None, None) => None,
        };
        if test.is_some() && !worldview {
            return Err("the tests run in the --worldview only".into());
        }
        Ok(Self { worldview, test })
    }
}
