//! 對應 C# `Core/TaskColumns.cs`：工作台「我的任務」清單可以設定的欄位（顯示、順序、寬度）。

use hb_backend::config::TaskColumnConfig;

pub struct TaskColumns;

impl TaskColumns {
    /// 所有欄位：Key、標題、預設寬度（名稱欄 0＝填滿）、預設是否顯示。計時按鈕固定在最前面，不算一欄。
    pub const ALL: [(&'static str, &'static str, f64, bool); 8] = [
        ("key", "編號", 76.0, true),
        ("title", "名稱", 0.0, true),
        ("project", "專案", 150.0, true),
        ("status", "狀態", 96.0, true),
        ("priority", "優先級", 60.0, false),
        ("start", "預計開始", 84.0, false),
        ("end", "預計結束", 96.0, true),
        ("elapsed", "今天計時", 76.0, false),
    ];

    pub const MIN_WIDTH: f64 = 40.0;
    pub const MAX_WIDTH: f64 = 480.0;

    pub fn label_of(key: &str) -> &str {
        Self::ALL.iter().find(|c| c.0 == key).map_or(key, |c| c.1)
    }

    /// 存的設定補成完整的欄位清單：照存的順序，存檔裡沒有的（新版加的欄位）照預設加在最後；
    /// 名稱欄一定顯示且寬度 0（填滿），其他欄寬度限制在合理範圍。
    pub fn normalize<'a>(saved: impl IntoIterator<Item = &'a TaskColumnConfig>) -> Vec<TaskColumnConfig> {
        let mut list: Vec<TaskColumnConfig> = Vec::new();
        for c in saved {
            if Self::ALL.iter().all(|a| a.0 != c.key) || list.iter().any(|x| x.key == c.key) {
                continue;
            }
            list.push(TaskColumnConfig { key: c.key.clone(), visible: c.visible, width: c.width });
        }
        for a in Self::ALL {
            if list.iter().all(|x| x.key != a.0) {
                list.push(TaskColumnConfig { key: a.0.to_owned(), visible: a.3, width: a.2 });
            }
        }
        for c in &mut list {
            if c.key == "title" {
                c.visible = true;
                c.width = 0.0;
            } else {
                let fallback = Self::ALL.iter().find(|a| a.0 == c.key).map_or(0.0, |a| a.2);
                c.width = if c.width <= 0.0 { fallback } else { c.width.clamp(Self::MIN_WIDTH, Self::MAX_WIDTH) };
            }
        }
        list
    }

    /// 把某欄往前（-1）或往後（+1）移一格。
    pub fn move_column(columns: &mut [TaskColumnConfig], key: &str, delta: i32) {
        let Some(i) = columns.iter().position(|c| c.key == key) else {
            return;
        };
        let j = i as i64 + delta as i64;
        if j < 0 || j >= columns.len() as i64 {
            return;
        }
        columns.swap(i, j as usize);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 對應 StatusFilterAndColumnsTests.欄位設定補齊新欄位_名稱欄一定顯示且填滿
    #[test]
    fn 欄位設定補齊新欄位_名稱欄一定顯示且填滿() {
        let col = |key: &str, visible: bool, width: f64| TaskColumnConfig { key: key.to_owned(), visible, width };
        let mut columns = TaskColumns::normalize(&[
            col("status", true, 9999.0),
            col("title", false, 100.0),
            col("unknown", true, 50.0),
            col("status", false, 10.0),
        ]);
        assert_eq!(TaskColumns::ALL.len(), columns.len());
        assert_eq!(vec!["status", "title"], columns.iter().take(2).map(|c| c.key.as_str()).collect::<Vec<_>>());
        assert_eq!(TaskColumns::MAX_WIDTH, columns[0].width);
        assert!(columns[1].visible);
        assert_eq!(0.0, columns[1].width);

        TaskColumns::move_column(&mut columns, "title", -1);
        assert_eq!("title", columns[0].key);
        TaskColumns::move_column(&mut columns, "title", -1);
        assert_eq!("title", columns[0].key);
    }

    #[test]
    fn label_of_找不到用key() {
        assert_eq!("狀態", TaskColumns::label_of("status"));
        assert_eq!("xyz", TaskColumns::label_of("xyz"));
    }
}
