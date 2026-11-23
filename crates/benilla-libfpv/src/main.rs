//! `benilla-libfpv`: WoW FPV, an FPV quad flying through the 1.12 world on libfpv, built on top of
//! benilla. Bare, it starts the full client through `benilla_app::run_with`, with FPV from the
//! character (`client.rs`: F7 launches, F goes back to the pilot); `--worldview` starts the
//! offline world viewer through `benilla_world::worldview::run_with`, with no server.
//!
//! ```text
//! benilla-libfpv --worldview                                   the viewer, with this crate's plugin
//! benilla-libfpv --worldview --roll-test <site>                fly a site's rolling path, live
//! benilla-libfpv --worldview --roll-test <site> --capture <dir>
//!                                                              photograph it flat and rolled, exit
//! benilla-libfpv --worldview --flight-test <park|wall|climb> [--capture <dir>]
//!                                                              a headless libfpv flight, PASS/FAIL
//! benilla-libfpv --worldview --fly [site]                      fly with the radio (default streets)
//! benilla-libfpv --worldview --scan x0,y0,x1,y1[,z]            survey the drone's collision in an area
//! benilla-libfpv --worldview --probe x,y,z,heading[,shot.png]  rays and a photo along a path
//! ```
//!
//! Sites: `streets` (Stormwind's Trade District), `cathedral` (inside the Cathedral of Light),
//! `open` (Westfall) and `over-city`. `roll_test.rs`, `flight_test.rs` and `fly.rs` say what each
//! does and writes.

use benilla_libfpv::{client, flight_test, fly, roll_test, scan};

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
            Some(Test::Fly(site)) => {
                app.add_plugins(fly::FlyPlugin(site));
            }
            Some(Test::Probe(p)) => {
                app.add_plugins(scan::ProbePlugin(p));
            }
            Some(Test::Scan(s)) => {
                app.add_plugins(scan::ScanPlugin(s));
            }
            None => {}
        })
    } else {
        // The full client, with FPV from the character (F7).
        benilla_app::run_with(build, |app| {
            app.add_plugins(client::ClientPlugin);
            if let Some(watch) = client::Watch::from_env() {
                app.insert_resource(watch)
                    .add_systems(bevy::prelude::Update, client::watch_players);
            }
        })
    }
}

enum Test {
    Roll(roll_test::RollTest),
    Flight(flight_test::FlightTest),
    Fly(roll_test::Site),
    Scan(scan::Scan),
    Probe(scan::Probe),
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
        let mut fly = None;
        let mut survey = None;
        let mut probe = None;
        let mut it = args.iter().peekable();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--worldview" => worldview = true,
                "--probe" => {
                    let spec = it.next().ok_or("--probe wants x,y,z,heading[,shot.png]")?;
                    probe = Some(scan::Probe::parse(spec).ok_or_else(|| {
                        format!("--probe wants x,y,z,heading[,shot.png], not `{spec}`")
                    })?);
                }
                "--scan" => {
                    let line = it.next().ok_or("--scan wants x0,y0,x1,y1")?;
                    survey = Some(
                        scan::Scan::parse(line)
                            .ok_or_else(|| format!("--scan wants x0,y0,x1,y1, not `{line}`"))?,
                    );
                }
                "--fly" => {
                    // An optional site name; a following flag is not one.
                    let name = it
                        .next_if(|n| !n.starts_with("--"))
                        .map_or("streets", String::as_str);
                    fly = Some(
                        roll_test::Site::named(name)
                            .ok_or_else(|| format!("unknown site `{name}`"))?,
                    );
                }
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
        let modes = [
            site.is_some(),
            scenario.is_some(),
            fly.is_some(),
            survey.is_some(),
            probe.is_some(),
        ];
        if modes.iter().filter(|&&m| m).count() > 1 {
            return Err(
                "one of --roll-test, --flight-test, --fly, --scan and --probe at a time".into(),
            );
        }
        if capture.is_some() && site.is_none() && scenario.is_none() {
            return Err("--capture wants a --roll-test or --flight-test".into());
        }
        let test = match (site, scenario, fly) {
            (Some(site), ..) => Some(Test::Roll(roll_test::RollTest { site, capture })),
            (_, Some(scenario), _) => {
                Some(Test::Flight(flight_test::FlightTest { scenario, capture }))
            }
            (.., Some(site)) => Some(Test::Fly(site)),
            _ => survey.map(Test::Scan).or(probe.map(Test::Probe)),
        };
        if test.is_some() && !worldview {
            return Err("the tests and --fly run in the --worldview only".into());
        }
        Ok(Self { worldview, test })
    }
}
