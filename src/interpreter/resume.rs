//! The state a run carries when it continues as new, and the frames that say
//! where it stopped.
//!
//! A run continues as new only between the tasks of a list, with nothing in
//! flight, and only when every task around that point is a `do` or a sequential
//! `for`: their position is all the state they have. The interpreter stops by
//! returning [`Halt::ContinueAsNew`](super::Halt::ContinueAsNew), and each layer
//! it unwinds through adds a frame to the front of the position. Resuming walks
//! the same path down, each layer taking its frame from the front.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What the next run needs to carry on. The definition, the run's input and
/// the invoking directory travel in the workflow input already.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// `$workflow.startedAt`, from the first run of the chain.
    pub started_at: Value,
    pub context: Value,
    /// Where the run stopped, from the workflow's own task list inwards.
    pub position: Vec<Frame>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "frame")]
pub enum Frame {
    /// A task list that stopped before the task at `index`, whose input is
    /// `input`. This is where the run carries on.
    Before { index: usize, input: Value },
    /// A task list in the middle of the task at `index`.
    Within { index: usize },
    /// That task: its raw input, its input after `input.from`, and
    /// `$task.startedAt`.
    #[serde(rename_all = "camelCase")]
    Task {
        input: Value,
        transformed: Value,
        started_at: Value,
    },
    /// A sequential `for`, in the iteration at `iteration` over `items`.
    For { items: Vec<Value>, iteration: usize },
}
