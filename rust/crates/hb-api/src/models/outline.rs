//! 批次新增任務的一個節點（C# `Core/TaskOutline.cs` 的 `OutlineNode`）。
//!
//! C# 放在 Core，但 `HayakuApi.CreateTasksAsync` 要用；hb-api 不能依賴 hb-core，所以型別放在這裡，
//! hb-core 的 `TaskOutline` 直接用這個型別（`hb_api::OutlineNode`）。

/// 批次新增任務的一個節點：名稱與底下的子任務。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OutlineNode {
    pub title: String,
    pub children: Vec<OutlineNode>,
}

impl OutlineNode {
    pub fn new(title: impl Into<String>, children: Vec<OutlineNode>) -> Self {
        Self { title: title.into(), children }
    }

    /// 自己加上所有子孫的數量。
    pub fn count(&self) -> usize {
        1 + self.children.iter().map(OutlineNode::count).sum::<usize>()
    }
}
