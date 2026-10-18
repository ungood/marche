mod steps;

use cucumber::World as _;
use cucumber::writer::Stats as _;
use futures::FutureExt as _;
use steps::{ows, services, world::MarcheWorld};

/// marche's own features, then the vendored Open Workflow conformance kit.
const SUITES: [&str; 2] = ["features", "vendor/ows/ctk/features"];

#[tokio::main]
async fn main() {
    let mut failed = false;
    for suite in SUITES {
        let summary = MarcheWorld::cucumber()
            .fail_on_skipped()
            // Stop a scenario's own server, worker and run as soon as it
            // ends, whether or not it passed.
            .after(|_, _, _, _, world| {
                if let Some(world) = world {
                    world.stop();
                }
                async {}.boxed_local()
            })
            .filter_run(suite, |_, _, scenario| {
                !scenario.tags.iter().any(|tag| tag == "wip") && ows::is_supported(scenario)
            })
            .await;
        failed |= summary.execution_has_failed();
    }
    services::stop();
    if failed {
        std::process::exit(1);
    }
}
