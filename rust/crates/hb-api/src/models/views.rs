//! 對應 C# `Mcp/HayakuApi.Views.cs` 的 record（專案視圖、批次修改結果）。

use std::fmt;

/// 專案的視圖（list_views）。filter：認得的篩選條件（statuses、assignees…，值都轉成字串清單），
/// C# 是不分大小寫的 Dictionary，這裡用 [`ProjectView::filter_values`] 查；
/// has_other_conditions：還有本機不會套用的條件，要請伺服器用 view 篩。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectView {
    pub id: String,
    pub name: String,
    pub r#type: String,
    pub is_default: bool,
    /// (條件名稱, 值)；名稱不分大小寫不重複，照第一次出現的順序。
    pub filter: Vec<(String, Vec<String>)>,
    pub has_other_conditions: bool,
}

impl ProjectView {
    /// 某個篩選條件的值（名稱不分大小寫，同 C# `Filter.TryGetValue`）。
    pub fn filter_values(&self, name: &str) -> Option<&[String]> {
        self.filter.iter().find(|(k, _)| eq_ignore_case(k, name)).map(|(_, v)| v.as_slice())
    }

    /// `filter[name] = values`（不分大小寫；已經有就換掉值、保留原本的名稱）。
    pub fn set_filter(filter: &mut Vec<(String, Vec<String>)>, name: &str, values: Vec<String>) {
        match filter.iter_mut().find(|(k, _)| eq_ignore_case(k, name)) {
            Some((_, v)) => *v = values,
            None => filter.push((name.to_owned(), values)),
        }
    }
}

/// `StringComparer.OrdinalIgnoreCase`。
pub(crate) fn eq_ignore_case(a: &str, b: &str) -> bool {
    a.to_uppercase() == b.to_uppercase()
}

impl fmt::Display for ProjectView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

/// 批次修改的一筆結果；error 為 None 是成功（message 可能是「已送出變更申請」）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchResult {
    pub task: String,
    pub error: Option<String>,
    pub message: Option<String>,
}
