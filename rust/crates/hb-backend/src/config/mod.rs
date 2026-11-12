//! 對應 C# `Config/`。

pub mod app_config;
pub mod app_state;
pub mod ci_map;

pub use app_config::{AppConfig, ProjectConfig, RepoConfig, TaskColumnConfig};
pub use app_state::{AppState, FolderHint, MeetingPause};
pub use ci_map::CiMap;
pub use indexmap::IndexMap;

#[cfg(test)]
mod cs_fixtures;
#[cfg(test)]
mod tests;
