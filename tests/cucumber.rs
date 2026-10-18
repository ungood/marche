mod steps;

use cucumber::World as _;
use cucumber::writer::Stats as _;
use futures::FutureExt as _;
use steps::{ows, services, world::MarcheWorld};

/// marche's own features.
const FEATURES: &str = "features";
/// The vendored Open Workflow conformance kit, whose scenarios run once marche
/// supports every task type they use.
const KIT: &str = "vendor/ows/ctk/features";

#[tokio::main]
async fn main() {
    let mut failed = false;
    for suite in [FEATURES, KIT] {
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
            .filter_run(suite, move |_, _, scenario| {
                if scenario.tags.iter().any(|tag| tag == "wip") {
                    return false;
                }
                if suite == KIT {
                    return ows::is_supported(scenario);
                }
                // A scenario of marche's own always runs. One that the kit's
                // filter would skip means the filter misreads definitions.
                let unsupported = ows::unsupported(scenario);
                assert!(
                    unsupported.is_empty(),
                    "the scenario {:?} uses {unsupported:?}, which aren't in SUPPORTED \
                     in tests/steps/ows.rs",
                    scenario.name
                );
                true
            })
            .await;
        failed |= summary.execution_has_failed();
    }
    services::stop().await;
    if failed {
        std::process::exit(1);
    }
}
