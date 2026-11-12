//! HayakuApi 用到的資料型別（C# 各 `HayakuApi*.cs` 檔頭的 record）。一個 C# partial 檔一個子模組。

mod core;
mod extras;
mod knowledge;
mod meetings;
mod outline;
mod projects;
mod stats;
mod task_extras;
mod templates;
mod testing;
mod views;
mod work;

pub use core::*;
pub use extras::*;
pub use knowledge::*;
pub use meetings::*;
pub use outline::*;
pub use projects::*;
pub use stats::*;
pub use task_extras::*;
pub use templates::*;
pub use testing::*;
pub use views::*;
pub use work::*;

/// C# `DateTimeOffset`。
pub type DateTimeOffset = chrono::DateTime<chrono::FixedOffset>;
