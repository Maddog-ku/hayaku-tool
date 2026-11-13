//! 對應 C# `Mcp/HayakuApi.Templates.cs` 的 record（子任務範本）。

/// 子任務範本（list_subtask_templates）：project 為 None 是全站範本；count 是範本裡的任務數（回應沒有就是 None）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtaskTemplate {
    pub id: String,
    pub name: String,
    pub project: Option<String>,
    pub count: Option<i32>,
    pub description: Option<String>,
}

impl SubtaskTemplate {
    pub fn label(&self) -> String {
        let count = self.count.map(|n| format!("（{n} 個任務）")).unwrap_or_default();
        let project = match &self.project {
            None => " · 全站".to_owned(),
            Some(p) => format!(" · {p}"),
        };
        format!("{}{count}{project}", self.name)
    }
}

/// 套用範本的結果（apply_subtask_template）：lines 是每個任務會建／建了什麼，message 是伺服器的說明，raw 是認不出格式時的原文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateApplyResult {
    pub lines: Vec<String>,
    pub message: Option<String>,
    pub raw: String,
}
