//! Finding runs for `marche status`, `list`, `show` and `cancel`: a run named
//! by its workflow ID or its ULID, runs listed through Temporal's visibility,
//! and what those commands say about each.
//!
//! A run's ULID holds the time `marche run` started it, so a ULID is resolved
//! by listing the runs that started since then and keeping those whose ID ends
//! with it. Visibility's SQL backends can't match a suffix or sort, so marche
//! filters and sorts what it lists.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use console::Style;
use futures::StreamExt;
use jiff::Timestamp;
use serde::{Serialize, Serializer};
use temporalio_client::errors::{ClientError, WorkflowInteractionError};
use temporalio_client::tonic::Request;
use temporalio_client::{
    Client, WorkflowDescribeOptions, WorkflowExecution, WorkflowExecutionStatus, WorkflowHandle,
    WorkflowListOptions,
};
use temporalio_common::protos::temporal::api::enums::v1::{TaskQueueKind, TaskQueueType};
use temporalio_common::protos::temporal::api::taskqueue::v1::TaskQueue;
use temporalio_common::protos::temporal::api::workflowservice::v1::DescribeTaskQueueRequest;
use ulid::Ulid;

use crate::error::Error;
use crate::temporal::{self, DEFINITION, DIRECTORY, Temporal};
use crate::workflow::Interpret;

const WORKFLOW_TYPE: &str = "marche.interpret";
/// How long before its ULID's time a run may have started, for a server whose
/// clock disagrees with the machine that started the run.
const CLOCK_SKEW: Duration = Duration::from_secs(60 * 60);
/// How recently a worker must have polled to count as serving a task queue.
const LIVE_POLL: Duration = Duration::from_secs(2 * 60);

/// A run, as `marche status` and `marche list` describe it.
#[derive(Debug, Clone, Serialize)]
pub struct Run {
    pub run: String,
    /// `<namespace>.<name>:<version>`, from the run's search attributes.
    pub definition: Option<String>,
    pub status: &'static str,
    #[serde(serialize_with = "rfc3339")]
    pub started: Option<Timestamp>,
    pub directory: Option<String>,
    #[serde(skip)]
    pub task_queue: String,
}

fn rfc3339<S: Serializer>(time: &Option<Timestamp>, serializer: S) -> Result<S::Ok, S::Error> {
    match time {
        Some(time) => serializer.serialize_str(&time.to_string()),
        None => serializer.serialize_none(),
    }
}

/// A run's status in OWS terms: running, completed, faulted or cancelled.
pub fn status_name(status: WorkflowExecutionStatus) -> &'static str {
    match status {
        WorkflowExecutionStatus::Completed => "completed",
        WorkflowExecutionStatus::Failed | WorkflowExecutionStatus::TimedOut => "faulted",
        WorkflowExecutionStatus::Canceled | WorkflowExecutionStatus::Terminated => "cancelled",
        _ => "running",
    }
}

/// The style a status is printed in.
pub fn tone(status: &str) -> Style {
    match status {
        "completed" => Style::new().green(),
        "faulted" => Style::new().red(),
        "cancelled" => Style::new().yellow(),
        _ => Style::new().cyan(),
    }
}

/// A time as the local clock shows it, to the second.
pub fn local_time(time: Timestamp) -> String {
    time.to_zoned(jiff::tz::TimeZone::system())
        .strftime("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// The directory marche is run in, as `marche run` records it.
pub fn current_dir() -> Result<String, Error> {
    let dir = std::env::current_dir().map_err(|error| Error::Read {
        path: ".".into(),
        error,
    })?;
    Ok(dir.to_string_lossy().into_owned())
}

/// The ULID at the end of a workflow ID.
fn ulid_of(id: &str) -> Option<Ulid> {
    let (_, suffix) = id.rsplit_once('-')?;
    Ulid::from_string(suffix).ok()
}

/// When a run started: its ULID's time, which stays the same when the run
/// continues as new, or else Temporal's start time.
fn started(id: &str, start_time: Option<SystemTime>) -> Option<Timestamp> {
    match ulid_of(id) {
        Some(ulid) => Timestamp::from_millisecond(ulid.timestamp_ms() as i64).ok(),
        None => start_time.and_then(|time| Timestamp::try_from(time).ok()),
    }
}

/// A string literal for a visibility query.
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

pub fn handle(temporal: &Temporal, id: &str) -> WorkflowHandle<Client, Interpret> {
    temporal
        .client
        .get_workflow_handle::<Interpret>(id.to_string())
}

/// The workflow ID of the run that `run` names: the ID itself, or a ULID.
pub async fn resolve(temporal: &Temporal, run: &str) -> Result<String, Error> {
    let ulid = match Ulid::from_string(run) {
        Ok(ulid) if !run.contains(['-', '.']) => ulid,
        _ => return Ok(run.to_string()),
    };
    let since = ulid.datetime() - CLOCK_SKEW;
    let since = Timestamp::try_from(since).unwrap_or(Timestamp::UNIX_EPOCH);
    let query = format!(
        "WorkflowType = {} AND StartTime >= {}",
        quote(WORKFLOW_TYPE),
        quote(&since.to_string())
    );
    let suffix = format!("-{ulid}");
    let mut ids: Vec<String> = executions(temporal, &query)
        .await?
        .iter()
        .map(|execution| execution.id().to_string())
        .filter(|id| id.ends_with(&suffix))
        .collect();
    ids.sort();
    ids.dedup();
    match ids.len() {
        0 => Err(Error::NoUlid(ulid.to_string())),
        1 => Ok(ids.remove(0)),
        _ => {
            let (last, rest) = ids.split_last().expect("more than one run");
            Err(Error::Ambiguous {
                ulid: ulid.to_string(),
                runs: format!("{} and {last}", rest.join(", ")),
            })
        }
    }
}

/// The run with this workflow ID, as Temporal describes it now.
pub async fn describe(temporal: &Temporal, id: &str) -> Result<Run, Error> {
    let description = handle(temporal, id)
        .describe(WorkflowDescribeOptions::default())
        .await
        .map_err(|error| match error {
            WorkflowInteractionError::NotFound(status) | WorkflowInteractionError::Rpc(status)
                if temporal::is_missing_namespace(&status) =>
            {
                Error::NoNamespace(temporal.namespace.clone())
            }
            WorkflowInteractionError::NotFound(_) => Error::NoRun(id.to_string()),
            other => Error::Lookup {
                run: id.to_string(),
                message: other.to_string(),
            },
        })?;
    let attributes = description.search_attributes();
    Ok(Run {
        run: id.to_string(),
        definition: attributes.get(&DEFINITION),
        status: status_name(description.status()),
        started: started(id, description.start_time()),
        directory: attributes.get(&DIRECTORY),
        task_queue: description.task_queue().to_string(),
    })
}

/// The runs started from `directory`, or from anywhere, newest first.
pub async fn list(temporal: &Temporal, directory: Option<&str>) -> Result<Vec<Run>, Error> {
    let mut query = format!("WorkflowType = {}", quote(WORKFLOW_TYPE));
    if let Some(directory) = directory {
        query.push_str(&format!(" AND {} = {}", DIRECTORY.name(), quote(directory)));
    }
    // A run that continued as new is listed once for each Temporal run.
    let mut chains: HashMap<String, Vec<WorkflowExecution>> = HashMap::new();
    for execution in executions(temporal, &query).await? {
        chains
            .entry(execution.id().to_string())
            .or_default()
            .push(execution);
    }
    let mut runs: Vec<Run> = chains
        .into_iter()
        .map(|(id, chain)| {
            let first = chain.iter().filter_map(WorkflowExecution::start_time).min();
            let latest = chain
                .iter()
                .max_by_key(|execution| execution.start_time())
                .expect("a chain has a run");
            let attributes = latest.search_attributes();
            Run {
                definition: attributes.get(&DEFINITION),
                status: status_name(latest.status()),
                started: started(&id, first),
                directory: attributes.get(&DIRECTORY),
                task_queue: latest.task_queue().to_string(),
                run: id,
            }
        })
        .collect();
    runs.sort_by(|a, b| b.started.cmp(&a.started).then_with(|| b.run.cmp(&a.run)));
    Ok(runs)
}

async fn executions(temporal: &Temporal, query: &str) -> Result<Vec<WorkflowExecution>, Error> {
    let mut stream = temporal
        .client
        .list_workflows(query, WorkflowListOptions::default());
    let mut executions = Vec::new();
    while let Some(execution) = stream.next().await {
        executions.push(execution.map_err(|error| {
            if let ClientError::Rpc(status) = &error
                && temporal::is_missing_namespace(status)
            {
                return Error::NoNamespace(temporal.namespace.clone());
            }
            let message = error.to_string();
            match [DEFINITION.name(), DIRECTORY.name()]
                .into_iter()
                .find(|name| message.contains(name))
            {
                Some(name) => Error::SearchAttributes(name.to_string()),
                None => Error::List(message),
            }
        })?);
    }
    Ok(executions)
}

/// Fails unless a worker is serving the run's task queue, since only a worker
/// can answer a query for the run's tasks. Without one, a query would wait
/// for its deadline.
pub async fn require_worker(temporal: &Temporal, run: &Run) -> Result<(), Error> {
    match worker_serves(temporal, &run.task_queue).await {
        Some(false) => Err(Error::NoWorker {
            queue: run.task_queue.clone(),
            run: run.run.clone(),
        }),
        // If the server can't say, the query will.
        _ => Ok(()),
    }
}

/// Whether a worker is serving the task queue now, or `None` when the server
/// can't say. Temporal lists a worker that has stopped for minutes after its
/// last poll, so only workers that have polled within `LIVE_POLL` count. An
/// idle worker's long poll lasts a minute, so that's its usual age.
pub async fn worker_serves(temporal: &Temporal, queue: &str) -> Option<bool> {
    let mut service = temporal.client.connection().workflow_service();
    let response = service
        .describe_task_queue(Request::new(DescribeTaskQueueRequest {
            namespace: temporal.namespace.clone(),
            task_queue: Some(TaskQueue {
                name: queue.to_string(),
                kind: TaskQueueKind::Normal as i32,
                ..Default::default()
            }),
            task_queue_type: TaskQueueType::Workflow as i32,
            ..Default::default()
        }))
        .await
        .ok()?;
    let now = SystemTime::now();
    let polled = response.get_ref().pollers.iter().map(|poller| {
        poller
            .last_access_time
            .as_ref()
            .map(|time| (time.seconds, time.nanos))
    });
    Some(any_live(polled, now))
}

/// Whether any of these last poll times, as seconds and nanoseconds since
/// the Unix epoch, is recent enough to count as a live worker.
fn any_live(polled: impl IntoIterator<Item = Option<(i64, i32)>>, now: SystemTime) -> bool {
    polled.into_iter().flatten().any(|(seconds, nanos)| {
        let Ok(seconds) = u64::try_from(seconds) else {
            return false;
        };
        let time = SystemTime::UNIX_EPOCH
            + Duration::from_secs(seconds)
            + Duration::from_nanos(u64::try_from(nanos).unwrap_or(0));
        // A poll time ahead of this machine's clock counts as now.
        now.duration_since(time).unwrap_or_default() <= LIVE_POLL
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_recent_pollers_are_live() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let ago = |seconds: i64| Some((1_000_000 - seconds, 0));
        assert!(any_live([ago(65)], now));
        assert!(any_live([ago(600), ago(5)], now));
        assert!(!any_live([ago(600)], now));
        assert!(!any_live([None], now));
        assert!(!any_live([], now));
        // A server clock a little ahead of this machine's.
        assert!(any_live([ago(-3)], now));
    }
}
