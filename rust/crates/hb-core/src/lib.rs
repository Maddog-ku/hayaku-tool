//! TimerEngine 與各種任務邏輯（對應 C# `Core/`；WorkMath、KeyFinder 在 hb-api）。

pub mod ci_set;
pub mod error;
pub mod file_reader;
pub mod my_task_tabs;
pub mod my_task_tree;
pub mod off_day_cache;
pub mod project_guess;
pub mod sync_policy;
pub mod task_columns;
pub mod task_order;
pub mod task_outline;
pub mod task_schedule;
pub mod task_tree;
pub mod time;
pub mod timer_engine;
pub mod view_filter;

#[cfg(test)]
pub(crate) mod test_support;

pub use ci_set::CiSet;
pub use error::CoreError;
pub use file_reader::FileReader;
pub use my_task_tabs::MyTaskTabs;
pub use my_task_tree::{MyTaskNode, MyTaskProject, MyTaskTree};
pub use off_day_cache::OffDayCache;
pub use project_guess::ProjectGuess;
pub use sync_policy::SyncPolicy;
pub use task_columns::TaskColumns;
pub use task_order::{StatusFilter, TaskOrder};
pub use task_outline::{OutlineNode, TaskOutline};
pub use task_schedule::{Drag, Segment, TaskSchedule};
pub use task_tree::{TaskTree, TaskTreeNode};
pub use timer_engine::*;
pub use view_filter::ViewFilter;
